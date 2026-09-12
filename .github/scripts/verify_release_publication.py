#!/usr/bin/env python3
"""Validate that every release surface points at the intended immutable build."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

REQUIRED_PLATFORMS = frozenset(("linux/amd64", "linux/arm64"))
REVISION_LABEL = "org.opencontainers.image.revision"
VERSION_LABEL = "org.opencontainers.image.version"


class PublicationError(ValueError):
    """Raised when a published release surface is incomplete or mismatched."""


def _require(condition: bool, message: str, errors: list[str]) -> None:
    if not condition:
        errors.append(message)


def verify_release_metadata(
    payload: dict[str, Any],
    *,
    release_tag: str,
    target_sha: str,
    release_prerelease: bool,
) -> list[str]:
    errors: list[str] = []
    _require(payload.get("tag_name") == release_tag, "GitHub Release tag_name does not match release_tag", errors)
    _require(payload.get("draft") is False, "GitHub Release must be published and not a draft", errors)
    _require(
        payload.get("prerelease") is release_prerelease,
        "GitHub Release prerelease state does not match release snapshot",
        errors,
    )
    _require(bool(payload.get("html_url")), "GitHub Release is missing html_url", errors)
    _require(bool(target_sha) and len(target_sha) == 40, "target_sha must be a full commit SHA", errors)
    return errors


def verify_tag_target(actual_sha: str, *, target_sha: str) -> list[str]:
    if actual_sha != target_sha:
        return ["Git tag does not resolve to target_sha"]
    return []


def verify_manifest(manifest: dict[str, Any], *, reference: str) -> list[str]:
    platforms = {
        f"{platform.get('os')}/{platform.get('architecture')}"
        for item in manifest.get("manifests", [])
        for platform in (item.get("platform") or {},)
        if platform.get("os") and platform.get("architecture")
    }
    missing = sorted(REQUIRED_PLATFORMS - platforms)
    if missing:
        return [f"{reference} is missing required platforms: {', '.join(missing)}"]
    return []


def verify_image_labels(
    labels: dict[str, Any],
    *,
    reference: str,
    platform: str,
    target_sha: str,
    app_effective_version: str,
) -> list[str]:
    errors: list[str] = []
    _require(
        labels.get(REVISION_LABEL) == target_sha,
        f"{reference} ({platform}) revision label does not match target_sha",
        errors,
    )
    _require(
        labels.get(VERSION_LABEL) == app_effective_version,
        f"{reference} ({platform}) version label does not match app_effective_version",
        errors,
    )
    return errors


def parse_image_proof(value: str) -> tuple[str, Path]:
    reference, separator, path = value.partition("|")
    if not separator or not reference or not path:
        raise PublicationError("image proof must use REFERENCE|JSON_PATH")
    return reference, Path(path)


def parse_image_label_proof(value: str) -> tuple[str, str, Path]:
    reference, separator, remainder = value.partition("|")
    platform, separator, path = remainder.partition("|")
    if not separator or not reference or not platform or not path:
        raise PublicationError("image label proof must use REFERENCE|PLATFORM|JSON_PATH")
    return reference, platform, Path(path)


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise PublicationError(f"{path} must contain a JSON object")
    return value


def verify_publication(
    *,
    release_payload: dict[str, Any],
    tag_target_sha: str,
    release_tag: str,
    target_sha: str,
    release_prerelease: bool,
    image_manifests: dict[str, dict[str, Any]],
    image_labels: dict[tuple[str, str], dict[str, Any]],
    expected_image_refs: set[str],
    app_effective_version: str,
) -> list[str]:
    errors = verify_release_metadata(
        release_payload,
        release_tag=release_tag,
        target_sha=target_sha,
        release_prerelease=release_prerelease,
    )
    errors.extend(verify_tag_target(tag_target_sha, target_sha=target_sha))

    actual_refs = set(image_manifests)
    _require(actual_refs == expected_image_refs, "published image tags do not match the release snapshot", errors)
    for reference, manifest in image_manifests.items():
        errors.extend(verify_manifest(manifest, reference=reference))

    expected_label_keys = {(reference, platform) for reference in expected_image_refs for platform in REQUIRED_PLATFORMS}
    actual_label_keys = set(image_labels)
    _require(actual_label_keys == expected_label_keys, "published image label proofs are incomplete", errors)
    for (reference, platform), labels in image_labels.items():
        if reference not in expected_image_refs or platform not in REQUIRED_PLATFORMS:
            continue
        errors.extend(
            verify_image_labels(
                labels,
                reference=reference,
                platform=platform,
                target_sha=target_sha,
                app_effective_version=app_effective_version,
            )
        )
    return errors


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-json", required=True, type=Path)
    parser.add_argument("--tag-target-sha", required=True)
    parser.add_argument("--release-tag", required=True)
    parser.add_argument("--target-sha", required=True)
    parser.add_argument("--app-effective-version", required=True)
    parser.add_argument("--release-prerelease", action="store_true")
    parser.add_argument("--tags-csv", required=True)
    parser.add_argument("--manifest-proof", action="append", default=[])
    parser.add_argument("--image-label-proof", action="append", default=[])
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        manifests = dict(parse_image_proof(value) for value in args.manifest_proof)
        image_manifests = {reference: load_json(path) for reference, path in manifests.items()}
        label_proofs = {
            (reference, platform): path for reference, platform, path in map(parse_image_label_proof, args.image_label_proof)
        }
        image_labels = {key: load_json(path) for key, path in label_proofs.items()}
        expected_refs = {tag.strip() for tag in args.tags_csv.split(",") if tag.strip()}
        errors = verify_publication(
            release_payload=load_json(args.release_json),
            tag_target_sha=args.tag_target_sha,
            release_tag=args.release_tag,
            target_sha=args.target_sha,
            release_prerelease=args.release_prerelease,
            image_manifests=image_manifests,
            image_labels=image_labels,
            expected_image_refs=expected_refs,
            app_effective_version=args.app_effective_version,
        )
    except (OSError, json.JSONDecodeError, PublicationError) as exc:
        print(f"release publication verification failed: {exc}", file=sys.stderr)
        return 1

    if errors:
        for error in errors:
            print(f"release publication verification failed: {error}", file=sys.stderr)
        return 1

    print(
        "release publication verified: "
        f"tag={args.release_tag} images={len(image_manifests)} "
        f"platforms={','.join(sorted(REQUIRED_PLATFORMS))}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
