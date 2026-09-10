#!/usr/bin/env python3
"""Compare a served Summary response with an independent source reducer.

The oracle intentionally reads only normalized invocation fields from the staged SQLite copy and
its authoritative archive files. It never imports the service implementation or its projection
tables, so a stale/partial projection cannot satisfy the production-copy gate.
"""

from __future__ import annotations

import argparse
import datetime as dt
import gzip
import json
import sqlite3
import subprocess
import tempfile
from pathlib import Path
from typing import Any


UTC = dt.timezone.utc
SUCCESS_STATUSES = {"success", "completed", "warning_success"}


def parse_time(value: str) -> dt.datetime:
    normalized = value.replace(" ", "T")
    if normalized.endswith("Z"):
        normalized = normalized[:-1] + "+00:00"
    parsed = dt.datetime.fromisoformat(normalized)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=UTC)
    return parsed.astimezone(UTC)


def source_rows(connection: sqlite3.Connection) -> list[dict[str, Any]]:
    columns = {
        row[1] for row in connection.execute("PRAGMA table_info(codex_invocations)")
    }
    required = {
        "id",
        "invoke_id",
        "occurred_at",
        "source",
        "status",
        "total_tokens",
        "cost",
    }
    if not required.issubset(columns):
        return []
    optional = [
        "error_message",
        "failure_kind",
        "failure_class",
        "is_actionable",
        "input_tokens",
        "output_tokens",
        "cache_input_tokens",
        "reasoning_tokens",
        "model",
        "response_model",
        "upstream_account_id",
    ]
    selected = sorted(required) + [column for column in optional if column in columns]
    return [
        dict(zip(selected, row, strict=True))
        for row in connection.execute(
            f"SELECT {', '.join(selected)} FROM codex_invocations"
        ).fetchall()
    ]


def open_archive(path: Path) -> tuple[sqlite3.Connection, tempfile.NamedTemporaryFile]:
    raw = path.read_bytes()
    if path.name.endswith((".gz", ".gzip")):
        raw = gzip.decompress(raw)
    temporary = tempfile.NamedTemporaryFile(prefix="summary-oracle-", suffix=".db")
    temporary.write(raw)
    temporary.flush()
    return sqlite3.connect(temporary.name), temporary


def decode_v2_payload(payload: bytes) -> list[dict[str, Any]]:
    try:
        decoded = subprocess.run(
            ["zstd", "-q", "-d", "-c"],
            input=payload,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=True,
        ).stdout
    except (FileNotFoundError, subprocess.CalledProcessError) as error:
        raise RuntimeError("zstd is required to decode V2 Snapshot pages") from error
    records = json.loads(decoded)
    if not isinstance(records, list):
        raise RuntimeError("V2 Snapshot payload is not a record list")
    field_names = {
        "invokeId": "invoke_id",
        "occurredAt": "occurred_at",
        "inputTokens": "input_tokens",
        "outputTokens": "output_tokens",
        "cacheInputTokens": "cache_input_tokens",
        "reasoningTokens": "reasoning_tokens",
        "reasoningEffort": "reasoning_effort",
        "totalTokens": "total_tokens",
        "costInput": "cost_input",
        "costCacheWrite": "cost_cache_write",
        "costCacheRead": "cost_cache_read",
        "costOutput": "cost_output",
        "costReasoning": "cost_reasoning",
        "errorMessage": "error_message",
        "failureKind": "failure_kind",
        "failureClass": "failure_class",
        "isActionable": "is_actionable",
        "responseModel": "response_model",
        "upstreamAccountId": "upstream_account_id",
    }
    normalized = []
    for record in records:
        if not isinstance(record, dict):
            continue
        normalized.append(
            {field_names.get(key, key): value for key, value in record.items()}
        )
    return normalized


def load_rows(database: Path, archive_root: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    connection = sqlite3.connect(database)
    try:
        rows.extend(source_rows(connection))
        tables = {
            row[0]
            for row in connection.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            ).fetchall()
        }
        batches = (
            connection.execute(
                "SELECT id, file_path, status FROM archive_batches "
                "WHERE dataset = 'codex_invocations' AND status = 'completed'"
            ).fetchall()
            if "archive_batches" in tables
            else []
        )
        snapshot_rows = (
            connection.execute(
                "SELECT payload, format_version FROM summary_archive_snapshot "
                "WHERE format_version >= 2 ORDER BY archive_batch_id, manifest_sha256, page_index"
            ).fetchall()
            if "summary_archive_snapshot" in tables
            else []
        )
        for payload, format_version in snapshot_rows:
            if int(format_version) >= 2:
                rows.extend(decode_v2_payload(payload))
        for _batch_id, stored_path, _status in batches:
            parts = Path(stored_path).parts
            try:
                archive_index = max(index for index, part in enumerate(parts) if part == "archives")
            except ValueError as error:
                raise RuntimeError("archive manifest path has no archives root") from error
            candidate = (archive_root.joinpath(*parts[archive_index + 1 :])).resolve()
            if archive_root not in (candidate, *candidate.parents) or not candidate.is_file():
                raise RuntimeError(f"archive authority is missing: {candidate}")
            archive_connection, archive_temporary = open_archive(candidate)
            try:
                rows.extend(source_rows(archive_connection))
            finally:
                archive_connection.close()
                archive_temporary.close()
    finally:
        connection.close()

    deduplicated: dict[tuple[Any, ...], dict[str, Any]] = {}
    for row in rows:
        identity = (
            "id",
            row.get("id"),
        ) if row.get("id") is not None else (
            "invoke",
            row.get("invoke_id"),
            row.get("occurred_at"),
        )
        deduplicated[identity] = row
    return list(deduplicated.values())


def is_success(row: dict[str, Any]) -> bool:
    status = str(row.get("status") or "").strip().lower()
    return status in SUCCESS_STATUSES or (
        status == "http_200" and not str(row.get("error_message") or "").strip()
    )


def is_terminal_failure(row: dict[str, Any]) -> bool:
    status = str(row.get("status") or "").strip().lower()
    return status not in {"running", "pending"} and not is_success(row)


def numeric(row: dict[str, Any], key: str) -> float:
    value = row.get(key)
    return float(value or 0)


def expected(rows: list[dict[str, Any]], window: str, now: dt.datetime) -> dict[str, Any]:
    parsed = [(parse_time(str(row["occurred_at"])), row) for row in rows]
    if window == "current":
        selected = [row for _at, row in sorted(parsed, key=lambda item: (item[0], int(item[1].get("id") or 0)), reverse=True)[:50]]
    else:
        if window == "all":
            start, end = None, None
        elif window == "today":
            local = now.astimezone(dt.timezone(dt.timedelta(hours=8)))
            start = local.replace(hour=0, minute=0, second=0, microsecond=0).astimezone(UTC)
            end = start + dt.timedelta(days=1)
        else:
            days = {"1d": 1, "7d": 7, "30d": 30}[window]
            start, end = now - dt.timedelta(days=days), now
        selected = [
            row for at, row in parsed
            if start is None or (start <= at < end)
        ]
    total_count = len(selected)
    success_count = sum(1 for row in selected if is_success(row))
    failure_count = sum(1 for row in selected if is_terminal_failure(row))
    total_tokens = sum(int(row.get("total_tokens") or 0) for row in selected)
    total_cost = sum(numeric(row, "cost") for row in selected)
    non_success_cost = sum(numeric(row, "cost") for row in selected if is_terminal_failure(row))
    result = {
        "totalCount": total_count,
        "successCount": success_count,
        "failureCount": failure_count,
        "totalTokens": total_tokens,
        "totalCost": total_cost,
        "nonSuccessCost": non_success_cost,
    }
    if window in {"1d", "7d", "30d", "today"}:
        result["nonSuccessTokens"] = sum(
            int(row.get("total_tokens") or 0) for row in selected if is_terminal_failure(row)
        )
    return result


def compare(response_path: Path, expected_values: dict[str, Any], window: str) -> None:
    response = json.loads(response_path.read_text(encoding="utf-8"))
    if not isinstance(response, dict):
        raise RuntimeError(f"{window}: response is not a JSON object")
    missing = sorted(set(expected_values) - set(response))
    if missing:
        raise RuntimeError(f"{window}: response is missing exact fields: {','.join(missing)}")
    for key, expected_value in expected_values.items():
        actual = response[key]
        if isinstance(expected_value, float):
            if abs(float(actual) - expected_value) > 1e-9:
                raise RuntimeError(f"{window}: {key} expected {expected_value}, got {actual}")
        elif actual != expected_value:
            raise RuntimeError(f"{window}: {key} expected {expected_value}, got {actual}")
    print(
        f"summary-production-exactness=window={window} "
        f"count={expected_values['totalCount']} tokens={expected_values['totalTokens']}"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--archives", type=Path, required=True)
    parser.add_argument("--response", type=Path, required=True)
    parser.add_argument("--window", choices=("current", "1d", "7d", "30d", "today", "all"), required=True)
    parser.add_argument("--now", type=int, required=True)
    args = parser.parse_args()
    rows = load_rows(args.database, args.archives)
    compare(args.response, expected(rows, args.window, dt.datetime.fromtimestamp(args.now, UTC)), args.window)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
