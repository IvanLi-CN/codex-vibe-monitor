#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 - <<'PY' "$repo_root/.github/scripts/release_snapshot.py"
from __future__ import annotations

import argparse
import importlib.util
import io
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


original_urlopen = module.request.urlopen
original_sleep = module.time.sleep
try:
    class FakeResponse:
        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb):
            return False

        def read(self):
            return b'{"ok": true}'

    def flaky_urlopen(req):
        nonlocal_attempts[0] += 1
        if nonlocal_attempts[0] == 1:
            raise module.error.HTTPError(req.full_url, 500, "server error", {}, io.BytesIO(b"temporary"))
        return FakeResponse()

    nonlocal_attempts = [0]
    module.request.urlopen = flaky_urlopen
    module.time.sleep = lambda seconds: None
    assert module.github_request_json("https://api.github.test", "token", "/repos/demo", max_attempts=2) == {"ok": True}
    assert nonlocal_attempts == [2]

    nonlocal_attempts = [0]

    def forbidden_urlopen(req):
        nonlocal_attempts[0] += 1
        raise module.error.HTTPError(req.full_url, 403, "forbidden", {}, io.BytesIO(b"denied"))

    module.request.urlopen = forbidden_urlopen
    try:
        module.github_request_json("https://api.github.test", "token", "/repos/demo", max_attempts=2)
    except module.SnapshotError as exc:
        assert "403" in str(exc)
    else:
        raise AssertionError("expected non-retryable GitHub API failure")
    assert nonlocal_attempts == [1]
finally:
    module.request.urlopen = original_urlopen
    module.time.sleep = original_sleep


original_github_request_json = module.github_request_json
try:
    calls: list[tuple[str, tuple[tuple[str, object], ...] | None]] = []

    def fake_github_request_json(api_root, token, path, query=None, *, max_attempts=4):
        calls.append((path, tuple(sorted((query or {}).items())) if query else None))
        if "/commits/" in path:
            raise AssertionError(f"unexpected commit-associated PR lookup: {path}")
        if path.endswith("/pulls"):
            return [
                {
                    "number": 699,
                    "title": "Closed but unmerged",
                    "merged_at": None,
                    "merge_commit_sha": "f" * 40,
                    "head": {"sha": "1" * 40},
                    "labels": [{"name": "type:patch"}, {"name": "channel:stable"}],
                },
                {
                    "number": 700,
                    "title": "Merged via squash",
                    "merged_at": "2026-03-15T00:00:00Z",
                    "merge_commit_sha": "a" * 40,
                    "head": {"sha": "2" * 40},
                    "labels": [{"name": "type:minor"}, {"name": "channel:stable"}],
                },
                {
                    "number": 701,
                    "title": "Different merged PR",
                    "merged_at": "2026-03-15T00:00:00Z",
                    "merge_commit_sha": "b" * 40,
                    "head": {"sha": "3" * 40},
                    "labels": [{"name": "type:patch"}, {"name": "channel:rc"}],
                },
            ]
        if path.endswith("/pulls/700"):
            return {
                "number": 700,
                "title": "Merged via squash",
                "head": {"sha": "2" * 40},
                "labels": [{"name": "type:minor"}, {"name": "channel:stable"}],
            }
        raise AssertionError(f"unexpected GitHub API path: {path}")

    module.github_request_json = fake_github_request_json
    merged_pr = module.load_pr_for_commit(
        "https://api.github.test",
        "IvanLi-CN/codex-vibe-monitor",
        "token",
        "a" * 40,
    )
    assert merged_pr is not None
    assert merged_pr["number"] == 700
    assert module.load_pr_for_commit(
        "https://api.github.test",
        "IvanLi-CN/codex-vibe-monitor",
        "token",
        "c" * 40,
        allow_zero=True,
    ) is None
    assert all("/commits/" not in path for path, _ in calls)
finally:
    module.github_request_json = original_github_request_json


original_github_request_json = module.github_request_json
try:
    def fake_github_request_json(api_root, token, path, query=None, *, max_attempts=4):
        head_sha = (query or {}).get("head_sha")
        if path.endswith("/actions/workflows/ci-main.yml/runs"):
            runs = {
                "a" * 40: [{"id": 10, "head_sha": "a" * 40, "conclusion": "success"}],
                "b" * 40: [{"id": 20, "head_sha": "b" * 40, "conclusion": "failure"}],
                "c" * 40: [{"id": 30, "head_sha": "c" * 40, "conclusion": "failure"}],
            }
            return {"workflow_runs": runs.get(head_sha, [])}
        if path.endswith("/actions/runs/20/jobs"):
            return {
                "jobs": [
                    {"name": "Release Snapshot", "conclusion": "failure"},
                    {"name": "Lint & Format Check", "conclusion": "success"},
                    {"name": "Backend Tests", "conclusion": "success"},
                ]
            }
        if path.endswith("/actions/runs/30/jobs"):
            return {
                "jobs": [
                    {"name": "Release Snapshot", "conclusion": "skipped"},
                    {"name": "Backend Tests", "conclusion": "failure"},
                ]
            }
        raise AssertionError(f"unexpected GitHub API path: {path}")

    module.github_request_json = fake_github_request_json
    assert module.ci_main_run_is_release_eligible(
        "https://api.github.test",
        "IvanLi-CN/codex-vibe-monitor",
        "token",
        "a" * 40,
    ) == "eligible"
    assert module.ci_main_run_is_release_eligible(
        "https://api.github.test",
        "IvanLi-CN/codex-vibe-monitor",
        "token",
        "b" * 40,
    ) == "eligible"
    assert module.ci_main_run_is_release_eligible(
        "https://api.github.test",
        "IvanLi-CN/codex-vibe-monitor",
        "token",
        "c" * 40,
    ) == "ineligible"
    assert module.ci_main_run_is_release_eligible(
        "https://api.github.test",
        "IvanLi-CN/codex-vibe-monitor",
        "token",
        "d" * 40,
    ) == "unknown"
finally:
    module.github_request_json = original_github_request_json


with tempfile.TemporaryDirectory(prefix="release-snapshot-merged-pr-") as tmp:
    repo = Path(tmp)
    run("init", cwd=repo)
    run("config", "user.name", "Test User", cwd=repo)
    run("config", "user.email", "test@example.com", cwd=repo)
    run("checkout", "-b", "main", cwd=repo)
    (repo / "Cargo.toml").write_text('[package]\nname = "demo"\nversion = "0.1.0"\n')
    (repo / "README.md").write_text("base\n")
    run("add", "Cargo.toml", "README.md", cwd=repo)
    run("commit", "-m", "base", cwd=repo)

    original_cwd = Path.cwd()
    original_loader = module.load_pr_for_commit
    try:
        os.chdir(repo)
        module.load_pr_for_commit = lambda api_root, repository, token, target_sha, **kwargs: make_pr(
            702, "Merged PR", target_sha, ["type:patch", "channel:stable"]
        )
        snapshot = module.build_snapshot(
            target_sha=run("rev-parse", "HEAD", cwd=repo),
            repository="IvanLi-CN/codex-vibe-monitor",
            token="token",
            notes_ref=module.DEFAULT_NOTES_REF,
            registry="ghcr.io",
            api_root="https://api.github.com",
            pr=make_pr(702, "Merged PR", "4" * 40, ["type:patch", "channel:stable"]),
            snapshot_source="merged-pr",
        )
        assert snapshot["snapshot_source"] == "merged-pr"
        assert snapshot["pr_number"] == 702
        assert snapshot["release_enabled"] is True
    finally:
        module.load_pr_for_commit = original_loader
        os.chdir(original_cwd)


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
        assert snapshot1["tags_csv"] == "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.1"
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
        assert snapshot2["tags_csv"] == "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.2.0"
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
        assert snapshot3["tags_csv"] == f"ghcr.io/ivanli-cn/codex-vibe-monitor:v0.2.1-rc.{sha3[:7]}"
        run("notes", f"--ref={module.DEFAULT_NOTES_REF}", "add", "-f", "-m", json.dumps(snapshot3), sha3, cwd=repo)

        assert module.publication_tags(snapshot1, notes_ref=module.DEFAULT_NOTES_REF, main_ref=sha3) == (
            "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.1,ghcr.io/ivanli-cn/codex-vibe-monitor:latest"
        )
        assert module.publication_tags(snapshot2, notes_ref=module.DEFAULT_NOTES_REF, main_ref=sha3) == (
            "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.2.0,ghcr.io/ivanli-cn/codex-vibe-monitor:latest"
        )
        assert module.publication_tags(snapshot3, notes_ref=module.DEFAULT_NOTES_REF, main_ref=sha3) == (
            f"ghcr.io/ivanli-cn/codex-vibe-monitor:v0.2.1-rc.{sha3[:7]}"
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
        filtered_pending = module.pending_release_targets(
            module.DEFAULT_NOTES_REF,
            sha3,
            is_release_eligible=lambda commit: "ineligible" if commit == sha2 else "eligible",
        )
        assert filtered_pending == [sha3], filtered_pending
        unknown_pending = module.pending_release_targets(
            module.DEFAULT_NOTES_REF,
            sha3,
            is_release_eligible=lambda commit: "unknown" if commit == sha2 else "eligible",
        )
        assert unknown_pending == [sha2, sha3], unknown_pending
        assert module.release_tag_points_to_target(snapshot1) is True
        assert module.release_tag_points_to_target(snapshot2) is False
        assert module.publication_tags(snapshot1, notes_ref=module.DEFAULT_NOTES_REF, main_ref=sha3) == (
            "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.1,ghcr.io/ivanli-cn/codex-vibe-monitor:latest"
        )

        original_ci_main_eligibility = module.ci_main_run_is_release_eligible
        original_fetch_notes_ref = module.fetch_notes_ref
        original_fetch_tags = module.fetch_tags
        try:
            module.ci_main_run_is_release_eligible = (
                lambda api_root, repository, token, target_sha: "ineligible" if target_sha == sha2 else "eligible"
            )
            module.fetch_notes_ref = lambda notes_ref: None
            module.fetch_tags = lambda: None
            github_output = repo / "next-pending.txt"
            exit_code = module.export_next_pending(
                argparse.Namespace(
                    notes_ref=module.DEFAULT_NOTES_REF,
                    main_ref=sha3,
                    upper_bound=sha3,
                    github_repository="IvanLi-CN/codex-vibe-monitor",
                    github_token="token",
                    api_root="https://api.github.test",
                    github_output=str(github_output),
                )
            )
            assert exit_code == 0
            assert github_output.read_text().strip() == f"target_sha={sha3}"
        finally:
            module.ci_main_run_is_release_eligible = original_ci_main_eligibility
            module.fetch_notes_ref = original_fetch_notes_ref
            module.fetch_tags = original_fetch_tags

        try:
            def flaky_ci_main_eligibility(api_root, repository, token, target_sha):
                if target_sha == sha2:
                    raise module.SnapshotError("workflow run not indexed yet")
                return "eligible"

            module.ci_main_run_is_release_eligible = flaky_ci_main_eligibility
            module.fetch_notes_ref = lambda notes_ref: None
            module.fetch_tags = lambda: None
            github_output = repo / "next-pending-unknown.txt"
            exit_code = module.export_next_pending(
                argparse.Namespace(
                    notes_ref=module.DEFAULT_NOTES_REF,
                    main_ref=sha3,
                    upper_bound=sha3,
                    github_repository="IvanLi-CN/codex-vibe-monitor",
                    github_token="token",
                    api_root="https://api.github.test",
                    github_output=str(github_output),
                )
            )
            assert exit_code == 0
            assert github_output.read_text().strip() == f"target_sha={sha2}"
        finally:
            module.ci_main_run_is_release_eligible = original_ci_main_eligibility
            module.fetch_notes_ref = original_fetch_notes_ref
            module.fetch_tags = original_fetch_tags

        run("tag", "v0.2.0", sha2, cwd=repo)
        assert module.release_tag_points_to_target(snapshot2) is True
        assert module.publication_tags(snapshot1, notes_ref=module.DEFAULT_NOTES_REF, main_ref=sha3) == (
            "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.1"
        )
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
        snapshot_sha = target_sha
        calls.append(snapshot_sha)
        if snapshot_sha == old_sha:
            raise AssertionError("target-only mode should not materialize older snapshots")
        return {
            "schema_version": module.SNAPSHOT_SCHEMA_VERSION,
            "target_sha": snapshot_sha,
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
            "tags_csv": "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.1",
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


with tempfile.TemporaryDirectory(prefix="release-snapshot-empty-notes-") as tmp:
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

    (repo / "README.md").write_text("patch one\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "patch one", cwd=repo)
    first_sha = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("patch two\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "patch two", cwd=repo)
    target_sha = run("rev-parse", "HEAD", cwd=repo)

    original_cwd = Path.cwd()
    original_load_pr = module.load_pr_for_commit
    original_git = module.git

    os.chdir(repo)
    try:
        def fake_git(*args: str, **kwargs: object):
            if args == ("push", "origin", module.DEFAULT_NOTES_REF):
                return subprocess.CompletedProcess(["git", *args], 0, "", "")
            return original_git(*args, **kwargs)

        def fake_load_pr(api_root, repository, token, commit_sha, **kwargs):
            return {
                first_sha: make_pr(401, "First patch", first_sha, ["type:patch", "channel:stable"]),
                target_sha: make_pr(402, "Second patch", target_sha, ["type:patch", "channel:stable"]),
            }.get(commit_sha)

        module.load_pr_for_commit = fake_load_pr
        module.git = fake_git
        exit_code = module.ensure_snapshot(
            argparse.Namespace(
                target_sha=target_sha,
                github_repository="IvanLi-CN/codex-vibe-monitor",
                github_token="token",
                notes_ref=module.DEFAULT_NOTES_REF,
                registry="ghcr.io",
                api_root="https://api.github.com",
                output=str(repo / "empty-notes.json"),
                max_attempts=1,
                target_only=False,
            )
        )
        assert exit_code == 0
        first_snapshot = module.read_snapshot(module.DEFAULT_NOTES_REF, first_sha)
        target_snapshot = module.read_snapshot(module.DEFAULT_NOTES_REF, target_sha)
        assert first_snapshot is not None
        assert first_snapshot["next_stable_version"] == "0.1.1"
        assert target_snapshot is not None
        assert target_snapshot["base_stable_version"] == "0.1.1"
        assert target_snapshot["next_stable_version"] == "0.1.2"
    finally:
        module.load_pr_for_commit = original_load_pr
        module.git = original_git
        os.chdir(original_cwd)


with tempfile.TemporaryDirectory(prefix="release-snapshot-catch-up-") as tmp:
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

    (repo / "README.md").write_text("legacy unlabeled\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "legacy unlabeled", cwd=repo)
    legacy_sha = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("existing snapshot\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "existing snapshot", cwd=repo)
    snap_sha = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("mid pending\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "mid pending", cwd=repo)
    mid_sha = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("target pending\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "target pending", cwd=repo)
    target_sha = run("rev-parse", "HEAD", cwd=repo)
    target_commit_sha = target_sha

    existing_snapshot = {
        "schema_version": module.SNAPSHOT_SCHEMA_VERSION,
        "target_sha": snap_sha,
        "pr_number": 301,
        "pr_title": "Existing snapshot",
        "registry": "ghcr.io",
        "pr_head_sha": snap_sha,
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
        "tags_csv": "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.1",
        "notes_ref": module.DEFAULT_NOTES_REF,
        "snapshot_source": "ci-main",
        "created_at": "2026-03-15T00:00:00Z",
    }
    run("notes", f"--ref={module.DEFAULT_NOTES_REF}", "add", "-f", "-m", json.dumps(existing_snapshot), snap_sha, cwd=repo)

    original_cwd = Path.cwd()
    original_load_pr = module.load_pr_for_commit
    original_build_snapshot = module.build_snapshot
    original_git = module.git
    calls: list[str] = []

    def fake_build_snapshot(*, target_sha: str, **kwargs: object):
        snapshot_sha = target_sha
        calls.append(snapshot_sha)
        version_map = {
            mid_sha: ("0.1.1", "0.1.2", "v0.1.2"),
            target_commit_sha: ("0.1.2", "0.1.3", "v0.1.3"),
        }
        if snapshot_sha not in version_map:
            raise AssertionError(f"unexpected snapshot build for {snapshot_sha}")
        base_version, next_version, release_tag = version_map[snapshot_sha]
        return {
            "schema_version": module.SNAPSHOT_SCHEMA_VERSION,
            "target_sha": snapshot_sha,
            "pr_number": 302 if snapshot_sha == mid_sha else 303,
            "pr_title": "Pending snapshot",
            "registry": "ghcr.io",
            "pr_head_sha": snapshot_sha,
            "type_label": "type:patch",
            "channel_label": "channel:stable",
            "release_bump": "patch",
            "release_channel": "stable",
            "release_enabled": True,
            "release_prerelease": False,
            "image_name_lower": "ivanli-cn/codex-vibe-monitor",
            "base_stable_version": base_version,
            "next_stable_version": next_version,
            "app_effective_version": next_version,
            "release_tag": release_tag,
            "tags_csv": f"ghcr.io/ivanli-cn/codex-vibe-monitor:{release_tag}",
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

        def fake_load_pr(api_root, repository, token, commit_sha, **kwargs):
            return {
                mid_sha: make_pr(302, "Mid pending", mid_sha, ["type:patch", "channel:stable"]),
                target_sha: make_pr(303, "Target pending", target_sha, ["type:patch", "channel:stable"]),
            }.get(commit_sha)

        module.load_pr_for_commit = fake_load_pr
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
                output=str(repo / "catch-up.json"),
                max_attempts=1,
                target_only=False,
            )
        )
        assert exit_code == 0
        assert calls == [mid_sha, target_sha]
        assert module.read_snapshot(module.DEFAULT_NOTES_REF, legacy_sha) is None
        assert module.read_snapshot(module.DEFAULT_NOTES_REF, snap_sha) is not None
        assert module.read_snapshot(module.DEFAULT_NOTES_REF, mid_sha) is not None
        stored = module.read_snapshot(module.DEFAULT_NOTES_REF, target_sha)
        assert stored is not None
        assert stored["release_tag"] == "v0.1.3"
    finally:
        module.load_pr_for_commit = original_load_pr
        module.build_snapshot = original_build_snapshot
        module.git = original_git
        os.chdir(original_cwd)

with tempfile.TemporaryDirectory(prefix="release-snapshot-sparse-history-") as tmp:
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

    (repo / "README.md").write_text("existing snapshot\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "existing snapshot", cwd=repo)
    existing_sha = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("missing snapshot\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "missing snapshot", cwd=repo)
    missing_sha = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("later snapshot\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "later snapshot", cwd=repo)
    later_sha = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("target pending\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "target pending", cwd=repo)
    target_sha = run("rev-parse", "HEAD", cwd=repo)
    target_commit_sha = target_sha

    original_cwd = Path.cwd()
    original_load_pr = module.load_pr_for_commit
    original_build_snapshot = module.build_snapshot
    original_git = module.git
    calls: list[str] = []

    def fake_build_snapshot(*, target_sha: str, **kwargs: object):
        snapshot_sha = target_sha
        calls.append(snapshot_sha)
        version_map = {
            missing_sha: ("0.1.1", "0.1.2", "v0.1.2"),
            target_commit_sha: ("0.1.3", "0.1.4", "v0.1.4"),
        }
        if snapshot_sha not in version_map:
            raise AssertionError(f"unexpected snapshot build for {snapshot_sha}")
        base_version, next_version, release_tag = version_map[snapshot_sha]
        return {
            "schema_version": module.SNAPSHOT_SCHEMA_VERSION,
            "target_sha": snapshot_sha,
            "pr_number": 501 if snapshot_sha == missing_sha else 502,
            "pr_title": "Sparse catch-up",
            "registry": "ghcr.io",
            "pr_head_sha": snapshot_sha,
            "type_label": "type:patch",
            "channel_label": "channel:stable",
            "release_bump": "patch",
            "release_channel": "stable",
            "release_enabled": True,
            "release_prerelease": False,
            "image_name_lower": "ivanli-cn/codex-vibe-monitor",
            "base_stable_version": base_version,
            "next_stable_version": next_version,
            "app_effective_version": next_version,
            "release_tag": release_tag,
            "tags_csv": f"ghcr.io/ivanli-cn/codex-vibe-monitor:{release_tag}",
            "notes_ref": module.DEFAULT_NOTES_REF,
            "snapshot_source": "ci-main",
            "created_at": "2026-03-15T00:00:00Z",
        }

    existing_snapshot = {
        "schema_version": module.SNAPSHOT_SCHEMA_VERSION,
        "target_sha": existing_sha,
        "pr_number": 500,
        "pr_title": "Existing snapshot",
        "registry": "ghcr.io",
        "pr_head_sha": existing_sha,
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
        "tags_csv": "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.1",
        "notes_ref": module.DEFAULT_NOTES_REF,
        "snapshot_source": "ci-main",
        "created_at": "2026-03-15T00:00:00Z",
    }
    later_snapshot = {
        **existing_snapshot,
        "target_sha": later_sha,
        "pr_number": 503,
        "pr_head_sha": later_sha,
        "base_stable_version": "0.1.2",
        "next_stable_version": "0.1.3",
        "app_effective_version": "0.1.3",
        "release_tag": "v0.1.3",
        "tags_csv": "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.3",
    }

    run("notes", f"--ref={module.DEFAULT_NOTES_REF}", "add", "-f", "-m", json.dumps(existing_snapshot), existing_sha, cwd=repo)
    run("notes", f"--ref={module.DEFAULT_NOTES_REF}", "add", "-f", "-m", json.dumps(later_snapshot), later_sha, cwd=repo)

    os.chdir(repo)
    try:
        def fake_git(*args: str, **kwargs: object):
            if args == ("push", "origin", module.DEFAULT_NOTES_REF):
                return subprocess.CompletedProcess(["git", *args], 0, "", "")
            return original_git(*args, **kwargs)

        def fake_load_pr(api_root, repository, token, commit_sha, **kwargs):
            return {
                missing_sha: make_pr(501, "Missing snapshot", missing_sha, ["type:patch", "channel:stable"]),
                target_sha: make_pr(502, "Target pending", target_sha, ["type:patch", "channel:stable"]),
            }.get(commit_sha)

        module.load_pr_for_commit = fake_load_pr
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
                output=str(repo / "sparse-history.json"),
                max_attempts=1,
                target_only=False,
            )
        )
        assert exit_code == 0
        assert calls == [missing_sha, target_sha]
        missing_snapshot = module.read_snapshot(module.DEFAULT_NOTES_REF, missing_sha)
        assert missing_snapshot is not None
        assert missing_snapshot["next_stable_version"] == "0.1.2"
        target_snapshot = module.read_snapshot(module.DEFAULT_NOTES_REF, target_sha)
        assert target_snapshot is not None
        assert target_snapshot["base_stable_version"] == "0.1.3"
        assert target_snapshot["next_stable_version"] == "0.1.4"
    finally:
        module.load_pr_for_commit = original_load_pr
        module.build_snapshot = original_build_snapshot
        module.git = original_git
        os.chdir(original_cwd)


with tempfile.TemporaryDirectory(prefix="release-snapshot-catch-up-window-") as tmp:
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

    (repo / "README.md").write_text("legacy unlabeled\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "legacy unlabeled", cwd=repo)
    legacy_sha = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("existing snapshot\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "existing snapshot", cwd=repo)
    existing_sha = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("mid pending\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "mid pending", cwd=repo)
    mid_sha = run("rev-parse", "HEAD", cwd=repo)

    (repo / "README.md").write_text("target pending\n")
    run("add", "README.md", cwd=repo)
    run("commit", "-m", "target pending", cwd=repo)
    target_sha = run("rev-parse", "HEAD", cwd=repo)

    existing_snapshot = {
        "schema_version": module.SNAPSHOT_SCHEMA_VERSION,
        "target_sha": existing_sha,
        "pr_number": 601,
        "pr_title": "Existing snapshot",
        "registry": "ghcr.io",
        "pr_head_sha": existing_sha,
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
        "tags_csv": "ghcr.io/ivanli-cn/codex-vibe-monitor:v0.1.1",
        "notes_ref": module.DEFAULT_NOTES_REF,
        "snapshot_source": "ci-main",
        "created_at": "2026-03-15T00:00:00Z",
    }
    run("notes", f"--ref={module.DEFAULT_NOTES_REF}", "add", "-f", "-m", json.dumps(existing_snapshot), existing_sha, cwd=repo)

    original_cwd = Path.cwd()
    original_load_pr = module.load_pr_for_commit
    original_build_snapshot = module.build_snapshot
    original_git = module.git
    calls: list[str] = []

    def fake_build_snapshot(*, target_sha: str, **kwargs: object):
        calls.append(target_sha)
        version_map = {
            mid_sha: ("0.1.1", "0.1.2", "v0.1.2"),
            target_sha: ("0.1.2", "0.1.3", "v0.1.3"),
        }
        if target_sha not in version_map:
            raise AssertionError(f"unexpected snapshot build for {target_sha}")
        base_version, next_version, release_tag = version_map[target_sha]
        return {
            "schema_version": module.SNAPSHOT_SCHEMA_VERSION,
            "target_sha": target_sha,
            "pr_number": 602 if target_sha == mid_sha else 603,
            "pr_title": "Pending snapshot",
            "registry": "ghcr.io",
            "pr_head_sha": target_sha,
            "type_label": "type:patch",
            "channel_label": "channel:stable",
            "release_bump": "patch",
            "release_channel": "stable",
            "release_enabled": True,
            "release_prerelease": False,
            "image_name_lower": "ivanli-cn/codex-vibe-monitor",
            "base_stable_version": base_version,
            "next_stable_version": next_version,
            "app_effective_version": next_version,
            "release_tag": release_tag,
            "tags_csv": f"ghcr.io/ivanli-cn/codex-vibe-monitor:{release_tag}",
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

        def fake_load_pr(api_root, repository, token, commit_sha, **kwargs):
            if commit_sha == legacy_sha:
                raise AssertionError("catch-up must not walk older unlabeled history once a snapshot ancestor exists")
            return {
                mid_sha: make_pr(602, "Mid pending", mid_sha, ["type:patch", "channel:stable"]),
                target_sha: make_pr(603, "Target pending", target_sha, ["type:patch", "channel:stable"]),
            }.get(commit_sha)

        module.load_pr_for_commit = fake_load_pr
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
                output=str(repo / "catch-up-window.json"),
                max_attempts=1,
                target_only=False,
            )
        )
        assert exit_code == 0
        assert calls == [mid_sha, target_sha]
        assert module.read_snapshot(module.DEFAULT_NOTES_REF, legacy_sha) is None
    finally:
        module.load_pr_for_commit = original_load_pr
        module.build_snapshot = original_build_snapshot
        module.git = original_git
        os.chdir(original_cwd)

print("test-release-snapshot: all checks passed")
PY
