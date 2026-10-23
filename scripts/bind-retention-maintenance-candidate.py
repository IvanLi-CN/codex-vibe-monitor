#!/usr/bin/env python3
"""Bind retention validation evidence to the current checkout and binary."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess


CARRIER_FILES = frozenset(
    {
        "docs/specs/retention-maintenance-task-ownership/IMPLEMENTATION.md",
        "docs/adr/assets/retention-maintenance-task-ownership/current-candidate-validation.json",
        "docs/adr/assets/retention-maintenance-task-ownership/persistent-state-migration-record.json",
        "docs/adr/assets/retention-maintenance-task-ownership/version-impact-record.json",
    }
)


def git(*arguments: str) -> str:
    return subprocess.run(
        ["git", *arguments],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def parent(commit: str, index: int = 1) -> str | None:
    try:
        return git("rev-parse", "--verify", f"{commit}^{index}")
    except subprocess.CalledProcessError:
        return None


def changed_files(base: str, head: str) -> set[str]:
    return {
        path
        for path in git("diff", "--name-only", "--no-renames", base, head).splitlines()
        if path
    }


def tree(commit: str) -> str:
    return git("rev-parse", f"{commit}^{{tree}}")


def carrier_candidate(commit: str, code_candidate: str) -> tuple[str, set[str]] | None:
    commit_parent = parent(commit)
    if commit_parent is None:
        return None
    files = changed_files(commit_parent, commit)
    if not files & CARRIER_FILES:
        return None
    if commit_parent != code_candidate:
        raise SystemExit(
            f"current carrier candidate {commit} must have code candidate parent {code_candidate}, "
            f"not {commit_parent}"
        )
    if files != CARRIER_FILES:
        raise SystemExit(
            "current documentation carrier must change exactly the four bound files; "
            f"got {sorted(files)}"
        )
    return commit, files


def bind_candidate(record_path: Path, github_sha: str, binary_sha256: str) -> dict[str, str]:
    checkout_sha = git("rev-parse", "HEAD")
    if checkout_sha != github_sha:
        raise SystemExit(f"GITHUB_SHA {github_sha} does not match checkout {checkout_sha}")
    record = json.loads(record_path.read_text())
    code_candidate = record["codeCandidateSha"]

    merge_group_head = parent(checkout_sha, 2)
    if merge_group_head is None:
        carrier = carrier_candidate(checkout_sha, code_candidate)
    else:
        carrier = carrier_candidate(merge_group_head, code_candidate)
        if carrier is not None and tree(checkout_sha) != tree(merge_group_head):
            # A multi-PR merge group contains more than this carrier. Bind the
            # artifact to the actual checkout instead of an older history node.
            carrier = None

    if carrier is None:
        implementation_sha = checkout_sha
        documentation_carrier_sha = checkout_sha
        binding = "checkout"
    else:
        documentation_carrier_sha, _ = carrier
        implementation_sha = code_candidate
        binding = "documentation-carrier"

    return {
        "githubSha": github_sha,
        "checkoutSha": checkout_sha,
        "implementationCandidateSha": implementation_sha,
        "documentationCarrierSha": documentation_carrier_sha,
        "candidateSha256": binary_sha256,
        "binding": binding,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--github-sha", required=True)
    parser.add_argument("--binary-sha256", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    identity = bind_candidate(args.record, args.github_sha, args.binary_sha256)
    args.output.write_text(json.dumps(identity, sort_keys=True) + "\n")
    print(json.dumps(identity, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
