#!/usr/bin/env python3
"""Check explicit Rust line budgets against blobs staged in Git's index."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


class CheckError(RuntimeError):
    """A concise, user-actionable staged-source check failure."""


def run_git(repo_root: Path, *arguments: str) -> bytes:
    try:
        result = subprocess.run(
            ["git", "-C", str(repo_root), *arguments],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    except OSError as error:
        detail = error.strerror or "unable to start Git"
        raise CheckError(f"git invocation failed: {detail}") from error
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


def validate_repo_relative_path(raw_path: Any, where: str) -> str:
    if not isinstance(raw_path, str) or not raw_path:
        raise CheckError(f"{where} must be a non-empty repository-relative path")
    if (
        raw_path.startswith("/")
        or "\\" in raw_path
        or any(component in {"", ".", ".."} for component in raw_path.split("/"))
        or any(ord(character) < 0x20 or ord(character) == 0x7F for character in raw_path)
    ):
        raise CheckError(f"{where} must be a canonical repository-relative path without control characters")
    return raw_path


def validate_relative_path(raw_path: Any, where: str) -> str:
    path = validate_repo_relative_path(raw_path, where)
    if not path.endswith(".rs"):
        raise CheckError(f"{where} must be a repository-relative .rs path without control characters")
    return path


def load_policy(raw_policy: bytes, policy_path: str) -> dict[str, int]:
    try:
        policy = json.loads(raw_policy.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
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


def staged_rust_paths(repo_root: Path) -> list[bytes]:
    output = run_git(
        repo_root,
        "diff",
        "--cached",
        "--name-only",
        "--diff-filter=ACMRT",
        "--no-renames",
        "-z",
        "--",
    )
    paths: list[bytes] = []
    for raw_path in output.split(b"\0"):
        if not raw_path:
            continue
        if not raw_path.endswith(b".rs"):
            continue
        paths.append(raw_path)
    return paths


def index_blobs(repo_root: Path) -> dict[bytes, tuple[bytes, bytes]]:
    output = run_git(repo_root, "ls-files", "--stage", "-z", "--")
    blobs: dict[bytes, tuple[bytes, bytes]] = {}
    for record in output.split(b"\0"):
        if not record:
            continue
        try:
            metadata, raw_path = record.split(b"\t", 1)
            mode, object_id, stage = metadata.split()
        except ValueError as error:
            raise CheckError("unable to parse the Git index") from error
        if stage == b"0":
            blobs[raw_path] = (object_id, mode)
    return blobs


def index_blob(
    repo_root: Path,
    blobs: dict[bytes, tuple[bytes, bytes]],
    raw_path: bytes,
    path: str,
) -> bytes:
    entry = blobs.get(raw_path)
    if entry is None:
        raise CheckError(f"{path}: staged path has no stage-0 Git index blob")
    object_id, mode = entry
    if mode not in {b"100644", b"100755"}:
        raise CheckError(f"{path}: staged index entry must be a regular file")
    return run_git(repo_root, "cat-file", "blob", object_id.decode("ascii"))


def ensure_index_snapshot(
    repo_root: Path,
    staged_paths: list[bytes],
    blobs: dict[bytes, tuple[bytes, bytes]],
    relevant_paths: set[bytes],
) -> None:
    current_staged_paths = staged_rust_paths(repo_root)
    if current_staged_paths != staged_paths:
        raise CheckError("Git index changed during staged Rust line-budget check; retry the check")

    current_blobs = index_blobs(repo_root)
    for raw_path in relevant_paths:
        if current_blobs.get(raw_path) != blobs.get(raw_path):
            raise CheckError("Git index changed during staged Rust line-budget check; retry the check")


def staged_source(
    repo_root: Path,
    blobs: dict[bytes, tuple[bytes, bytes]],
    raw_path: bytes,
    path: str,
) -> str:
    content = index_blob(repo_root, blobs, raw_path, path)
    try:
        return content.decode("utf-8")
    except UnicodeDecodeError as error:
        raise CheckError(f"{path}: staged blob is not valid UTF-8") from error


def resolve_policy_path(repo_root: Path, raw_policy: str) -> str:
    candidate = Path(raw_policy)
    if candidate.is_absolute():
        try:
            relative = candidate.relative_to(repo_root)
        except ValueError as error:
            raise CheckError("policy path must remain inside the repository") from error
        raw_relative = relative.as_posix()
    else:
        raw_relative = raw_policy
    return validate_repo_relative_path(raw_relative, "policy path")


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
        staged_paths = staged_rust_paths(repo_root)
        if not staged_paths:
            return 0
        policy_relative_path = resolve_policy_path(repo_root, str(args.policy))
        blobs = index_blobs(repo_root)
        policy_content = index_blob(
            repo_root,
            blobs,
            policy_relative_path.encode("utf-8"),
            policy_relative_path,
        )
        budgets = load_policy(policy_content, policy_relative_path)
        selected_paths: list[tuple[bytes, str, int]] = []
        for raw_path in staged_paths:
            try:
                path = decode_git_path(raw_path)
            except CheckError:
                continue
            if path not in budgets:
                continue
            validate_relative_path(path, "staged path")
            selected_paths.append((raw_path, path, budgets[path]))
        if not selected_paths:
            ensure_index_snapshot(
                repo_root,
                staged_paths,
                blobs,
                {policy_relative_path.encode("utf-8")},
            )
            return 0
        failures: list[str] = []
        for raw_path, path, budget in selected_paths:
            actual = len(staged_source(repo_root, blobs, raw_path, path).splitlines())
            if actual > budget:
                failures.append(
                    f"{path}: {actual} physical lines exceeds explicit budget {budget}"
                )
        ensure_index_snapshot(
            repo_root,
            staged_paths,
            blobs,
            {policy_relative_path.encode("utf-8")}
            | {raw_path for raw_path, _, _ in selected_paths},
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
