#!/usr/bin/env python3
"""Check the explicit Rust source-quality policy without third-party dependencies."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path
from typing import Any


EXPECTED_LOC_TARGETS = {"production": 2500, "test_helper": 3000}
ATTRIBUTE_RE = re.compile(r"#\[\s*(allow|expect)\b")
INCLUDE_RE = re.compile(r"\binclude\s*!\s*\(")
VAGUE_MARKERS = ("TBD", "TODO", "WILDCARD")


def normalize_attribute(attribute: str) -> str:
    return " ".join(attribute.split())


def physical_line_count(source: str) -> int:
    return len(source.splitlines())


def _raw_string_end(source: str, start: int) -> int | None:
    prefix_end: int
    if source.startswith("br", start):
        prefix_end = start + 2
    elif source.startswith("r", start):
        prefix_end = start + 1
    else:
        return None

    hash_count = 0
    while prefix_end + hash_count < len(source) and source[prefix_end + hash_count] == "#":
        hash_count += 1
    quote_index = prefix_end + hash_count
    if quote_index >= len(source) or source[quote_index] != '"':
        return None

    terminator = '"' + ("#" * hash_count)
    end = source.find(terminator, quote_index + 1)
    return len(source) if end == -1 else end + len(terminator)


def _quoted_end(source: str, quote_index: int) -> int:
    index = quote_index + 1
    while index < len(source):
        if source[index] == "\\":
            index += 2
            continue
        if source[index] == source[quote_index]:
            return index + 1
        index += 1
    return len(source)


def _literal_end(source: str, start: int) -> int | None:
    raw_end = _raw_string_end(source, start)
    if raw_end is not None:
        return raw_end

    if source.startswith("b\"", start) or source.startswith("b'", start):
        return _quoted_end(source, start + 1)
    if start >= len(source):
        return None
    if source[start] == '"':
        return _quoted_end(source, start)
    if source[start] == "'":
        # A lifetime such as 'static is not a character literal.
        if start + 2 < len(source) and (
            source[start + 1] == "\\" or source[start + 2] == "'"
        ):
            return _quoted_end(source, start)
    return None


def _block_comment_end(source: str, start: int) -> int:
    depth = 1
    index = start + 2
    while index < len(source) - 1:
        if source.startswith("/*", index):
            depth += 1
            index += 2
        elif source.startswith("*/", index):
            depth -= 1
            index += 2
            if depth == 0:
                return index
        else:
            index += 1
    return len(source)


def _balanced_attribute_end(source: str, start: int) -> int | None:
    depth = 0
    index = start
    while index < len(source):
        if source.startswith("//", index):
            newline = source.find("\n", index + 2)
            index = len(source) if newline == -1 else newline + 1
            continue
        if source.startswith("/*", index):
            index = _block_comment_end(source, index)
            continue
        literal_end = _literal_end(source, index)
        if literal_end is not None:
            index = literal_end
            continue
        character = source[index]
        if character == "[":
            depth += 1
        elif character == "]":
            depth -= 1
            if depth == 0:
                return index + 1
        index += 1
    return None


def collect_suppressions(repo_root: Path) -> list[dict[str, Any]]:
    suppressions: list[dict[str, Any]] = []
    for path in sorted((repo_root / "src").rglob("*.rs")):
        source = path.read_text(encoding="utf-8")
        index = 0
        while index < len(source):
            if source.startswith("//", index):
                newline = source.find("\n", index + 2)
                index = len(source) if newline == -1 else newline + 1
                continue
            if source.startswith("/*", index):
                index = _block_comment_end(source, index)
                continue
            literal_end = _literal_end(source, index)
            if literal_end is not None:
                index = literal_end
                continue
            if source.startswith("#[", index):
                match = ATTRIBUTE_RE.match(source, index)
                end = _balanced_attribute_end(source, index)
                if end is None:
                    break
                if match is not None:
                    suppressions.append(
                        {
                            "path": path.relative_to(repo_root).as_posix(),
                            "kind": match.group(1),
                            "declaration": normalize_attribute(source[index:end]),
                            "line": source.count("\n", 0, index) + 1,
                        }
                    )
                # Skip the whole attribute so nested cfg_attr tokens cannot be
                # mistaken for standalone suppression declarations.
                index = end
                continue
            index += 1
    return suppressions


def mask_non_code(source: str) -> str:
    masked = list(source)

    def blank(start: int, end: int) -> None:
        for offset in range(start, end):
            if masked[offset] != "\n":
                masked[offset] = " "

    index = 0
    while index < len(source):
        if source.startswith("//", index):
            newline = source.find("\n", index + 2)
            end = len(source) if newline == -1 else newline
            blank(index, end)
            index = end
            continue
        if source.startswith("/*", index):
            end = _block_comment_end(source, index)
            blank(index, end)
            index = end
            continue
        literal_end = _literal_end(source, index)
        if literal_end is not None:
            blank(index, literal_end)
            index = literal_end
            continue
        index += 1
    return "".join(masked)


def policy_path(repo_root: Path, raw_path: str) -> Path:
    candidate = Path(raw_path)
    return candidate if candidate.is_absolute() else repo_root / candidate


def _validate_relative_source_path(raw_path: Any, where: str, errors: list[str]) -> str | None:
    if not isinstance(raw_path, str) or not raw_path:
        errors.append(f"{where}.path must be a non-empty string")
        return None
    if (
        raw_path.startswith("/")
        or "\\" in raw_path
        or ".." in Path(raw_path).parts
        or not raw_path.startswith("src/")
        or not raw_path.endswith(".rs")
        or any(marker in raw_path for marker in ("*", "?", "[", "]"))
    ):
        errors.append(f"{where}.path must be an explicit src/**/*.rs path without wildcards")
        return None
    return raw_path


def _validate_reason(value: Any, where: str, errors: list[str], label: str) -> None:
    if not isinstance(value, str) or len(value.strip()) < 20:
        errors.append(f"{where}.{label} must be a narrow reason of at least 20 characters")
        return
    upper = value.upper()
    if any(marker in upper for marker in VAGUE_MARKERS) or "*" in value:
        errors.append(f"{where}.{label} must not use TBD, TODO, wildcard, or placeholder wording")


def validate_policy(
    repo_root: Path, policy: Any
) -> tuple[list[dict[str, Any]], list[dict[str, Any]], list[str]]:
    errors: list[str] = []
    if not isinstance(policy, dict):
        return [], [], ["policy must be a JSON object"]
    if policy.get("schema_version") != 1:
        errors.append("policy.schema_version must be 1")
    targets = policy.get("loc_targets")
    if targets != EXPECTED_LOC_TARGETS:
        errors.append(f"policy.loc_targets must stay {EXPECTED_LOC_TARGETS!r}")

    file_records: list[dict[str, Any]] = []
    raw_files = policy.get("files")
    if not isinstance(raw_files, list):
        errors.append("policy.files must be an array of explicit path entries")
    else:
        for index, record in enumerate(raw_files):
            where = f"policy.files[{index}]"
            if not isinstance(record, dict):
                errors.append(f"{where} must be an object")
                continue
            path = _validate_relative_source_path(record.get("path"), where, errors)
            role = record.get("role")
            if role not in EXPECTED_LOC_TARGETS:
                errors.append(f"{where}.role must be 'production' or 'test_helper'")
            budget = record.get("line_budget")
            if isinstance(budget, bool) or not isinstance(budget, int) or budget <= 0:
                errors.append(f"{where}.line_budget must be a positive integer")
            workstream = record.get("next_module_workstream")
            exception = record.get("cohesive_exception")
            if (workstream is None) == (exception is None):
                errors.append(
                    f"{where} must contain exactly one of next_module_workstream or cohesive_exception"
                )
            if workstream is not None:
                _validate_reason(workstream, where, errors, "next_module_workstream")
            if exception is not None:
                if not isinstance(exception, dict):
                    errors.append(f"{where}.cohesive_exception must be an object")
                else:
                    _validate_reason(exception.get("reason"), where, errors, "cohesive_exception.reason")
            if path is not None:
                file_records.append(record)

        paths = [record["path"] for record in file_records]
        if len(paths) != len(set(paths)):
            errors.append("policy.files must not contain duplicate paths")

    baseline = policy.get("baseline")
    if baseline is not None:
        if not isinstance(baseline, dict):
            errors.append("policy.baseline must be an object")
        else:
            if not isinstance(baseline.get("commit"), str) or not re.fullmatch(r"[0-9a-f]{40}", baseline["commit"]):
                errors.append("policy.baseline.commit must be a full lowercase commit SHA")
            if baseline.get("production_candidates") != 32:
                errors.append("policy.baseline.production_candidates must remain 32")
            if baseline.get("test_helper_candidates") != 23:
                errors.append("policy.baseline.test_helper_candidates must remain 23")
            expected_entries = baseline.get("inventory_entries")
            if expected_entries != len(file_records):
                errors.append(
                    "policy.baseline.inventory_entries must equal the explicit file inventory length"
                )
            expected_suppressions = baseline.get("suppression_entries")
            raw_suppressions = policy.get("suppressions")
            actual_suppression_entries = len(raw_suppressions) if isinstance(raw_suppressions, list) else -1
            if expected_suppressions != actual_suppression_entries:
                errors.append(
                    "policy.baseline.suppression_entries must equal the explicit suppression inventory length"
                )

    suppression_records: list[dict[str, Any]] = []
    raw_suppressions = policy.get("suppressions")
    if not isinstance(raw_suppressions, list):
        errors.append("policy.suppressions must be an array")
    else:
        for index, record in enumerate(raw_suppressions):
            where = f"policy.suppressions[{index}]"
            if not isinstance(record, dict):
                errors.append(f"{where} must be an object")
                continue
            path = _validate_relative_source_path(record.get("path"), where, errors)
            if record.get("kind") not in {"allow", "expect"}:
                errors.append(f"{where}.kind must be 'allow' or 'expect'")
            declaration = record.get("declaration")
            if not isinstance(declaration, str) or not declaration.startswith("#["):
                errors.append(f"{where}.declaration must be a normalized Rust attribute")
            else:
                if declaration != normalize_attribute(declaration):
                    errors.append(f"{where}.declaration must use single-space normalization")
                kind = record.get("kind")
                if not re.match(rf"#\[\s*{re.escape(str(kind))}\b", declaration):
                    errors.append(f"{where}.declaration must start with the declared suppression kind")
            _validate_reason(record.get("reason"), where, errors, "reason")
            if path is not None and isinstance(declaration, str):
                suppression_records.append(record)

    return file_records, suppression_records, errors


def check_repository(repo_root: Path, policy: Any) -> list[str]:
    file_records, suppression_records, errors = validate_policy(repo_root, policy)
    if errors:
        return errors

    src_root = repo_root / "src"
    for record in file_records:
        relative_path = record["path"]
        path = repo_root / relative_path
        if not path.is_file():
            errors.append(f"{relative_path}: selected policy path does not exist")
            continue
        try:
            source = path.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(f"{relative_path}: unable to read selected policy path: {error}")
            continue
        actual_lines = physical_line_count(source)
        budget = record["line_budget"]
        if actual_lines > budget:
            errors.append(
                f"{relative_path}: selected path has {actual_lines} physical lines; explicit budget is {budget}"
            )

    actual_suppressions = collect_suppressions(repo_root)
    expected_keys = Counter(
        (record["path"], record["kind"], record["declaration"]) for record in suppression_records
    )
    actual_keys = Counter(
        (record["path"], record["kind"], record["declaration"]) for record in actual_suppressions
    )
    for key, count in sorted((actual_keys - expected_keys).items()):
        path, kind, declaration = key
        errors.append(
            f"{path}: unrecorded or materially altered {kind} suppression ({count} occurrence(s)): {declaration}"
        )
    for key, count in sorted((expected_keys - actual_keys).items()):
        path, kind, declaration = key
        errors.append(
            f"{path}: suppression policy has {count} stale {kind} inventory occurrence(s): {declaration}"
        )

    for path in sorted(src_root.rglob("*.rs")):
        relative_path = path.relative_to(repo_root).as_posix()
        source = path.read_text(encoding="utf-8")
        masked_source = mask_non_code(source)
        for match in INCLUDE_RE.finditer(masked_source):
            line = masked_source.count("\n", 0, match.start()) + 1
            errors.append(f"{relative_path}:{line}: include! control-flow composition is not allowed")

    return errors


def load_json(path: Path) -> Any:
    with path.open(encoding="utf-8") as stream:
        return json.load(stream)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=Path.cwd())
    parser.add_argument("--policy", type=Path)
    parser.add_argument(
        "--dump-suppressions",
        action="store_true",
        help="print the current standalone allow/expect inventory as JSON",
    )
    args = parser.parse_args()
    repo_root = args.repo_root.resolve()

    if args.dump_suppressions:
        print(json.dumps(collect_suppressions(repo_root), indent=2, ensure_ascii=True))
        return 0

    selected_policy = args.policy or Path(".github/rust-source-quality-policy.json")
    policy_file = policy_path(repo_root, str(selected_policy))
    try:
        policy = load_json(policy_file)
    except (OSError, json.JSONDecodeError) as error:
        print(f"unable to load Rust source-quality policy {policy_file}: {error}", file=sys.stderr)
        return 1

    errors = check_repository(repo_root, policy)
    if errors:
        for error in errors:
            print(f"rust-source-quality: {error}", file=sys.stderr)
        return 1
    print("rust-source-quality: explicit policy checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
