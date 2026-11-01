#!/usr/bin/env python3
"""Check explicit Rust line budgets against blobs staged in Git's index."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path, PurePosixPath
from typing import Any


class CheckError(RuntimeError):
    """A concise, user-actionable staged-source check failure."""


def run_git(repo_root: Path, *arguments: str) -> bytes:
    result = subprocess.run(
        ["git", "-C", str(repo_root), *arguments],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", errors="replace").strip()
        command = "git " + " ".join(arguments)
        raise CheckError(f"{command} failed{f': {detail}' if detail else ''}")
    return result.stdout


def decode_git_path(raw_path: bytes) -> str:
    try:
        return raw_path.decode("utf-8")
    except UnicodeDecodeError as error:
        raise CheckError("staged Rust path is not valid UTF-8") from error


def validate_relative_path(raw_path: Any, where: str) -> str:
    if not isinstance(raw_path, str) or not raw_path:
        raise CheckError(f"{where} must be a non-empty repository-relative path")
    path = PurePosixPath(raw_path)
    if (
        raw_path.startswith("/")
        or "\\" in raw_path
        or path.is_absolute()
        or ".." in path.parts
        or not raw_path.endswith(".rs")
    ):
        raise CheckError(f"{where} must be a repository-relative .rs path")
    return raw_path


def load_policy(policy_path: Path) -> dict[str, int]:
    try:
        with policy_path.open(encoding="utf-8") as stream:
            policy = json.load(stream)
    except (OSError, json.JSONDecodeError) as error:
        raise CheckError(f"unable to load Rust source-quality policy {policy_path}: {error}") from error

    if not isinstance(policy, dict):
        raise CheckError("Rust source-quality policy must be a JSON object")
    raw_files = policy.get("files")
    if not isinstance(raw_files, list):
        raise CheckError("Rust source-quality policy.files must be an array")

    budgets: dict[str, int] = {}
    for index, record in enumerate(raw_files):
        where = f"policy.files[{index}]"
        if not isinstance(record, dict):
            raise CheckError(f"{where} must be an object")
        path = validate_relative_path(record.get("path"), f"{where}.path")
        if not path.startswith("src/"):
            raise CheckError(f"{where}.path must be an explicit src/**/*.rs path")
        budget = record.get("line_budget")
        if isinstance(budget, bool) or not isinstance(budget, int) or budget <= 0:
            raise CheckError(f"{where}.line_budget must be a positive integer")
        if path in budgets:
            raise CheckError(f"policy.files must not contain duplicate path {path}")
        budgets[path] = budget
    return budgets


def staged_rust_paths(repo_root: Path) -> list[tuple[bytes, str]]:
    output = run_git(
        repo_root,
        "diff",
        "--cached",
        "--name-only",
        "--diff-filter=ACMR",
        "--no-renames",
        "-z",
        "--",
    )
    paths: list[tuple[bytes, str]] = []
    for raw_path in output.split(b"\0"):
        if not raw_path:
            continue
        if not raw_path.endswith(b".rs"):
            continue
        path = decode_git_path(raw_path)
        validate_relative_path(path, "staged path")
        paths.append((raw_path, path))
    return paths


def index_blobs(repo_root: Path) -> dict[bytes, bytes]:
    output = run_git(repo_root, "ls-files", "--stage", "-z", "--")
    blobs: dict[bytes, bytes] = {}
    for record in output.split(b"\0"):
        if not record:
            continue
        try:
            metadata, raw_path = record.split(b"\t", 1)
            _mode, object_id, stage = metadata.split()
        except ValueError as error:
            raise CheckError("unable to parse the Git index") from error
        if stage == b"0":
            blobs[raw_path] = object_id
    return blobs


def staged_source(repo_root: Path, blobs: dict[bytes, bytes], raw_path: bytes, path: str) -> str:
    object_id = blobs.get(raw_path)
    if object_id is None:
        raise CheckError(f"{path}: staged path has no stage-0 Git index blob")
    content = run_git(repo_root, "cat-file", "blob", object_id.decode("ascii"))
    try:
        return content.decode("utf-8")
    except UnicodeDecodeError as error:
        raise CheckError(f"{path}: staged blob is not valid UTF-8") from error


def resolve_policy_path(repo_root: Path, raw_policy: str) -> Path:
    candidate = Path(raw_policy)
    if not candidate.is_absolute():
        candidate = repo_root / candidate
    resolved = candidate.resolve()
    try:
        resolved.relative_to(repo_root)
    except ValueError as error:
        raise CheckError("policy path must remain inside the repository") from error
    return resolved


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path.cwd())
    parser.add_argument(
        "--policy",
        type=Path,
        default=Path(".github/rust-source-quality-policy.json"),
    )
    args = parser.parse_args()
    repo_root = args.repo_root.resolve()

    try:
        budgets = load_policy(resolve_policy_path(repo_root, str(args.policy)))
        staged_paths = staged_rust_paths(repo_root)
        selected_paths = [(raw_path, path, budgets[path]) for raw_path, path in staged_paths if path in budgets]
        if not selected_paths:
            return 0
        blobs = index_blobs(repo_root)
        failures: list[str] = []
        for raw_path, path, budget in selected_paths:
            actual = len(staged_source(repo_root, blobs, raw_path, path).splitlines())
            if actual > budget:
                failures.append(
                    f"{path}: {actual} physical lines exceeds explicit budget {budget}"
                )
    except CheckError as error:
        print(f"rust-source-quality: {error}", file=sys.stderr)
        return 1

    if failures:
        for failure in failures:
            print(f"rust-source-quality: {failure}", file=sys.stderr)
        return 1
    print("rust-source-quality: staged line budgets passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
