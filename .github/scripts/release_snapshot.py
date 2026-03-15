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
RELEASE_INTENT_ARTIFACT_PREFIX = "release-intent-pr-"
TRUSTED_RELEASE_INTENT_WORKFLOW_PATH = ".github/workflows/label-gate.yml"
TRUSTED_RELEASE_INTENT_EVENT = "pull_request"
ALLOWED_SNAPSHOT_SOURCES = {"ci-main", "pr-intent-artifact", "legacy-pr-labels"}
RELEASE_INTENT_SUPPORT_PATHS = (
    ".github/quality-gates.json",
    ".github/scripts/check_quality_gates_contract.py",
    ".github/scripts/metadata_gate.py",
    ".github/workflows/ci-pr.yml",
    ".github/workflows/ci-main.yml",
    ".github/workflows/release.yml",
    ".github/workflows/label-gate.yml",
    ".github/workflows/review-policy.yml",
)
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
    ensure.add_argument(
        "--allow-current-pr-label-fallback",
        action="store_true",
        help="Allow historical backfill to fall back to the merged PR's current labels when no frozen intent artifact exists.",
    )

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
    if not isinstance(payload.get("snapshot_source"), str) or not payload.get("snapshot_source"):
        payload = dict(payload)
        payload["snapshot_source"] = "ci-main"

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


def github_paginate_artifacts(api_root: str, token: str, path: str) -> list[dict[str, Any]]:
    page = 1
    items: list[dict[str, Any]] = []
    while True:
        payload = github_request_json(api_root, token, path, {"per_page": 100, "page": page})
        if not isinstance(payload, dict):
            raise SnapshotError(f"GitHub API returned an unexpected artifacts payload for {path}")
        artifacts = payload.get("artifacts") or []
        if not isinstance(artifacts, list):
            raise SnapshotError(f"GitHub API returned malformed artifacts data for {path}")
        normalized = [item for item in artifacts if isinstance(item, dict)]
        items.extend(normalized)
        if len(artifacts) < 100:
            return items
        page += 1


def parse_github_timestamp(value: str, *, where: str) -> datetime:
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as exc:
        raise SnapshotError(f"{where} must be an ISO-8601 timestamp") from exc


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


def labels_at_merge_time(api_root: str, repository: str, token: str, pr: dict[str, Any]) -> list[str]:
    owner, repo = repository.split("/", 1)
    pr_number = pr.get("number")
    merged_at = pr.get("merged_at")
    if not isinstance(pr_number, int):
        raise SnapshotError("Pull request payload is missing a numeric PR number")
    if not isinstance(merged_at, str) or not merged_at:
        raise SnapshotError(f"Pull request #{pr_number} is missing merged_at; cannot freeze release labels")

    merge_moment = parse_github_timestamp(merged_at, where=f"Pull request #{pr_number} merged_at")
    timeline = github_paginate(api_root, token, f"/repos/{owner}/{repo}/issues/{pr_number}/timeline")

    labels: set[str] = set()
    for event in sorted(timeline, key=lambda item: str(item.get("created_at", ""))):
        created_at = event.get("created_at")
        if not isinstance(created_at, str):
            continue
        event_moment = parse_github_timestamp(created_at, where=f"Pull request #{pr_number} timeline created_at")
        if event_moment > merge_moment:
            continue
        event_type = event.get("event")
        label = event.get("label")
        if not isinstance(label, dict):
            continue
        name = label.get("name")
        if not isinstance(name, str):
            continue
        if event_type == "labeled":
            labels.add(name)
        elif event_type == "unlabeled":
            labels.discard(name)
    return sorted(labels)


def current_pr_labels(pr: dict[str, Any]) -> list[str]:
    labels = pr.get("labels")
    if not isinstance(labels, list):
        raise SnapshotError("Pull request payload is missing labels")
    names: list[str] = []
    for label in labels:
        if isinstance(label, str):
            names.append(label)
            continue
        if isinstance(label, dict):
            name = label.get("name")
            if isinstance(name, str):
                names.append(name)
    return sorted(names)


def repo_root_supports_release_intent_artifact(repo_root: Path) -> bool:
    contract_script = repo_root / ".github/scripts/check_quality_gates_contract.py"
    metadata_script = repo_root / ".github/scripts/metadata_gate.py"
    if not contract_script.is_file() or not metadata_script.is_file():
        return False

    contract_check = subprocess.run(
        [sys.executable, str(contract_script), "--repo-root", str(repo_root), "--profile", "final"],
        check=False,
        text=True,
        capture_output=True,
    )
    if contract_check.returncode != 0:
        return False

    metadata_help = subprocess.run(
        [sys.executable, str(metadata_script), "label", "--help"],
        check=False,
        text=True,
        capture_output=True,
    )
    if metadata_help.returncode != 0:
        return False

    help_output = f"{metadata_help.stdout}\n{metadata_help.stderr}"
    return "--write-intent" in help_output


def checkout_commit_file(commit_sha: str, path: str, destination: Path) -> bool:
    result = git("show", f"{commit_sha}:{path}", check=False)
    if result.returncode != 0:
        return False
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(result.stdout)
    return True


def commit_supports_release_intent_artifact(commit_sha: str) -> bool:
    with tempfile.TemporaryDirectory(prefix="release-intent-support-") as tmp:
        repo_root = Path(tmp)
        for path in RELEASE_INTENT_SUPPORT_PATHS:
            if not checkout_commit_file(commit_sha, path, repo_root / path):
                return False
        return repo_root_supports_release_intent_artifact(repo_root)


def support_rollout_moment_for_target(target_sha: str) -> datetime | None:
    parents = git_output("rev-list", "--parents", "-n", "1", target_sha).split()
    if len(parents) <= 1:
        return None
    previous_main_sha = parents[1]
    if not commit_supports_release_intent_artifact(previous_main_sha):
        return None

    for commit_sha in git_output("rev-list", "--first-parent", "--reverse", previous_main_sha).splitlines():
        if commit_supports_release_intent_artifact(commit_sha):
            committed_at = git_output("show", "-s", "--format=%cI", commit_sha)
            return parse_github_timestamp(committed_at, where=f"Mainline support commit {commit_sha} committed_at")
    raise SnapshotError("Failed to locate the first mainline commit that introduced release-intent artifact support")


def pr_had_rollout_trigger_after(
    api_root: str, repository: str, token: str, pr: dict[str, Any], rollout_moment: datetime
) -> bool:
    pr_number = pr.get("number")
    created_at = pr.get("created_at")
    if not isinstance(pr_number, int):
        raise SnapshotError("Pull request payload is missing a numeric PR number")
    if isinstance(created_at, str) and parse_github_timestamp(created_at, where=f"Pull request #{pr_number} created_at") >= rollout_moment:
        return True

    owner, repo = repository.split("/", 1)
    rollout_events = {"reopened", "synchronize", "labeled", "unlabeled", "ready_for_review", "edited"}
    timeline = github_paginate(api_root, token, f"/repos/{owner}/{repo}/issues/{pr_number}/timeline")
    for event in timeline:
        if not isinstance(event, dict):
            continue
        event_type = event.get("event")
        created_at = event.get("created_at")
        if event_type not in rollout_events or not isinstance(created_at, str):
            continue
        if parse_github_timestamp(created_at, where=f"Pull request #{pr_number} timeline created_at") >= rollout_moment:
            return True
    return False


def legacy_fallback_allowed_for_target(
    api_root: str, repository: str, token: str, pr: dict[str, Any], *, target_sha: str
) -> bool:
    rollout_moment = support_rollout_moment_for_target(target_sha)
    if rollout_moment is None:
        return True
    return not pr_had_rollout_trigger_after(api_root, repository, token, pr, rollout_moment)


def merged_pr_head_sha(target_sha: str) -> str | None:
    parents = git_output("rev-list", "--parents", "-n", "1", target_sha).split()
    if len(parents) >= 3 and re.fullmatch(r"[0-9a-f]{40}", parents[2]):
        return parents[2]
    return None


def load_release_intent_artifact(
    api_root: str, repository: str, token: str, pr_number: int, *, merged_at: str, expected_head_sha: str | None = None
) -> dict[str, Any] | None:
    owner, repo = repository.split("/", 1)
    artifact_prefix = f"{RELEASE_INTENT_ARTIFACT_PREFIX}{pr_number}-"
    merge_moment = parse_github_timestamp(merged_at, where=f"Pull request #{pr_number} merged_at")
    artifacts = github_paginate_artifacts(api_root, token, f"/repos/{owner}/{repo}/actions/artifacts")

    candidates: list[dict[str, Any]] = []
    workflow_runs: dict[int, dict[str, Any]] = {}
    for artifact in artifacts:
        if not isinstance(artifact, dict):
            continue
        artifact_name = artifact.get("name")
        if not isinstance(artifact_name, str) or not artifact_name.startswith(artifact_prefix):
            continue
        if artifact.get("expired") is not False:
            continue
        created_at = artifact.get("created_at")
        if not isinstance(created_at, str):
            continue
        if parse_github_timestamp(created_at, where=f"Artifact {artifact_name} created_at") > merge_moment:
            continue
        workflow_run = artifact.get("workflow_run")
        if not isinstance(workflow_run, dict):
            raise SnapshotError(f"Artifact {artifact_name} is missing workflow_run metadata")
        run_id = workflow_run.get("id")
        if not isinstance(run_id, int):
            raise SnapshotError(f"Artifact {artifact_name} is missing a numeric workflow_run.id")
        run_payload = workflow_runs.get(run_id)
        if run_payload is None:
            run_payload = github_request_json(api_root, token, f"/repos/{owner}/{repo}/actions/runs/{run_id}")
            if not isinstance(run_payload, dict):
                raise SnapshotError(f"GitHub API returned malformed workflow run data for artifact {artifact_name}")
            workflow_runs[run_id] = run_payload
        if run_payload.get("path") != TRUSTED_RELEASE_INTENT_WORKFLOW_PATH:
            continue
        if run_payload.get("event") != TRUSTED_RELEASE_INTENT_EVENT:
            continue
        if run_payload.get("status") != "completed":
            continue
        if run_payload.get("conclusion") != "success":
            continue
        pull_requests = run_payload.get("pull_requests") or []
        if not isinstance(pull_requests, list):
            raise SnapshotError(f"GitHub API returned malformed workflow run pull_requests for artifact {artifact_name}")
        if not any(
            isinstance(item, dict)
            and item.get("number") == pr_number
            and (
                expected_head_sha is None
                or (isinstance(item.get("head"), dict) and item["head"].get("sha") == expected_head_sha)
            )
            for item in pull_requests
        ):
            continue
        candidates.append(artifact)
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
    return validate_release_intent(payload, expected_pr_number=pr_number, expected_head_sha=expected_head_sha)


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
    api_root: str,
    repository: str,
    token: str,
    pr: dict[str, Any],
    *,
    target_sha: str,
    allow_current_pr_label_fallback: bool = False,
) -> tuple[str, str, str, str]:
    pr_number = pr.get("number")
    if not isinstance(pr_number, int):
        raise SnapshotError("Pull request payload is missing a numeric PR number")
    merged_at = pr.get("merged_at")
    if not isinstance(merged_at, str) or not merged_at:
        raise SnapshotError(f"Pull request #{pr_number} is missing merged_at")
    expected_head_sha = merged_pr_head_sha(target_sha)

    release_intent = load_release_intent_artifact(
        api_root,
        repository,
        token,
        pr_number,
        merged_at=merged_at,
        expected_head_sha=expected_head_sha,
    )
    if release_intent is not None:
        pr_head_sha = str(release_intent["pr_head_sha"])
        return (
            str(release_intent["type_label"]),
            str(release_intent["channel_label"]),
            "pr-intent-artifact",
            pr_head_sha,
        )

    head = pr.get("head") or {}
    pr_head_sha = head.get("sha") if isinstance(head, dict) else None
    if not isinstance(pr_head_sha, str) or not re.fullmatch(r"[0-9a-f]{40}", pr_head_sha):
        if expected_head_sha is None:
            raise SnapshotError(f"Pull request #{pr_number} is missing a valid head.sha")
        pr_head_sha = expected_head_sha
    if legacy_fallback_allowed_for_target(api_root, repository, token, pr, target_sha=target_sha):
        labels = (
            current_pr_labels(pr)
            if allow_current_pr_label_fallback
            else labels_at_merge_time(api_root, repository, token, pr)
        )
        type_label, channel_label = parse_release_labels(labels)
        return (type_label, channel_label, "legacy-pr-labels", pr_head_sha)

    raise SnapshotError(
        f"Missing pre-frozen release intent artifact {RELEASE_INTENT_ARTIFACT_PREFIX}{pr_number}-* for PR #{pr_number}; "
        "legacy label fallback is only allowed when the target commit's previous mainline parent predates artifact-capable label-gate support"
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
    allow_current_pr_label_fallback: bool = False,
) -> dict[str, Any]:
    if pr is None:
        pr = load_pr_for_commit(api_root, repository, token, target_sha)
    if pr is None:
        raise SnapshotError(f"Commit {target_sha} is not associated with a merged pull request")
    type_label, channel_label, snapshot_source, pr_head_sha = resolve_release_intent_for_pr(
        api_root,
        repository,
        token,
        pr,
        target_sha=target_sha,
        allow_current_pr_label_fallback=allow_current_pr_label_fallback,
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
                    allow_current_pr_label_fallback=args.allow_current_pr_label_fallback,
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
