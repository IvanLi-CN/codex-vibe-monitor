#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
release_workflow="$repo_root/.github/workflows/release.yml"
grep -q -- "- name: Verify published release surfaces" "$release_workflow"
grep -q "verify_release_publication.py" "$release_workflow"

python3 - "$repo_root/.github/scripts/verify_release_publication.py" <<'PY'
from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
from pathlib import Path

script_path = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("verify_release_publication", script_path)
module = importlib.util.module_from_spec(spec)
assert spec is not None and spec.loader is not None
spec.loader.exec_module(module)

target_sha = "a" * 40
release_tag = "v2.71.8"
app_effective_version = "2.71.8"
refs = {
    "ghcr.io/ivanli-cn/codex-vibe-monitor:v2.71.8",
    "ghcr.io/ivanli-cn/codex-vibe-monitor:latest",
}
manifest = {
    "schemaVersion": 2,
    "manifests": [
        {"platform": {"os": "linux", "architecture": "amd64"}},
        {"platform": {"os": "linux", "architecture": "arm64"}},
    ],
}
release = {
    "tag_name": release_tag,
    "draft": False,
    "prerelease": False,
    "html_url": "https://github.com/example/example/releases/tag/v2.71.8",
}

image_manifests = {reference: manifest for reference in refs}
image_labels = {
    (reference, platform): {
        "org.opencontainers.image.revision": target_sha,
        "org.opencontainers.image.version": app_effective_version,
    }
    for reference in refs
    for platform in module.REQUIRED_PLATFORMS
}

assert module.verify_publication(
    release_payload=release,
    tag_target_sha=target_sha,
    release_tag=release_tag,
    target_sha=target_sha,
    release_prerelease=False,
    image_manifests=image_manifests,
    image_labels=image_labels,
    expected_image_refs=refs,
    app_effective_version=app_effective_version,
) == []

first_ref = next(iter(refs))

def assert_failure(**overrides: object) -> None:
    values = {
        "release_payload": release,
        "tag_target_sha": target_sha,
        "release_tag": release_tag,
        "target_sha": target_sha,
        "release_prerelease": False,
        "image_manifests": image_manifests,
        "image_labels": image_labels,
        "expected_image_refs": refs,
        "app_effective_version": app_effective_version,
    }
    values.update(overrides)
    assert module.verify_publication(**values), overrides

assert_failure(tag_target_sha="b" * 40)
assert_failure(release_payload={**release, "draft": True})
assert_failure(release_payload={**release, "tag_name": "v2.71.7"})
assert_failure(image_manifests={first_ref: manifest}, expected_image_refs=refs)
assert_failure(
    image_manifests={first_ref: {"manifests": [{"platform": {"os": "linux", "architecture": "amd64"}}]},
        **{reference: manifest for reference in refs if reference != first_ref}},
    expected_image_refs=refs,
)
assert_failure(
    image_labels={
        **image_labels,
        (first_ref, "linux/amd64"): {
            "org.opencontainers.image.revision": "b" * 40,
            "org.opencontainers.image.version": app_effective_version,
        },
    }
)
assert_failure(image_labels={key: value for key, value in image_labels.items() if key[1] != "linux/arm64"})

with tempfile.TemporaryDirectory(prefix="release-publication-verification-") as tmp:
    root = Path(tmp)
    release_file = root / "release.json"
    release_file.write_text(json.dumps(release), encoding="utf-8")
    manifest_file = root / "manifest.json"
    manifest_file.write_text(json.dumps(manifest), encoding="utf-8")
    assert module.parse_image_proof(f"{first_ref}|{manifest_file}")[0] in refs
    labels_file = root / "labels.json"
    labels_file.write_text(json.dumps(next(iter(image_labels.values()))), encoding="utf-8")
    assert module.parse_image_label_proof(f"{first_ref}|linux/amd64|{labels_file}")[:2] == (first_ref, "linux/amd64")

    cli_args = [
        "verify_release_publication.py",
        "--release-json",
        str(release_file),
        "--tag-target-sha",
        target_sha,
        "--release-tag",
        release_tag,
        "--target-sha",
        target_sha,
        "--app-effective-version",
        app_effective_version,
        "--tags-csv",
        ",".join(sorted(refs)),
    ]
    for reference in sorted(refs):
        cli_args.extend(("--manifest-proof", f"{reference}|{manifest_file}"))
    for reference, platform in sorted(image_labels):
        cli_args.extend(("--image-label-proof", f"{reference}|{platform}|{labels_file}"))

    original_argv = sys.argv
    sys.argv = cli_args
    try:
        assert module.main() == 0
    finally:
        sys.argv = original_argv

print("test-release-publication-verification: all checks passed")
PY
