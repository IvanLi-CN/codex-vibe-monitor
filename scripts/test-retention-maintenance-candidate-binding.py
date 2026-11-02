#!/usr/bin/env python3
"""Exercise retention candidate binding against small synthetic Git histories."""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile


SOURCE = Path(__file__).with_name("bind-retention-maintenance-candidate.py")
SPEC = importlib.util.spec_from_file_location("retention_candidate_binding", SOURCE)
BINDING = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(BINDING)


def run(root: Path, *arguments: str, input_text: str | None = None) -> str:
    return subprocess.run(
        [*arguments],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
        input=input_text,
    ).stdout.strip()


def commit(root: Path, files: dict[str, str], message: str) -> str:
    for relative, content in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
    run(root, "git", "add", ".")
    run(root, "git", "commit", "-m", message)
    return run(root, "git", "rev-parse", "HEAD")


def identity(root: Path, github_sha: str) -> dict[str, str]:
    previous = Path.cwd()
    os.chdir(root)
    try:
        payload = BINDING.bind_candidate(
            root / "docs/adr/assets/retention-maintenance-task-ownership/current-candidate-validation.json",
            github_sha,
            "binary-sha",
        )
    finally:
        os.chdir(previous)
    return payload


def carrier_files(code_sha: str) -> dict[str, str]:
    record = json.dumps({"codeCandidateSha": code_sha}) + "\n"
    return {
        "docs/specs/retention-maintenance-task-ownership/IMPLEMENTATION.md": "implementation\n",
        "docs/adr/assets/retention-maintenance-task-ownership/current-candidate-validation.json": record,
        "docs/adr/assets/retention-maintenance-task-ownership/persistent-state-migration-record.json": "migration\n",
        "docs/adr/assets/retention-maintenance-task-ownership/version-impact-record.json": "version\n",
    }


def test_bindings() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        run(root, "git", "init", "-q", "-b", "main")
        run(root, "git", "config", "user.name", "Candidate Binding Test")
        run(root, "git", "config", "user.email", "candidate-binding@example.invalid")
        base = commit(root, {"README": "base\n"}, "test: create base")
        code = commit(root, {"src/candidate.rs": "candidate\n"}, "test: create code candidate")
        carrier = commit(root, carrier_files(code), "docs: create carrier")

        run(root, "git", "switch", "--detach", carrier)
        strict = identity(root, carrier)
        assert strict["binding"] == "documentation-carrier", strict
        assert strict["implementationCandidateSha"] == code, strict
        assert strict["documentationCarrierSha"] == carrier, strict

        future = commit(root, {"src/future.rs": "future\n"}, "feat: future source change")
        generic = identity(root, future)
        assert generic["binding"] == "checkout", generic
        assert generic["implementationCandidateSha"] == future, generic
        assert generic["documentationCarrierSha"] == future, generic

        progress = commit(
            root,
            {
                "docs/specs/retention-maintenance-task-ownership/IMPLEMENTATION.md": "future progress\n",
                "docs/specs/task-operations/IMPLEMENTATION.md": "future task progress\n",
                "docs/specs/bounded-retention-runs/IMPLEMENTATION.md": "future bounded progress\n",
            },
            "docs: synchronize future implementation",
        )
        progress_identity = identity(root, progress)
        assert progress_identity["binding"] == "checkout", progress_identity
        assert progress_identity["implementationCandidateSha"] == progress, progress_identity
        assert progress_identity["documentationCarrierSha"] == progress, progress_identity
        try:
            identity(root, future)
        except SystemExit:
            pass
        else:
            raise AssertionError("mismatched checkout and GitHub SHA was accepted")

        run(root, "git", "switch", "--detach", carrier)
        feature_one = commit(root, {"src/feature-one.rs": "one\n"}, "feat: first merge-group change")
        feature_two = commit(root, {"src/feature-two.rs": "two\n"}, "feat: second merge-group change")
        run(root, "git", "switch", "--detach", carrier)
        merge_tree = run(root, "git", "rev-parse", f"{feature_two}^{{tree}}")
        merge_group = run(root, "git", "commit-tree", merge_tree, "-p", carrier, "-p", feature_two, input_text="merge group\n")
        run(root, "git", "reset", "--hard", "-q", merge_group)
        multi_pr = identity(root, merge_group)
        assert multi_pr["binding"] == "checkout", multi_pr
        assert multi_pr["implementationCandidateSha"] == merge_group, multi_pr
        assert feature_one != feature_two

        run(root, "git", "switch", "--detach", base)
        merge_tree = run(root, "git", "rev-parse", f"{carrier}^{{tree}}")
        carrier_merge_group = run(root, "git", "commit-tree", merge_tree, "-p", base, "-p", carrier, input_text="carrier merge group\n")
        run(root, "git", "reset", "--hard", "-q", carrier_merge_group)
        merged_carrier = identity(root, carrier_merge_group)
        assert merged_carrier["binding"] == "documentation-carrier", merged_carrier
        assert merged_carrier["documentationCarrierSha"] == carrier, merged_carrier

        run(root, "git", "switch", "--detach", code)
        corrupted = commit(root, {**carrier_files(code), "docs/extra.md": "unexpected\n"}, "docs: corrupt carrier")
        try:
            identity(root, corrupted)
        except SystemExit:
            pass
        else:
            raise AssertionError("corrupted carrier was accepted")

        run(root, "git", "switch", "--detach", code)
        incomplete_files = carrier_files(code)
        del incomplete_files["docs/specs/retention-maintenance-task-ownership/IMPLEMENTATION.md"]
        incomplete = commit(root, incomplete_files, "docs: incomplete carrier")
        try:
            identity(root, incomplete)
        except SystemExit:
            pass
        else:
            raise AssertionError("carrier missing its implementation document was accepted")

        run(root, "git", "switch", "--detach", future)
        wrong_parent_files = carrier_files(code)
        wrong_parent_files["docs/specs/retention-maintenance-task-ownership/IMPLEMENTATION.md"] = "changed carrier\n"
        wrong_parent_files["docs/adr/assets/retention-maintenance-task-ownership/current-candidate-validation.json"] = json.dumps({"codeCandidateSha": code, "status": "pending"}) + "\n"
        wrong_parent_files["docs/adr/assets/retention-maintenance-task-ownership/persistent-state-migration-record.json"] = "changed migration\n"
        wrong_parent_files["docs/adr/assets/retention-maintenance-task-ownership/version-impact-record.json"] = "changed version\n"
        wrong_parent = commit(root, wrong_parent_files, "docs: stale carrier parent")
        try:
            identity(root, wrong_parent)
        except SystemExit:
            pass
        else:
            raise AssertionError("carrier with a stale implementation parent was accepted")


if __name__ == "__main__":
    test_bindings()
    print("test-retention-maintenance-candidate-binding: all checks passed")
