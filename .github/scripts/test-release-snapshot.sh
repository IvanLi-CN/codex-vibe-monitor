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


def run(*args: str, cwd: Path) -> str:
    result = subprocess.run(["git", *args], cwd=cwd, check=True, text=True, capture_output=True)
    return result.stdout.strip()


def make_pr(number: int, title: str, head_sha: str) -> dict[str, object]:
    return {
        "number": number,
        "title": title,
        "merged_at": "2026-03-14T00:00:00Z",
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
            lambda api_root, repository, token, pr_number, pr_head_sha: make_release_intent(pr_number, pr_head_sha)
        )
        module.current_labels_for_pr = lambda api_root, repository, token, pr_number: []
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
            lambda api_root, repository, token, pr_number, pr_head_sha: make_release_intent(
                pr_number,
                pr_head_sha,
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

        module.load_pr_for_commit = lambda api_root, repository, token, target_sha, **kwargs: make_pr(
            130, "Historical stable release", target_sha
        )
        module.load_release_intent_artifact = lambda api_root, repository, token, pr_number, pr_head_sha: None
        module.current_labels_for_pr = (
            lambda api_root, repository, token, pr_number: ["type:patch", "channel:stable"]
        )
        legacy_snapshot = module.build_snapshot(
            target_sha=sha1,
            repository="IvanLi-CN/codex-vibe-monitor",
            token="token",
            notes_ref=module.DEFAULT_NOTES_REF,
            registry="ghcr.io",
            api_root="https://api.github.com",
        )
        assert legacy_snapshot["snapshot_source"] == "legacy-pr-labels"

        module.load_pr_for_commit = lambda api_root, repository, token, target_sha, **kwargs: make_pr(
            140, "Future release without artifact", target_sha
        )
        module.load_release_intent_artifact = lambda api_root, repository, token, pr_number, pr_head_sha: None
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
        lambda api_root, repository, token, pr_number, pr_head_sha: make_release_intent(pr_number, pr_head_sha)
    )
    module.current_labels_for_pr = lambda api_root, repository, token, pr_number: []

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
            lambda api_root, repository, token, pr_number, pr_head_sha: make_release_intent(pr_number, pr_head_sha)
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

payload = make_release_intent(140, "a" * 40)
buffer = io.BytesIO()
with zipfile.ZipFile(buffer, "w") as archive:
    archive.writestr("release-intent.json", json.dumps(payload))

artifact_bytes = buffer.getvalue()
real_request_json = module.github_request_json
real_request_bytes = module.github_request_bytes
try:
    def fake_request_json(api_root, token, path, query=None):
        if path.endswith("/actions/artifacts"):
            return {
                "artifacts": [
                    {
                        "name": module.artifact_name_for_pr(140, "a" * 40),
                        "expired": False,
                        "created_at": "2026-03-15T00:00:01Z",
                        "archive_download_url": "https://example.test/artifacts/2/zip",
                        "workflow_run": {"id": 2},
                    },
                    {
                        "name": module.artifact_name_for_pr(140, "a" * 40),
                        "expired": False,
                        "created_at": "2026-03-15T00:00:00Z",
                        "archive_download_url": "https://example.test/artifacts/1/zip",
                        "workflow_run": {"id": 1},
                    },
                ]
            }
        if path.endswith("/actions/runs/1"):
            return {
                "path": module.TRUSTED_RELEASE_INTENT_WORKFLOW_PATH,
                "event": module.TRUSTED_RELEASE_INTENT_EVENT,
                "head_sha": "a" * 40,
                "pull_requests": [{"number": 140, "head": {"sha": "a" * 40}}],
            }
        if path.endswith("/actions/runs/2"):
            return {
                "path": ".github/workflows/ci-pr.yml",
                "event": "pull_request",
                "head_sha": "a" * 40,
                "pull_requests": [{"number": 140, "head": {"sha": "a" * 40}}],
            }
        raise AssertionError(f"unexpected path: {path}")

    module.github_request_json = fake_request_json
    module.github_request_bytes = lambda url, token: artifact_bytes
    loaded_intent = module.load_release_intent_artifact(
        "https://api.github.com",
        "IvanLi-CN/codex-vibe-monitor",
        "token",
        140,
        "a" * 40,
    )
    assert loaded_intent is not None
    assert loaded_intent["type_label"] == "type:patch"
finally:
    module.github_request_json = real_request_json
    module.github_request_bytes = real_request_bytes

print("test-release-snapshot: all checks passed")
PY
