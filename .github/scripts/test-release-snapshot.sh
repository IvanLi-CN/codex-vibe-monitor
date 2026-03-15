#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 - <<'PY' "$repo_root/.github/scripts/release_snapshot.py"
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
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


def make_pr(number: int, title: str, head_sha: str, labels: list[str]) -> dict[str, object]:
    return {
        "number": number,
        "title": title,
        "head": {"sha": head_sha},
        "labels": [{"name": label} for label in labels],
    }


with tempfile.TemporaryDirectory(prefix="release-snapshot-") as tmp:
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

    prs = {
        sha1: make_pr(101, "Patch release", sha1, ["type:patch", "channel:stable"]),
        sha2: make_pr(102, "Minor release", sha2, ["type:minor", "channel:stable"]),
        sha3: make_pr(103, "RC release", sha3, ["type:patch", "channel:rc"]),
    }

    original_cwd = Path.cwd()
    original_loader = module.load_pr_for_commit
    try:
        os.chdir(repo)
        module.load_pr_for_commit = lambda api_root, repository, token, target_sha, **kwargs: prs[target_sha]

        snapshot1 = module.build_snapshot(
            target_sha=sha1,
            repository="IvanLi-CN/codex-vibe-monitor",
            token="token",
            notes_ref=module.DEFAULT_NOTES_REF,
            registry="ghcr.io",
            api_root="https://api.github.com",
        )
        assert snapshot1["snapshot_source"] == "ci-main"
        assert snapshot1["next_stable_version"] == "0.1.1"
        assert snapshot1["release_tag"] == "v0.1.1"
        run("notes", f"--ref={module.DEFAULT_NOTES_REF}", "add", "-f", "-m", json.dumps(snapshot1), sha1, cwd=repo)

        snapshot2 = module.build_snapshot(
            target_sha=sha2,
            repository="IvanLi-CN/codex-vibe-monitor",
            token="token",
            notes_ref=module.DEFAULT_NOTES_REF,
            registry="ghcr.io",
            api_root="https://api.github.com",
        )
        assert snapshot2["base_stable_version"] == "0.1.1"
        assert snapshot2["next_stable_version"] == "0.2.0"
        run("notes", f"--ref={module.DEFAULT_NOTES_REF}", "add", "-f", "-m", json.dumps(snapshot2), sha2, cwd=repo)

        snapshot3 = module.build_snapshot(
            target_sha=sha3,
            repository="IvanLi-CN/codex-vibe-monitor",
            token="token",
            notes_ref=module.DEFAULT_NOTES_REF,
            registry="ghcr.io",
            api_root="https://api.github.com",
        )
        assert snapshot3["base_stable_version"] == "0.2.0"
        assert snapshot3["next_stable_version"] == "0.2.1"
        assert snapshot3["app_effective_version"] == f"0.2.1-rc.{sha3[:7]}"
        assert snapshot3["release_prerelease"] is True
        run("notes", f"--ref={module.DEFAULT_NOTES_REF}", "add", "-f", "-m", json.dumps(snapshot3), sha3, cwd=repo)

        assert module.publication_tags(snapshot1, notes_ref=module.DEFAULT_NOTES_REF, main_ref=sha3) == (
            "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.1"
        )
        assert module.publication_tags(snapshot2, notes_ref=module.DEFAULT_NOTES_REF, main_ref=sha3) == (
            "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.2.0,ghcr.io/ivanli-cn/codex-vibe-monitor:latest"
        )

        docs_snapshot = module.build_snapshot(
            target_sha=sha1,
            repository="IvanLi-CN/codex-vibe-monitor",
            token="token",
            notes_ref=module.DEFAULT_NOTES_REF,
            registry="ghcr.io",
            api_root="https://api.github.com",
            pr=make_pr(104, "Docs only", sha1, ["type:docs", "channel:stable"]),
        )
        assert docs_snapshot["release_enabled"] is False
        assert docs_snapshot["release_tag"] == ""

        skip_snapshot = module.build_snapshot(
            target_sha=sha1,
            repository="IvanLi-CN/codex-vibe-monitor",
            token="token",
            notes_ref=module.DEFAULT_NOTES_REF,
            registry="ghcr.io",
            api_root="https://api.github.com",
            pr=make_pr(105, "Skip release", sha1, ["type:skip", "channel:stable"]),
        )
        assert skip_snapshot["release_enabled"] is False
        assert skip_snapshot["release_tag"] == ""

        try:
            module.build_snapshot(
                target_sha=sha1,
                repository="IvanLi-CN/codex-vibe-monitor",
                token="token",
                notes_ref=module.DEFAULT_NOTES_REF,
                registry="ghcr.io",
                api_root="https://api.github.com",
                pr=make_pr(106, "Broken labels", sha1, ["type:patch", "type:minor", "channel:stable"]),
            )
        except module.SnapshotError as exc:
            assert "Expected exactly 1 type:* label" in str(exc)
        else:
            raise AssertionError("expected invalid type labels to fail")

        run("tag", "v0.1.1", sha1, cwd=repo)
        pending = module.pending_release_targets(module.DEFAULT_NOTES_REF, sha3)
        assert pending == [sha2, sha3], (pending, sha2, sha3)
        assert module.release_tag_points_to_target(snapshot1) is True
        assert module.release_tag_points_to_target(snapshot2) is False
    finally:
        module.load_pr_for_commit = original_loader
        os.chdir(original_cwd)

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
    calls: list[str] = []

    def fake_build_snapshot(*, target_sha: str, **kwargs: object):
        calls.append(target_sha)
        if target_sha == old_sha:
            raise AssertionError("target-only mode should not materialize older snapshots")
        return {
            "schema_version": module.SNAPSHOT_SCHEMA_VERSION,
            "target_sha": target_sha,
            "pr_number": 202,
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
            "snapshot_source": "ci-main",
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
                old_sha: make_pr(201, "Old merge", old_sha, ["type:patch", "channel:stable"]),
                target_sha: make_pr(202, "Target merge", target_sha, ["type:patch", "channel:stable"]),
            }.get(commit_sha)
            if kwargs.get("allow_zero")
            else {
                old_sha: make_pr(201, "Old merge", old_sha, ["type:patch", "channel:stable"]),
                target_sha: make_pr(202, "Target merge", target_sha, ["type:patch", "channel:stable"]),
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
