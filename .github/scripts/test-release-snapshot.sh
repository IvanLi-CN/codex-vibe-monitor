#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 - <<'PY' "$repo_root/.github/scripts/release_snapshot.py"
from __future__ import annotations

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

    os.chdir(repo)

    module.load_pr_for_commit = lambda api_root, repository, token, target_sha: {
        "number": 101,
        "title": f"Release {target_sha[:7]}",
        "labels": [{"name": "type:patch"}, {"name": "channel:stable"}],
    }
    snapshot1 = module.build_snapshot(
        target_sha=sha1,
        repository="IvanLi-CN/codex-vibe-monitor",
        token="token",
        notes_ref=module.DEFAULT_NOTES_REF,
        registry="ghcr.io",
        api_root="https://api.github.com",
    )
    assert snapshot1["next_stable_version"] == "0.1.1"
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

    module.load_pr_for_commit = lambda api_root, repository, token, target_sha: {
        "number": 102,
        "title": f"RC {target_sha[:7]}",
        "labels": [{"name": "type:patch"}, {"name": "channel:rc"}],
    }
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

    read_back = module.read_snapshot(module.DEFAULT_NOTES_REF, sha2)
    assert read_back is not None
    assert read_back["release_tag"] == "v0.1.2"

print("test-release-snapshot: all checks passed")
PY
