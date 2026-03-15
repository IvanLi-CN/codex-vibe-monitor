#!/usr/bin/env python3
from __future__ import annotations

import argparse
import io
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import zipfile
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any
from urllib import error, parse, request

SNAPSHOT_SCHEMA_VERSION = 1
RELEASE_INTENT_SCHEMA_VERSION = 1
DEFAULT_NOTES_REF = "refs/notes/release-snapshots"
LEGACY_LABEL_FALLBACK_MAX_PR_NUMBER = 130
RELEASE_INTENT_ARTIFACT_PREFIX = "release-intent-pr-"
ALLOWED_SNAPSHOT_SOURCES = {"ci-main", "pr-intent-artifact", "legacy-pr-labels"}
ALLOWED_TYPE_LABELS = {
    "type:patch",
    "type:minor",
    "type:major",
    "type:docs",
    "type:skip",
}
ALLOWED_CHANNEL_LABELS = {"channel:stable", "channel:rc"}
STABLE_TAG_RE = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")


class SnapshotError(RuntimeError):
    pass


@dataclass(frozen=True, order=True)
class StableVersion:
    major: int
    minor: int
    patch: int

    @classmethod
    def parse(cls, value: str) -> "StableVersion":
        match = STABLE_TAG_RE.fullmatch(f"v{value}")
        if not match:
            raise SnapshotError(f"Invalid stable version: {value}")
        return cls(*(int(part) for part in match.groups()))

    @classmethod
    def from_tag(cls, tag: str) -> "StableVersion | None":
        match = STABLE_TAG_RE.fullmatch(tag)
        if not match:
            return None
        return cls(*(int(part) for part in match.groups()))

    def bump(self, bump: str) -> "StableVersion":
        if bump == "patch":
            return StableVersion(self.major, self.minor, self.patch + 1)
        if bump == "minor":
            return StableVersion(self.major, self.minor + 1, 0)
        if bump == "major":
            return StableVersion(self.major + 1, 0, 0)
        raise SnapshotError(f"Unknown release bump: {bump}")

    def render(self) -> str:
        return f"{self.major}.{self.minor}.{self.patch}"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Manage immutable release snapshots stored in git notes.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    ensure = subparsers.add_parser("ensure", help="Create or reuse the immutable snapshot for a main commit.")
    ensure.add_argument("--target-sha", required=True)
    ensure.add_argument("--github-repository", required=True)
    ensure.add_argument("--github-token", required=True)
    ensure.add_argument("--notes-ref", default=DEFAULT_NOTES_REF)
    ensure.add_argument("--registry", default="ghcr.io")
    ensure.add_argument("--api-root", default=os.environ.get("GITHUB_API_URL", "https://api.github.com"))
    ensure.add_argument("--output", required=True)
    ensure.add_argument("--max-attempts", type=int, default=6)

    export_cmd = subparsers.add_parser("export", help="Export a stored release snapshot into GitHub outputs.")
    export_cmd.add_argument("--target-sha", required=True)
    export_cmd.add_argument("--notes-ref", default=DEFAULT_NOTES_REF)
    export_cmd.add_argument("--main-ref", default="")
    export_cmd.add_argument(
        "--resolve-publication-tags",
        action="store_true",
        help="Re-resolve stable manifest tags so superseded releases stop updating latest.",
    )
    export_cmd.add_argument("--github-output", default=os.environ.get("GITHUB_OUTPUT", ""))

    return parser.parse_args()


def git(*args: str, check: bool = True, capture_output: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        ["git", *args],
        check=False,
        text=True,
        capture_output=capture_output,
    )
    if check and result.returncode != 0:
        stderr = result.stderr.strip()
        stdout = result.stdout.strip()
        detail = stderr or stdout or f"git {' '.join(args)} failed"
        raise SnapshotError(detail)
    return result


def git_output(*args: str) -> str:
    return git(*args).stdout.strip()


def note_exists(notes_ref: str, target_sha: str) -> bool:
    return git("notes", f"--ref={notes_ref}", "show", target_sha, check=False).returncode == 0


def read_snapshot(notes_ref: str, target_sha: str) -> dict[str, Any] | None:
    result = git("notes", f"--ref={notes_ref}", "show", target_sha, check=False)
    if result.returncode != 0:
        return None
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise SnapshotError(f"Release snapshot note for {target_sha} is not valid JSON") from exc
    return validate_snapshot(payload, expected_sha=target_sha)


def validate_snapshot(payload: Any, *, expected_sha: str | None = None) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise SnapshotError("Release snapshot note must decode to an object")
    if payload.get("schema_version") != SNAPSHOT_SCHEMA_VERSION:
        raise SnapshotError(f"Unsupported release snapshot schema: {payload.get('schema_version')!r}")
    target_sha = payload.get("target_sha")
    if not isinstance(target_sha, str) or not re.fullmatch(r"[0-9a-f]{40}", target_sha):
        raise SnapshotError("Release snapshot target_sha must be a 40-char commit SHA")
    if expected_sha and target_sha != expected_sha:
        raise SnapshotError(f"Release snapshot target_sha mismatch: expected {expected_sha}, got {target_sha}")

    required_strings = [
        "type_label",
        "channel_label",
        "release_bump",
        "release_channel",
        "image_name_lower",
        "snapshot_source",
    ]
    for key in required_strings:
        value = payload.get(key)
        if not isinstance(value, str) or not value:
            raise SnapshotError(f"Release snapshot {key} must be a non-empty string")

    if not isinstance(payload.get("release_enabled"), bool):
        raise SnapshotError("Release snapshot release_enabled must be boolean")
    if not isinstance(payload.get("release_prerelease"), bool):
        raise SnapshotError("Release snapshot release_prerelease must be boolean")

    pr_number = payload.get("pr_number")
    if pr_number is not None and not isinstance(pr_number, int):
        raise SnapshotError("Release snapshot pr_number must be an integer or null")
    pr_title = payload.get("pr_title")
    if pr_title is not None and not isinstance(pr_title, str):
        raise SnapshotError("Release snapshot pr_title must be a string or null")
    pr_head_sha = payload.get("pr_head_sha")
    if pr_head_sha not in (None, "") and (
        not isinstance(pr_head_sha, str) or not re.fullmatch(r"[0-9a-f]{40}", pr_head_sha)
    ):
        raise SnapshotError("Release snapshot pr_head_sha must be a 40-char commit SHA when present")
    if payload.get("snapshot_source") not in ALLOWED_SNAPSHOT_SOURCES:
        raise SnapshotError(
            f"Release snapshot snapshot_source must be one of {', '.join(sorted(ALLOWED_SNAPSHOT_SOURCES))}"
        )

    if payload["release_enabled"]:
        for key in ("base_stable_version", "next_stable_version", "app_effective_version", "release_tag", "tags_csv"):
            value = payload.get(key)
            if not isinstance(value, str) or not value:
                raise SnapshotError(f"Release snapshot {key} must be a non-empty string when release_enabled=true")
        StableVersion.parse(payload["base_stable_version"])
        StableVersion.parse(payload["next_stable_version"])
        if not str(payload["release_tag"]).startswith("v"):
            raise SnapshotError("Release snapshot release_tag must start with 'v'")
    else:
        for key in ("base_stable_version", "next_stable_version", "app_effective_version", "release_tag", "tags_csv"):
            value = payload.get(key)
            if value not in (None, ""):
                raise SnapshotError(f"Release snapshot {key} must be empty when release_enabled=false")

    return payload


def fetch_notes_ref(notes_ref: str) -> None:
    probe = git("ls-remote", "--exit-code", "origin", notes_ref, check=False)
    if probe.returncode != 0:
        return
    git("fetch", "--no-tags", "origin", f"+{notes_ref}:{notes_ref}")


def normalize_sha(target_sha: str) -> str:
    if not re.fullmatch(r"[0-9a-f]{40}", target_sha):
        raise SnapshotError(f"Invalid target SHA: {target_sha}")
    git("cat-file", "-e", f"{target_sha}^{{commit}}")
    return target_sha


def github_request_json(api_root: str, token: str, path: str, query: dict[str, Any] | None = None) -> Any:
    url = f"{api_root.rstrip('/')}{path}"
    if query:
        url += "?" + parse.urlencode(query)
    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json, application/vnd.github.groot-preview+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "codex-vibe-monitor-release-snapshot",
    }
    req = request.Request(url, headers=headers)
    try:
        with request.urlopen(req) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise SnapshotError(f"GitHub API error on {path}: {exc.code} {body}") from exc
    except error.URLError as exc:
        raise SnapshotError(f"GitHub API request failed on {path}: {exc}") from exc


def github_request_bytes(url: str, token: str) -> bytes:
    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/octet-stream",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "codex-vibe-monitor-release-snapshot",
    }
    req = request.Request(url, headers=headers)
    try:
        with request.urlopen(req) as resp:
            return resp.read()
    except error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise SnapshotError(f"GitHub artifact download failed on {url}: {exc.code} {body}") from exc
    except error.URLError as exc:
        raise SnapshotError(f"GitHub artifact download failed on {url}: {exc}") from exc


def github_paginate(api_root: str, token: str, path: str) -> list[dict[str, Any]]:
    page = 1
    items: list[dict[str, Any]] = []
    while True:
        payload = github_request_json(api_root, token, path, {"per_page": 100, "page": page})
        if not isinstance(payload, list):
            raise SnapshotError(f"GitHub API returned an unexpected payload for {path}")
        normalized = [item for item in payload if isinstance(item, dict)]
        items.extend(normalized)
        if len(payload) < 100:
            return items
        page += 1


def load_pr_for_commit(
    api_root: str,
    repository: str,
    token: str,
    target_sha: str,
    *,
    allow_zero: bool = False,
) -> dict[str, Any] | None:
    owner, repo = repository.split("/", 1)
    payload = github_request_json(api_root, token, f"/repos/{owner}/{repo}/commits/{target_sha}/pulls")
    if not isinstance(payload, list):
        raise SnapshotError("GitHub API returned an unexpected payload for commit-associated PRs")
    if len(payload) == 0 and allow_zero:
        return None
    if len(payload) != 1:
        raise SnapshotError(f"Expected exactly 1 PR associated with commit {target_sha}, got {len(payload)}")
    pr_summary = payload[0]
    if not isinstance(pr_summary, dict):
        raise SnapshotError("GitHub API returned a malformed PR payload")
    pr_number = pr_summary.get("number")
    if not isinstance(pr_number, int):
        raise SnapshotError("Commit-associated PR payload is missing a numeric PR number")
    pr = github_request_json(api_root, token, f"/repos/{owner}/{repo}/pulls/{pr_number}")
    if not isinstance(pr, dict):
        raise SnapshotError("GitHub API returned a malformed pull request payload")
    return pr


def artifact_name_for_pr(pr_number: int, pr_head_sha: str) -> str:
    return f"{RELEASE_INTENT_ARTIFACT_PREFIX}{pr_number}-{pr_head_sha}"


def validate_release_intent(
    payload: Any, *, expected_pr_number: int | None = None, expected_head_sha: str | None = None
) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise SnapshotError("Release intent artifact must decode to an object")
    if payload.get("schema_version") != RELEASE_INTENT_SCHEMA_VERSION:
        raise SnapshotError(f"Unsupported release intent schema: {payload.get('schema_version')!r}")

    pr_number = payload.get("pr_number")
    if not isinstance(pr_number, int):
        raise SnapshotError("Release intent pr_number must be an integer")
    if expected_pr_number is not None and pr_number != expected_pr_number:
        raise SnapshotError(f"Release intent pr_number mismatch: expected {expected_pr_number}, got {pr_number}")

    pr_head_sha = payload.get("pr_head_sha")
    if not isinstance(pr_head_sha, str) or not re.fullmatch(r"[0-9a-f]{40}", pr_head_sha):
        raise SnapshotError("Release intent pr_head_sha must be a 40-char commit SHA")
    if expected_head_sha is not None and pr_head_sha != expected_head_sha:
        raise SnapshotError(f"Release intent pr_head_sha mismatch: expected {expected_head_sha}, got {pr_head_sha}")

    for key, allowed in (("type_label", ALLOWED_TYPE_LABELS), ("channel_label", ALLOWED_CHANNEL_LABELS)):
        value = payload.get(key)
        if not isinstance(value, str) or value not in allowed:
            raise SnapshotError(f"Release intent {key} must be one of {', '.join(sorted(allowed))}")

    created_at = payload.get("created_at")
    if not isinstance(created_at, str) or not created_at:
        raise SnapshotError("Release intent created_at must be a non-empty string")

    return payload


def current_labels_for_pr(api_root: str, repository: str, token: str, pr_number: int) -> list[str]:
    owner, repo = repository.split("/", 1)
    payload = github_request_json(api_root, token, f"/repos/{owner}/{repo}/issues/{pr_number}")
    if not isinstance(payload, dict):
        raise SnapshotError(f"GitHub API returned a malformed issue payload for PR #{pr_number}")
    labels = payload.get("labels") or []
    if not isinstance(labels, list):
        raise SnapshotError(f"GitHub API returned malformed labels for PR #{pr_number}")
    names = [str(label.get("name")) for label in labels if isinstance(label, dict) and label.get("name")]
    return sorted(set(names))


def load_release_intent_artifact(
    api_root: str, repository: str, token: str, pr_number: int, pr_head_sha: str
) -> dict[str, Any] | None:
    owner, repo = repository.split("/", 1)
    artifact_name = artifact_name_for_pr(pr_number, pr_head_sha)
    payload = github_request_json(
        api_root,
        token,
        f"/repos/{owner}/{repo}/actions/artifacts",
        {"per_page": 100, "name": artifact_name},
    )
    if not isinstance(payload, dict):
        raise SnapshotError("GitHub API returned an unexpected artifact listing payload")
    artifacts = payload.get("artifacts") or []
    if not isinstance(artifacts, list):
        raise SnapshotError("GitHub API returned malformed artifacts data")

    candidates = [
        artifact
        for artifact in artifacts
        if isinstance(artifact, dict)
        and artifact.get("name") == artifact_name
        and artifact.get("expired") is False
    ]
    if not candidates:
        return None

    candidates.sort(key=lambda artifact: str(artifact.get("created_at") or ""), reverse=True)
    artifact = candidates[0]
    archive_url = artifact.get("archive_download_url")
    if not isinstance(archive_url, str) or not archive_url:
        raise SnapshotError(f"Artifact {artifact_name} is missing archive_download_url")

    raw_bytes = github_request_bytes(archive_url, token)
    with zipfile.ZipFile(io.BytesIO(raw_bytes)) as archive:
        members = [name for name in archive.namelist() if name.endswith(".json")]
        if len(members) != 1:
            raise SnapshotError(f"Artifact {artifact_name} must contain exactly one JSON file")
        try:
            payload = json.loads(archive.read(members[0]).decode("utf-8"))
        except json.JSONDecodeError as exc:
            raise SnapshotError(f"Artifact {artifact_name} does not contain valid JSON") from exc
    return validate_release_intent(payload, expected_pr_number=pr_number, expected_head_sha=pr_head_sha)


def parse_release_labels(labels: list[str]) -> tuple[str, str]:
    type_labels = [label for label in labels if label.startswith("type:")]
    channel_labels = [label for label in labels if label.startswith("channel:")]

    if len(type_labels) != 1:
        raise SnapshotError(
            f"Expected exactly 1 type:* label, got {len(type_labels)}: {', '.join(type_labels) or '(none)'}"
        )
    if len(channel_labels) != 1:
        raise SnapshotError(
            f"Expected exactly 1 channel:* label, got {len(channel_labels)}: {', '.join(channel_labels) or '(none)'}"
        )

    type_label = type_labels[0]
    channel_label = channel_labels[0]
    if type_label not in ALLOWED_TYPE_LABELS:
        raise SnapshotError(f"Unknown type label: {type_label}")
    if channel_label not in ALLOWED_CHANNEL_LABELS:
        raise SnapshotError(f"Unknown channel label: {channel_label}")
    return type_label, channel_label


def resolve_release_intent_for_pr(
    api_root: str, repository: str, token: str, pr: dict[str, Any]
) -> tuple[str, str, str, str]:
    pr_number = pr.get("number")
    if not isinstance(pr_number, int):
        raise SnapshotError("Pull request payload is missing a numeric PR number")
    head = pr.get("head") or {}
    pr_head_sha = head.get("sha") if isinstance(head, dict) else None
    if not isinstance(pr_head_sha, str) or not re.fullmatch(r"[0-9a-f]{40}", pr_head_sha):
        raise SnapshotError(f"Pull request #{pr_number} is missing a valid head.sha")

    release_intent = load_release_intent_artifact(api_root, repository, token, pr_number, pr_head_sha)
    if release_intent is not None:
        return (
            str(release_intent["type_label"]),
            str(release_intent["channel_label"]),
            "pr-intent-artifact",
            pr_head_sha,
        )

    if pr_number <= LEGACY_LABEL_FALLBACK_MAX_PR_NUMBER:
        type_label, channel_label = parse_release_labels(current_labels_for_pr(api_root, repository, token, pr_number))
        return (type_label, channel_label, "legacy-pr-labels", pr_head_sha)

    artifact_name = artifact_name_for_pr(pr_number, pr_head_sha)
    raise SnapshotError(
        f"Missing pre-frozen release intent artifact {artifact_name} for PR #{pr_number}; "
        "legacy label fallback is only allowed for historical releases"
    )


def cargo_base_version(target_sha: str) -> StableVersion:
    cargo_toml = git_output("show", f"{target_sha}:Cargo.toml")
    match = re.search(r'^version\s*=\s*"(\d+\.\d+\.\d+)"', cargo_toml, re.MULTILINE)
    if not match:
        raise SnapshotError("Failed to detect version from Cargo.toml")
    return StableVersion.parse(match.group(1))


def stable_versions_from_tags(target_sha: str) -> list[StableVersion]:
    tags = git_output("tag", "--merged", target_sha, "-l", "v*").splitlines()
    versions: list[StableVersion] = []
    for tag in tags:
        version = StableVersion.from_tag(tag.strip())
        if version is not None:
            versions.append(version)
    return versions


def stable_versions_from_snapshots(notes_ref: str, target_sha: str) -> list[StableVersion]:
    commits = git_output("rev-list", "--first-parent", target_sha).splitlines()
    versions: list[StableVersion] = []
    for commit in commits[1:]:
        snapshot = read_snapshot(notes_ref, commit)
        if not snapshot or not snapshot.get("release_enabled"):
            continue
        if snapshot.get("release_channel") != "stable":
            continue
        next_stable = snapshot.get("next_stable_version")
        if not isinstance(next_stable, str):
            raise SnapshotError(f"Stable snapshot for {commit} is missing next_stable_version")
        versions.append(StableVersion.parse(next_stable))
    return versions


def compute_base_stable_version(notes_ref: str, target_sha: str) -> StableVersion:
    candidates = stable_versions_from_tags(target_sha)
    candidates.extend(stable_versions_from_snapshots(notes_ref, target_sha))
    if not candidates:
        return cargo_base_version(target_sha)
    return max(candidates)


def commits_after_target(main_ref: str, target_sha: str) -> list[str]:
    git("merge-base", "--is-ancestor", target_sha, main_ref)
    commits = git_output("rev-list", "--first-parent", f"{target_sha}..{main_ref}")
    return [commit for commit in commits.splitlines() if commit]


def has_newer_stable_snapshot(notes_ref: str, main_ref: str, target_sha: str) -> bool:
    for commit in commits_after_target(main_ref, target_sha):
        snapshot = read_snapshot(notes_ref, commit)
        if not snapshot or not snapshot.get("release_enabled"):
            continue
        if snapshot.get("release_channel") != "stable":
            continue
        return True
    return False


def publication_tags(snapshot: dict[str, Any], *, notes_ref: str, main_ref: str) -> str:
    if not snapshot.get("release_enabled"):
        return ""

    image = f"{snapshot['registry']}/{snapshot['image_name_lower']}"
    release_tag = str(snapshot["release_tag"])
    tags = [f"{image}:{release_tag}"]
    if snapshot.get("release_channel") == "stable" and not has_newer_stable_snapshot(
        notes_ref, main_ref, str(snapshot["target_sha"])
    ):
        tags.append(f"{image}:latest")
    return ",".join(tags)


def build_snapshot(
    *,
    target_sha: str,
    repository: str,
    token: str,
    notes_ref: str,
    registry: str,
    api_root: str,
    pr: dict[str, Any] | None = None,
) -> dict[str, Any]:
    if pr is None:
        pr = load_pr_for_commit(api_root, repository, token, target_sha)
    if pr is None:
        raise SnapshotError(f"Commit {target_sha} is not associated with a merged pull request")
    type_label, channel_label, snapshot_source, pr_head_sha = resolve_release_intent_for_pr(
        api_root, repository, token, pr
    )
    release_bump = type_label.split(":", 1)[1]
    release_channel = channel_label.split(":", 1)[1]
    image_name_lower = repository.lower()
    snapshot: dict[str, Any] = {
        "schema_version": SNAPSHOT_SCHEMA_VERSION,
        "target_sha": target_sha,
        "pr_number": pr.get("number"),
        "pr_title": pr.get("title") or "",
        "registry": registry,
        "pr_head_sha": pr_head_sha,
        "type_label": type_label,
        "channel_label": channel_label,
        "release_bump": release_bump,
        "release_channel": release_channel,
        "release_enabled": type_label not in {"type:docs", "type:skip"},
        "release_prerelease": False,
        "image_name_lower": image_name_lower,
        "base_stable_version": "",
        "next_stable_version": "",
        "app_effective_version": "",
        "release_tag": "",
        "tags_csv": "",
        "notes_ref": notes_ref,
        "snapshot_source": snapshot_source,
        "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }

    if snapshot["release_enabled"]:
        base = compute_base_stable_version(notes_ref, target_sha)
        next_stable = base.bump(release_bump)
        effective = next_stable.render()
        prerelease = False
        if release_channel == "rc":
            effective = f"{effective}-rc.{target_sha[:7]}"
            prerelease = True

        snapshot.update(
            {
                "base_stable_version": base.render(),
                "next_stable_version": next_stable.render(),
                "app_effective_version": effective,
                "release_tag": f"v{effective}",
                "release_prerelease": prerelease,
            }
        )
        image = f"{registry}/{image_name_lower}"
        if release_channel == "stable":
            snapshot["tags_csv"] = f"{image}:{snapshot['release_tag']},{image}:latest"
        else:
            snapshot["tags_csv"] = f"{image}:{snapshot['release_tag']}"

    return validate_snapshot(snapshot, expected_sha=target_sha)


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")


def first_parent_commits(target_sha: str) -> list[str]:
    commits = git_output("rev-list", "--first-parent", "--reverse", target_sha)
    return [commit for commit in commits.splitlines() if commit]


def export_snapshot(snapshot: dict[str, Any], github_output: str) -> None:
    lines = []
    for key in (
        "target_sha",
        "release_enabled",
        "release_bump",
        "release_channel",
        "pr_number",
        "pr_title",
        "image_name_lower",
        "app_effective_version",
        "release_tag",
        "release_prerelease",
        "tags_csv",
    ):
        value = snapshot.get(key)
        if isinstance(value, bool):
            rendered = "true" if value else "false"
        elif value is None:
            rendered = ""
        else:
            rendered = str(value)
        if "\n" in rendered:
            lines.append(f"{key}<<__CODex__")
            lines.append(rendered)
            lines.append("__CODex__")
        else:
            lines.append(f"{key}={rendered}")
    payload = "\n".join(lines) + "\n"
    if github_output:
        with Path(github_output).open("a", encoding="utf-8") as handle:
            handle.write(payload)
    else:
        sys.stdout.write(payload)


def ensure_snapshot(args: argparse.Namespace) -> int:
    target_sha = normalize_sha(args.target_sha)
    output_path = Path(args.output)

    for attempt in range(1, args.max_attempts + 1):
        fetch_notes_ref(args.notes_ref)
        existing = read_snapshot(args.notes_ref, target_sha)
        if existing is not None:
            write_json(output_path, existing)
            return 0

        target_snapshot: dict[str, Any] | None = None
        with tempfile.TemporaryDirectory(prefix="release-snapshot-notes-") as tmp:
            temp_note = Path(tmp) / "snapshot.json"
            for commit in first_parent_commits(target_sha):
                snapshot = read_snapshot(args.notes_ref, commit)
                if snapshot is not None:
                    if commit == target_sha:
                        target_snapshot = snapshot
                        break
                    continue

                pr = load_pr_for_commit(
                    args.api_root,
                    args.github_repository,
                    args.github_token,
                    commit,
                    allow_zero=(commit != target_sha),
                )
                if pr is None:
                    continue

                snapshot = build_snapshot(
                    target_sha=commit,
                    repository=args.github_repository,
                    token=args.github_token,
                    notes_ref=args.notes_ref,
                    registry=args.registry,
                    api_root=args.api_root,
                    pr=pr,
                )
                write_json(temp_note, snapshot)
                git("notes", f"--ref={args.notes_ref}", "add", "-f", "-F", str(temp_note), commit)
                if commit == target_sha:
                    target_snapshot = snapshot

        if target_snapshot is None:
            raise SnapshotError(f"Failed to materialize release snapshot for {target_sha}")

        write_json(output_path, target_snapshot)

        # The notes ref acts like a CAS register: if another allocator wins first,
        # our stale push is rejected and we recompute against the newer remote tip.
        push = git("push", "origin", args.notes_ref, check=False)
        if push.returncode == 0:
            return 0

        if attempt == args.max_attempts:
            detail = push.stderr.strip() or push.stdout.strip() or "git push origin notes ref failed"
            raise SnapshotError(f"Failed to publish release snapshot after {attempt} attempts: {detail}")

    raise SnapshotError("release snapshot retry loop exhausted unexpectedly")


def export_existing_snapshot(args: argparse.Namespace) -> int:
    target_sha = normalize_sha(args.target_sha)
    fetch_notes_ref(args.notes_ref)
    snapshot = read_snapshot(args.notes_ref, target_sha)
    if snapshot is None:
        raise SnapshotError(f"Missing immutable release snapshot for {target_sha}")
    if args.resolve_publication_tags:
        if not args.main_ref:
            raise SnapshotError("--main-ref is required when --resolve-publication-tags is set")
        snapshot = dict(snapshot)
        snapshot["tags_csv"] = publication_tags(snapshot, notes_ref=args.notes_ref, main_ref=args.main_ref)
    export_snapshot(snapshot, args.github_output)
    return 0


def main() -> int:
    args = parse_args()
    try:
        if args.command == "ensure":
            return ensure_snapshot(args)
        if args.command == "export":
            return export_existing_snapshot(args)
        raise SnapshotError(f"Unsupported command: {args.command}")
    except SnapshotError as exc:
        print(f"release_snapshot.py: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
