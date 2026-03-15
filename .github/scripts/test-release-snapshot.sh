#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 - <<'PY' "$repo_root/.github/scripts/release_snapshot.py"
from __future__ import annotations

import importlib.util
import argparse
import json
import io
import os
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

script_path = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("release_snapshot", script_path)
module = importlib.util.module_from_spec(spec)
assert spec is not None and spec.loader is not None
sys.modules[spec.name] = module
spec.loader.exec_module(module)
real_load_release_intent_artifact = module.load_release_intent_artifact
real_legacy_fallback_allowed_for_target = module.legacy_fallback_allowed_for_target


def run(*args: str, cwd: Path) -> str:
    result = subprocess.run(["git", *args], cwd=cwd, check=True, text=True, capture_output=True)
    return result.stdout.strip()


def make_pr(number: int, title: str, head_sha: str, *, merged_at: str = "2026-03-14T00:00:00Z") -> dict[str, object]:
    return {
        "number": number,
        "title": title,
        "merged_at": merged_at,
        "head": {"sha": head_sha},
    }


def make_release_intent(
    pr_number: int,
    head_sha: str,
    *,
    type_label: str = "type:patch",
    channel_label: str = "channel:stable",
) -> dict[str, object]:
    return {
        "schema_version": module.RELEASE_INTENT_SCHEMA_VERSION,
        "pr_number": pr_number,
        "pr_head_sha": head_sha,
        "type_label": type_label,
        "channel_label": channel_label,
        "created_at": "2026-03-14T00:00:00Z",
    }


with tempfile.TemporaryDirectory(prefix="release-snapshot-") as tmp:
    repo = Path(tmp)
    run("init", cwd=repo)
    run("config", "user.name", "Test User", cwd=repo)
    run("config", "user.email", "test@example.com", cwd=repo)
    (repo / "Cargo.toml").write_text('[package]\nname = "demo"\nversion = "0.1.0"\n')
    (repo / "README.md").write_text("base\n")
    run("add", "Cargo.toml", "README.md", cwd=repo)
    run("commit", "-m", "base", cwd=repo)
    run("tag", "v0.1.0", cwd=repo)

    (repo / "README.md").write_text("one\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "one", cwd=repo)
    sha1 = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("two\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "two", cwd=repo)
    sha2 = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("three\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "three", cwd=repo)
    sha3 = run("rev-parse", "HEAD", cwd=repo)

    original_cwd = Path.cwd()
    os.chdir(repo)

    try:
        module.load_pr_for_commit = lambda api_root, repository, token, target_sha, **kwargs: make_pr(
            131, f"Release {target_sha[:7]}", target_sha
        )
        module.load_release_intent_artifact = (
            lambda api_root, repository, token, pr_number, **kwargs: make_release_intent(pr_number, "1" * 40)
        )
        module.labels_at_merge_time = lambda api_root, repository, token, pr: []
        module.legacy_fallback_allowed_for_target = lambda *args, **kwargs: False
        snapshot1 = module.build_snapshot(
            target_sha=sha1,
            repository="IvanLi-CN/codex-vibe-monitor",
            token="token",
            notes_ref=module.DEFAULT_NOTES_REF,
            registry="ghcr.io",
            api_root="https://api.github.com",
        )
        assert snapshot1["next_stable_version"] == "0.1.1"
        assert snapshot1["snapshot_source"] == "pr-intent-artifact"
        assert snapshot1["pr_head_sha"] == "1" * 40
        run("notes", f"--ref={module.DEFAULT_NOTES_REF}", "add", "-f", "-m", json.dumps(snapshot1), sha1, cwd=repo)

        snapshot2 = module.build_snapshot(
            target_sha=sha2,
            repository="IvanLi-CN/codex-vibe-monitor",
            token="token",
            notes_ref=module.DEFAULT_NOTES_REF,
            registry="ghcr.io",
            api_root="https://api.github.com",
        )
        assert snapshot2["next_stable_version"] == "0.1.2"
        run("notes", f"--ref={module.DEFAULT_NOTES_REF}", "add", "-f", "-m", json.dumps(snapshot2), sha2, cwd=repo)

        assert module.publication_tags(snapshot1, notes_ref=module.DEFAULT_NOTES_REF, main_ref=sha3) == (
            "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.1"
        )
        assert module.publication_tags(snapshot2, notes_ref=module.DEFAULT_NOTES_REF, main_ref=sha3) == (
            "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.2,ghcr.io/ivanli-cn/codex-vibe-monitor:latest"
        )

        module.load_release_intent_artifact = (
            lambda api_root, repository, token, pr_number, **kwargs: make_release_intent(
                pr_number,
                "2" * 40,
                type_label="type:patch",
                channel_label="channel:rc",
            )
        )
        snapshot3 = module.build_snapshot(
            target_sha=sha3,
            repository="IvanLi-CN/codex-vibe-monitor",
            token="token",
            notes_ref=module.DEFAULT_NOTES_REF,
            registry="ghcr.io",
            api_root="https://api.github.com",
        )
        assert snapshot3["next_stable_version"] == "0.1.3"
        assert snapshot3["app_effective_version"] == f"0.1.3-rc.{sha3[:7]}"
        assert module.publication_tags(snapshot3, notes_ref=module.DEFAULT_NOTES_REF, main_ref=sha3) == (
            f"ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.3-rc.{sha3[:7]}"
        )

        read_back = module.read_snapshot(module.DEFAULT_NOTES_REF, sha2)
        assert read_back is not None
        assert read_back["release_tag"] == "v0.1.2"
        legacy_note = dict(snapshot1)
        legacy_note.pop("snapshot_source", None)
        restored_legacy_note = module.validate_snapshot(legacy_note, expected_sha=sha1)
        assert restored_legacy_note["snapshot_source"] == "ci-main"

        module.load_pr_for_commit = lambda api_root, repository, token, target_sha, **kwargs: make_pr(
            130, "Historical stable release", target_sha
        )
        module.load_release_intent_artifact = lambda api_root, repository, token, pr_number, **kwargs: None
        module.current_pr_labels = lambda pr: ["type:patch", "channel:stable"]
        module.labels_at_merge_time = lambda api_root, repository, token, pr: []
        module.legacy_fallback_allowed_for_target = lambda *args, **kwargs: True
        legacy_snapshot = module.build_snapshot(
            target_sha=sha1,
            repository="IvanLi-CN/codex-vibe-monitor",
            token="token",
            notes_ref=module.DEFAULT_NOTES_REF,
            registry="ghcr.io",
            api_root="https://api.github.com",
            allow_current_pr_label_fallback=True,
        )
        assert legacy_snapshot["snapshot_source"] == "legacy-pr-labels"

        module.load_pr_for_commit = lambda api_root, repository, token, target_sha, **kwargs: make_pr(
            140, "Future release without artifact", target_sha, merged_at="2026-03-16T00:00:01Z"
        )
        module.load_release_intent_artifact = lambda api_root, repository, token, pr_number, **kwargs: None
        module.legacy_fallback_allowed_for_target = lambda *args, **kwargs: False
        try:
            module.build_snapshot(
                target_sha=sha2,
                repository="IvanLi-CN/codex-vibe-monitor",
                token="token",
                notes_ref=module.DEFAULT_NOTES_REF,
                registry="ghcr.io",
                api_root="https://api.github.com",
            )
        except module.SnapshotError as exc:
            assert "Missing pre-frozen release intent artifact" in str(exc)
        else:
            raise AssertionError("expected future snapshot without artifact to fail closed")
    finally:
        os.chdir(original_cwd)

with tempfile.TemporaryDirectory(prefix="release-snapshot-race-") as tmp:
    tmp_root = Path(tmp)
    origin = tmp_root / "origin.git"
    seed = tmp_root / "seed"
    worker_a = tmp_root / "worker-a"
    worker_b = tmp_root / "worker-b"

    run("init", "--bare", str(origin), cwd=tmp_root)
    run("clone", str(origin), str(seed), cwd=tmp_root)
    run("config", "user.name", "Test User", cwd=seed)
    run("config", "user.email", "test@example.com", cwd=seed)
    run("switch", "-c", "main", cwd=seed)
    (seed / "Cargo.toml").write_text('[package]\nname = "demo"\nversion = "0.1.0"\n')
    (seed / "README.md").write_text("base\n")
    run("add", "Cargo.toml", "README.md", cwd=seed)
    run("commit", "-m", "base", cwd=seed)
    run("tag", "v0.1.0", cwd=seed)

    (seed / "README.md").write_text("one\n")
    run("add", "README.md", cwd=seed)
    run("commit", "-m", "one", cwd=seed)
    race_sha1 = run("rev-parse", "HEAD", cwd=seed)

    (seed / "README.md").write_text("two\n")
    run("add", "README.md", cwd=seed)
    run("commit", "-m", "two", cwd=seed)
    race_sha2 = run("rev-parse", "HEAD", cwd=seed)

    run("push", "-u", "origin", "main", "--tags", cwd=seed)
    run("symbolic-ref", "HEAD", "refs/heads/main", cwd=origin)

    run("clone", str(origin), str(worker_a), cwd=tmp_root)
    run("clone", str(origin), str(worker_b), cwd=tmp_root)
    for clone in (worker_a, worker_b):
        run("config", "user.name", "Test User", cwd=clone)
        run("config", "user.email", "test@example.com", cwd=clone)

    prs = {
        race_sha1: make_pr(201, "Stable one", race_sha1),
        race_sha2: make_pr(202, "Stable two", race_sha2),
    }
    module.load_pr_for_commit = (
        lambda api_root, repository, token, target_sha, **kwargs: prs.get(target_sha)
        if kwargs.get("allow_zero")
        else prs[target_sha]
    )
    module.load_release_intent_artifact = (
        lambda api_root, repository, token, pr_number, **kwargs: make_release_intent(pr_number, "3" * 40)
    )
    module.labels_at_merge_time = lambda api_root, repository, token, pr: []

    snapshot_a_path = worker_a / "snapshot-a.json"
    snapshot_b_path = worker_b / "snapshot-b.json"

    old_cwd = Path.cwd()
    try:
        os.chdir(worker_b)
        exit_code = module.ensure_snapshot(
            argparse.Namespace(
                target_sha=race_sha2,
                github_repository="IvanLi-CN/codex-vibe-monitor",
                github_token="token",
                notes_ref=module.DEFAULT_NOTES_REF,
                registry="ghcr.io",
                api_root="https://api.github.com",
                output=str(snapshot_b_path),
                max_attempts=3,
                allow_current_pr_label_fallback=False,
                target_only=False,
            )
        )
        assert exit_code == 0
        module.fetch_notes_ref(module.DEFAULT_NOTES_REF)
        snap_a = module.read_snapshot(module.DEFAULT_NOTES_REF, race_sha1)
        snap_b = module.read_snapshot(module.DEFAULT_NOTES_REF, race_sha2)
        assert snap_a is not None
        assert snap_a["next_stable_version"] == "0.1.1"
        assert snap_b is not None
        assert snap_b["next_stable_version"] == "0.1.2"
    finally:
        os.chdir(old_cwd)

    run("push", "origin", f":{module.DEFAULT_NOTES_REF}", cwd=seed)
    for clone in (worker_a, worker_b):
        subprocess.run(["git", "update-ref", "-d", module.DEFAULT_NOTES_REF], cwd=clone, check=False)

    real_git = module.git
    injected = {"done": False}

    def git_with_race(*args: str, **kwargs: object):
        if args == ("push", "origin", module.DEFAULT_NOTES_REF) and not injected["done"]:
            old_cwd = Path.cwd()
            try:
                os.chdir(worker_a)
                snapshot_a = module.build_snapshot(
                    target_sha=race_sha1,
                    repository="IvanLi-CN/codex-vibe-monitor",
                    token="token",
                    notes_ref=module.DEFAULT_NOTES_REF,
                    registry="ghcr.io",
                    api_root="https://api.github.com",
                )
                module.write_json(snapshot_a_path, snapshot_a)
                real_git("notes", f"--ref={module.DEFAULT_NOTES_REF}", "add", "-f", "-F", str(snapshot_a_path), race_sha1)
                real_git("push", "origin", module.DEFAULT_NOTES_REF)
            finally:
                os.chdir(old_cwd)
            injected["done"] = True
        return real_git(*args, **kwargs)

    try:
        module.git = git_with_race
        os.chdir(worker_b)
        exit_code = module.ensure_snapshot(
            argparse.Namespace(
                target_sha=race_sha2,
                github_repository="IvanLi-CN/codex-vibe-monitor",
                github_token="token",
                notes_ref=module.DEFAULT_NOTES_REF,
                registry="ghcr.io",
                api_root="https://api.github.com",
                output=str(snapshot_b_path),
                max_attempts=3,
                allow_current_pr_label_fallback=False,
                target_only=False,
            )
        )
        assert exit_code == 0
        module.fetch_notes_ref(module.DEFAULT_NOTES_REF)
        snap_b = module.read_snapshot(module.DEFAULT_NOTES_REF, race_sha2)
        assert snap_b is not None
        assert snap_b["next_stable_version"] == "0.1.2"
        snap_a = module.read_snapshot(module.DEFAULT_NOTES_REF, race_sha1)
        assert snap_a is not None
        assert snap_a["next_stable_version"] == "0.1.1"
        assert json.loads(snapshot_b_path.read_text())["next_stable_version"] == "0.1.2"
        assert injected["done"] is True
    finally:
        module.git = real_git
        os.chdir(old_cwd)

with tempfile.TemporaryDirectory(prefix="release-snapshot-cargo-version-") as tmp:
    repo = Path(tmp)
    run("init", cwd=repo)
    run("config", "user.name", "Test User", cwd=repo)
    run("config", "user.email", "test@example.com", cwd=repo)
    (repo / "Cargo.toml").write_text('[package]\nname = "demo"\nversion = "0.1.0"\n')
    (repo / "README.md").write_text("base\n")
    run("add", "Cargo.toml", "README.md", cwd=repo)
    run("commit", "-m", "base", cwd=repo)
    old_sha = run("rev-parse", "HEAD", cwd=repo)

    (repo / "Cargo.toml").write_text('[package]\nname = "demo"\nversion = "0.2.0"\n')
    (repo / "README.md").write_text("next\n")
    run("add", "Cargo.toml", "README.md", cwd=repo)
    run("commit", "-m", "next", cwd=repo)

    original_cwd = Path.cwd()
    os.chdir(repo)
    try:
        module.load_pr_for_commit = lambda api_root, repository, token, target_sha, **kwargs: make_pr(
            301, "Initial stable release", target_sha
        )
        module.load_release_intent_artifact = (
            lambda api_root, repository, token, pr_number, **kwargs: make_release_intent(pr_number, "4" * 40)
        )
        snapshot = module.build_snapshot(
            target_sha=old_sha,
            repository="IvanLi-CN/codex-vibe-monitor",
            token="token",
            notes_ref=module.DEFAULT_NOTES_REF,
            registry="ghcr.io",
            api_root="https://api.github.com",
        )
        assert snapshot["base_stable_version"] == "0.1.0"
        assert snapshot["next_stable_version"] == "0.1.1"
    finally:
        os.chdir(original_cwd)

with tempfile.TemporaryDirectory(prefix="release-intent-support-") as tmp:
    repo = Path(tmp)
    source_repo = script_path.parents[2]
    run("init", cwd=repo)
    run("config", "user.name", "Test User", cwd=repo)
    run("config", "user.email", "test@example.com", cwd=repo)

    support_paths = module.RELEASE_INTENT_SUPPORT_PATHS
    for relative in support_paths:
        destination = repo / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text((source_repo / relative).read_text())

    broken_metadata = repo / ".github/scripts/metadata_gate.py"
    text = broken_metadata.read_text()
    broken_arg = '    parser.add_argument("--write-intent", default="")\n'
    assert broken_arg in text
    broken_metadata.write_text(text.replace(broken_arg, "", 1))
    run("add", ".github", cwd=repo)
    run("commit", "-m", "broken rollout support", cwd=repo)
    broken_sha = run("rev-parse", "HEAD", cwd=repo)

    broken_metadata.write_text((source_repo / ".github/scripts/metadata_gate.py").read_text())
    run("add", ".github", cwd=repo)
    run("commit", "-m", "full rollout support", cwd=repo)
    full_sha = run("rev-parse", "HEAD", cwd=repo)

    original_cwd = Path.cwd()
    os.chdir(repo)
    try:
        assert module.commit_supports_release_intent_artifact(broken_sha) is False
        assert module.commit_supports_release_intent_artifact(full_sha) is True
    finally:
        os.chdir(original_cwd)

real_support_rollout_moment_for_target = module.support_rollout_moment_for_target
real_pr_had_rollout_trigger_after = module.pr_had_rollout_trigger_after
try:
    module.legacy_fallback_allowed_for_target = real_legacy_fallback_allowed_for_target
    module.support_rollout_moment_for_target = (
        lambda target_sha: module.parse_github_timestamp("2026-03-15T00:00:00Z", where="test rollout")
    )
    module.pr_had_rollout_trigger_after = lambda api_root, repository, token, pr, rollout_moment: False
    assert module.legacy_fallback_allowed_for_target(
        "https://api.github.com",
        "IvanLi-CN/codex-vibe-monitor",
        "token",
        make_pr(150, "Old PR without rerun", "5" * 40),
        target_sha="6" * 40,
    ) is True
    module.pr_had_rollout_trigger_after = lambda api_root, repository, token, pr, rollout_moment: True
    assert module.legacy_fallback_allowed_for_target(
        "https://api.github.com",
        "IvanLi-CN/codex-vibe-monitor",
        "token",
        make_pr(151, "New PR with rerun", "7" * 40),
        target_sha="8" * 40,
    ) is False
finally:
    module.support_rollout_moment_for_target = real_support_rollout_moment_for_target
    module.pr_had_rollout_trigger_after = real_pr_had_rollout_trigger_after
    module.legacy_fallback_allowed_for_target = real_legacy_fallback_allowed_for_target

artifact_payloads = {
    "https://example.test/artifacts/1/zip": make_release_intent(140, "a" * 40),
    "https://example.test/artifacts/2/zip": make_release_intent(140, "b" * 40),
    "https://example.test/artifacts/3/zip": make_release_intent(140, "c" * 40),
}
artifact_bytes = {}
for url, artifact_payload in artifact_payloads.items():
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        archive.writestr("release-intent.json", json.dumps(artifact_payload))
    artifact_bytes[url] = buffer.getvalue()

real_request_json = module.github_request_json
real_request_bytes = module.github_request_bytes
try:
    module.load_release_intent_artifact = real_load_release_intent_artifact
    def fake_request_json(api_root, token, path, query=None):
        if path.endswith("/actions/artifacts"):
            return {
                "artifacts": [
                    {
                        "name": module.artifact_name_for_pr(140, "c" * 40),
                        "expired": False,
                        "created_at": "2026-03-15T00:00:01Z",
                        "archive_download_url": "https://example.test/artifacts/3/zip",
                        "workflow_run": {"id": 3},
                    },
                    {
                        "name": module.artifact_name_for_pr(140, "b" * 40),
                        "expired": False,
                        "created_at": "2026-03-15T00:00:00Z",
                        "archive_download_url": "https://example.test/artifacts/2/zip",
                        "workflow_run": {"id": 2},
                    },
                    {
                        "name": module.artifact_name_for_pr(140, "a" * 40),
                        "expired": False,
                        "created_at": "2026-03-14T23:59:59Z",
                        "archive_download_url": "https://example.test/artifacts/1/zip",
                        "workflow_run": {"id": 1},
                    },
                ]
            }
        if path.endswith("/actions/runs/1"):
            return {
                "path": module.TRUSTED_RELEASE_INTENT_WORKFLOW_PATH,
                "event": module.TRUSTED_RELEASE_INTENT_EVENT,
                "status": "completed",
                "conclusion": "success",
                "head_sha": "a" * 40,
            }
        if path.endswith("/actions/runs/2"):
            return {
                "path": module.TRUSTED_RELEASE_INTENT_WORKFLOW_PATH,
                "event": "pull_request",
                "status": "completed",
                "conclusion": "success",
                "pull_requests": [{"number": 140, "head": {"sha": "b" * 40}}],
            }
        if path.endswith("/actions/runs/3"):
            return {
                "path": module.TRUSTED_RELEASE_INTENT_WORKFLOW_PATH,
                "event": "pull_request",
                "status": "completed",
                "conclusion": "cancelled",
                "pull_requests": [{"number": 140, "head": {"sha": "c" * 40}}],
            }
        raise AssertionError(f"unexpected path: {path}")

    module.github_request_json = fake_request_json
    module.github_request_bytes = lambda url, token: artifact_bytes[url]
    loaded_intent = module.load_release_intent_artifact(
        "https://api.github.com",
        "IvanLi-CN/codex-vibe-monitor",
        "token",
        140,
        merged_at="2026-03-15T00:00:02Z",
        expected_head_sha="a" * 40,
    )
    assert loaded_intent is not None
    assert loaded_intent["type_label"] == "type:patch"
    assert loaded_intent["pr_head_sha"] == "a" * 40
finally:
    module.github_request_json = real_request_json
    module.github_request_bytes = real_request_bytes
    module.load_release_intent_artifact = real_load_release_intent_artifact
    module.legacy_fallback_allowed_for_target = real_legacy_fallback_allowed_for_target

real_merged_pr_head_sha = module.merged_pr_head_sha
real_load_release_intent_artifact = module.load_release_intent_artifact
try:
    captured_expected_head_sha = {"value": None}

    def fake_load_release_intent_artifact(api_root, repository, token, pr_number, *, merged_at, expected_head_sha=None):
        captured_expected_head_sha["value"] = expected_head_sha
        return make_release_intent(pr_number, "d" * 40, type_label="type:skip")

    module.merged_pr_head_sha = lambda target_sha: None
    module.load_release_intent_artifact = fake_load_release_intent_artifact

    type_label, channel_label, snapshot_source, pr_head_sha = module.resolve_release_intent_for_pr(
        "https://api.github.com",
        "IvanLi-CN/codex-vibe-monitor",
        "token",
        make_pr(141, "Squash merged PR", "d" * 40),
        target_sha="e" * 40,
    )
    assert captured_expected_head_sha["value"] == "d" * 40
    assert type_label == "type:skip"
    assert channel_label == "channel:stable"
    assert snapshot_source == "pr-intent-artifact"
    assert pr_head_sha == "d" * 40
finally:
    module.merged_pr_head_sha = real_merged_pr_head_sha
    module.load_release_intent_artifact = real_load_release_intent_artifact

with tempfile.TemporaryDirectory(prefix="release-snapshot-target-only-") as tmp:
    repo = Path(tmp)
    run("init", cwd=repo)
    run("config", "user.name", "Test User", cwd=repo)
    run("config", "user.email", "test@example.com", cwd=repo)
    run("checkout", "-b", "main", cwd=repo)
    (repo / "Cargo.toml").write_text('[package]\nname = "demo"\nversion = "0.1.0"\n')
    (repo / "README.md").write_text("base\n")
    run("add", "Cargo.toml", "README.md", cwd=repo)
    run("commit", "-m", "base", cwd=repo)
    run("tag", "v0.1.0", cwd=repo)

    (repo / "README.md").write_text("old merge\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "old merge", cwd=repo)
    old_sha = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("target merge\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "target merge", cwd=repo)
    target_sha = run("rev-parse", "HEAD", cwd=repo)

    original_cwd = Path.cwd()
    original_load_pr = module.load_pr_for_commit
    original_build_snapshot = module.build_snapshot
    original_git = module.git
    calls = []

    def fake_build_snapshot(*, target_sha: str, **kwargs: object):
        calls.append(target_sha)
        if target_sha == old_sha:
            raise AssertionError("manual backfill should not materialize older missing snapshots")
        return {
            "schema_version": module.SNAPSHOT_SCHEMA_VERSION,
            "target_sha": target_sha,
            "pr_number": 402,
            "pr_title": "Target labeled merge",
            "registry": "ghcr.io",
            "pr_head_sha": "6" * 40,
            "type_label": "type:patch",
            "channel_label": "channel:stable",
            "release_bump": "patch",
            "release_channel": "stable",
            "release_enabled": True,
            "release_prerelease": False,
            "image_name_lower": "ivanli-cn/codex-vibe-monitor",
            "base_stable_version": "0.1.0",
            "next_stable_version": "0.1.1",
            "app_effective_version": "0.1.1",
            "release_tag": "v0.1.1",
            "tags_csv": "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.1,ghcr.io/ivanli-cn/codex-vibe-monitor:latest",
            "notes_ref": module.DEFAULT_NOTES_REF,
            "snapshot_source": "legacy-pr-labels",
            "created_at": "2026-03-15T00:00:00Z",
        }

    os.chdir(repo)
    try:
        def fake_git(*args: str, **kwargs: object):
            if args == ("push", "origin", module.DEFAULT_NOTES_REF):
                return subprocess.CompletedProcess(["git", *args], 0, "", "")
            return original_git(*args, **kwargs)

        module.load_pr_for_commit = (
            lambda api_root, repository, token, commit_sha, **kwargs: {
                old_sha: make_pr(401, "Old unlabeled merge", old_sha),
                target_sha: make_pr(402, "Target labeled merge", target_sha),
            }.get(commit_sha)
            if kwargs.get("allow_zero")
            else {
                old_sha: make_pr(401, "Old unlabeled merge", old_sha),
                target_sha: make_pr(402, "Target labeled merge", target_sha),
            }[commit_sha]
        )
        module.build_snapshot = fake_build_snapshot
        module.git = fake_git
        exit_code = module.ensure_snapshot(
            argparse.Namespace(
                target_sha=target_sha,
                github_repository="IvanLi-CN/codex-vibe-monitor",
                github_token="token",
                notes_ref=module.DEFAULT_NOTES_REF,
                registry="ghcr.io",
                api_root="https://api.github.com",
                output=str(repo / "target-only.json"),
                max_attempts=1,
                allow_current_pr_label_fallback=True,
                target_only=True,
            )
        )
        assert exit_code == 0
        assert calls == [target_sha]
        assert module.read_snapshot(module.DEFAULT_NOTES_REF, old_sha) is None
        stored = module.read_snapshot(module.DEFAULT_NOTES_REF, target_sha)
        assert stored is not None
        assert stored["release_tag"] == "v0.1.1"
    finally:
        module.load_pr_for_commit = original_load_pr
        module.build_snapshot = original_build_snapshot
        module.git = original_git
        os.chdir(original_cwd)

print("test-release-snapshot: all checks passed")
PY
