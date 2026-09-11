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
import hashlib
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
        missing = ",".join(sorted(required - columns))
        raise RuntimeError(f"codex_invocations is missing required columns: {missing}")
    optional = [
        "error_message",
        "failure_kind",
        "failure_class",
        "is_actionable",
        "input_tokens",
        "output_tokens",
        "cache_input_tokens",
        "reasoning_tokens",
        "reasoning_effort",
        "model",
        "response_model",
        "upstream_account_id",
        "payload",
        "cost_input",
        "cost_cache_write",
        "cost_cache_read",
        "cost_output",
        "cost_reasoning",
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
            raise RuntimeError("V2 Snapshot payload contains a non-object record")
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
                "SELECT id, file_path, status, sha256 FROM archive_batches "
                "WHERE dataset = 'codex_invocations' AND status = 'completed'"
            ).fetchall()
            if "archive_batches" in tables
            else []
        )
        proof_batches: set[tuple[int, str]] = set()
        if "summary_archive_snapshot" in tables and "summary_archive_snapshot_v2_proof" in tables:
            proofs = connection.execute(
                "SELECT archive_batch_id, manifest_sha256, page_count, row_count, "
                "coverage_start, coverage_end, semantic_sha256 "
                "FROM summary_archive_snapshot_v2_proof"
            ).fetchall()
            for batch_id, manifest_sha, page_count, row_count, proof_start, proof_end, semantic_sha in proofs:
                pages = connection.execute(
                    "SELECT page_index, row_count, payload, coverage_start, coverage_end, "
                    "snapshot_sha256, payload_bytes, format_version "
                    "FROM summary_archive_snapshot "
                    "WHERE archive_batch_id = ? AND manifest_sha256 = ? AND format_version = 2 "
                    "ORDER BY page_index",
                    (batch_id, manifest_sha),
                ).fetchall()
                if len(pages) != int(page_count) or sum(int(page[1]) for page in pages) != int(row_count):
                    raise RuntimeError(
                        f"V2 final proof page metadata is incomplete for archive batch {batch_id}"
                    )
                semantic = hashlib.sha256()
                previous_end: dt.datetime | None = None
                previous_key: tuple[dt.datetime, int] | None = None
                seen_ids: set[int] = set()
                seen_invokes: set[str] = set()
                for expected_page, page in enumerate(pages):
                    page_index, page_row_count, payload, coverage_start, coverage_end, snapshot_sha, payload_bytes, format_version = page
                    if int(page_index) != expected_page or int(format_version) != 2:
                        raise RuntimeError("V2 final proof page order or format is invalid")
                    if int(payload_bytes) != len(payload) or hashlib.sha256(payload).hexdigest() != snapshot_sha:
                        raise RuntimeError("V2 final proof page integrity is invalid")
                    start = parse_time(str(coverage_start))
                    end = parse_time(str(coverage_end))
                    if start > end or (previous_end is not None and start < previous_end):
                        raise RuntimeError("V2 final proof page coverage is invalid")
                    semantic.update(int(page_index).to_bytes(8, "little", signed=True))
                    semantic.update(str(snapshot_sha).encode())
                    semantic.update(int(page_row_count).to_bytes(8, "little", signed=True))
                    semantic.update(str(coverage_start).encode())
                    semantic.update(str(coverage_end).encode())
                    decoded = decode_v2_payload(payload)
                    if len(decoded) != int(page_row_count):
                        raise RuntimeError("V2 final proof payload row count is invalid")
                    for record in decoded:
                        occurred = parse_time(str(record["occurred_at"]))
                        key = (occurred, int(record.get("id") or 0))
                        if previous_key is not None and key < previous_key:
                            raise RuntimeError("V2 final proof record order is invalid")
                        if int(record.get("id") or 0) in seen_ids or str(record.get("invoke_id")) in seen_invokes:
                            raise RuntimeError("V2 final proof contains duplicate identity")
                        if occurred < start or occurred > end:
                            raise RuntimeError("V2 final proof record coverage is invalid")
                        seen_ids.add(int(record.get("id") or 0))
                        seen_invokes.add(str(record.get("invoke_id")))
                        previous_key = key
                    previous_end = end
                    rows.extend(decoded)
                if str(proof_start) != str(pages[0][3]) or str(proof_end) != str(pages[-1][4]) or semantic.hexdigest() != str(semantic_sha):
                    raise RuntimeError("V2 final proof semantic coverage is invalid")
                proof_batches.add((int(batch_id), str(manifest_sha)))
        for _batch_id, stored_path, _status, stored_sha in batches:
            if (int(_batch_id), str(stored_sha)) in proof_batches:
                continue
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
        previous = deduplicated.get(identity)
        if previous is not None and not canonical_rows_equal(previous, row):
            raise RuntimeError(f"conflicting authoritative rows for {identity!r}")
        deduplicated[identity] = row
    return list(deduplicated.values())


def canonical_row(row: dict[str, Any]) -> tuple[Any, ...]:
    """Return the fields whose disagreement would change Summary exactness."""
    fields = (
        "id",
        "invoke_id",
        "occurred_at",
        "source",
        "status",
        "total_tokens",
        "cost",
        "input_tokens",
        "output_tokens",
        "cache_input_tokens",
        "reasoning_tokens",
        "model",
        "response_model",
        "reasoning_effort",
        "upstream_account_id",
        "error_message",
        "failure_kind",
        "failure_class",
        "is_actionable",
        "cost_input",
        "cost_cache_write",
        "cost_cache_read",
        "cost_output",
        "cost_reasoning",
    )
    return tuple(row.get(field) for field in fields)


def canonical_rows_equal(left: dict[str, Any], right: dict[str, Any]) -> bool:
    if canonical_row(left) != canonical_row(right):
        return False
    # V2 intentionally omits raw payload text. When both authorities retain payload, compare
    # the payload markers that affect failure classification; an absent V2 payload is not itself
    # a disagreement.
    for key in ("failureKind", "downstreamErrorMessage"):
        left_value = payload_text(left, key)
        right_value = payload_text(right, key)
        if left_value and right_value and left_value != right_value:
            return False
    return True


def payload_text(row: dict[str, Any], key: str) -> str:
    payload = row.get("payload")
    if not payload:
        return ""
    try:
        value = json.loads(payload) if isinstance(payload, str) else payload
    except (TypeError, json.JSONDecodeError):
        return ""
    candidate = value.get(key) if isinstance(value, dict) else None
    return str(candidate or "").strip().lower()


def resolved_failure_class(row: dict[str, Any]) -> str:
    status = str(row.get("status") or "").strip().lower()
    error = str(row.get("error_message") or "").strip().lower()
    failure_kind = payload_text(row, "failureKind") or str(row.get("failure_kind") or "").strip().lower()
    explicit = str(row.get("failure_class") or "").strip().lower()
    if explicit in {"service_failure", "client_failure", "client_abort"}:
        return explicit
    downstream = payload_text(row, "downstreamErrorMessage")
    if status in {"success", "completed"} and not error and not downstream and not failure_kind:
        return "none"
    if status == "warning_success" and not error and failure_kind == "downstream_closed":
        return "none"
    if status in {"running", "pending"} and not error:
        return "none"
    if not status and not error and not downstream and not failure_kind:
        return "none"
    if failure_kind == "downstream_closed" or error.startswith("[downstream_closed]") or "downstream closed while streaming upstream response" in error or "downstream closed while streaming upstream response" in downstream:
        return "client_abort"
    if status == "http_429" or failure_kind == "upstream_http_429":
        return "service_failure"
    if failure_kind in {"request_body_stream_error_client_closed", "invalid_api_key", "api_key_not_found", "api_key_missing"} or status.startswith("http_4") and status != "http_429":
        return "client_failure"
    if failure_kind in {"failed_contact_upstream", "upstream_response_failed", "upstream_stream_error", "request_body_read_timeout", "upstream_handshake_timeout"} or status.startswith("http_5"):
        return "service_failure"
    if status in SUCCESS_STATUSES:
        return "none"
    if status == "http_200" and not error and not downstream and not failure_kind:
        return "none"
    return "service_failure"


def is_success(row: dict[str, Any]) -> bool:
    return resolved_failure_class(row) == "none" and str(row.get("status") or "").strip().lower() not in {"running", "pending"}


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
    if window != "current":
        result["usageBreakdown"] = usage_breakdown(selected)
    if window in {"1d", "7d", "30d", "today"}:
        result["nonSuccessTokens"] = sum(
            int(row.get("total_tokens") or 0) for row in selected if is_terminal_failure(row)
        )
    return result


def usage_breakdown(rows: list[dict[str, Any]]) -> dict[str, Any]:
    model_groups: dict[tuple[str, str | None], dict[str, Any]] = {}
    totals = {
        "cacheWriteTokens": 0,
        "cacheReadTokens": 0,
        "outputTokens": 0,
    }
    total_costs = {key: 0.0 for key in ("input", "cache_write", "cache_read", "output", "reasoning", "unknown")}
    has_cost = False
    for row in rows:
        cache_read = max(int(row.get("cache_input_tokens") or 0), 0)
        cache_write = max(int(row.get("input_tokens") or 0) - cache_read, 0)
        output = max(int(row.get("output_tokens") or 0), 0)
        totals["cacheWriteTokens"] += cache_write
        totals["cacheReadTokens"] += cache_read
        totals["outputTokens"] += output
        model = payload_text(row, "responseModel") or str(row.get("response_model") or row.get("model") or "unknown").strip() or "unknown"
        reasoning = payload_text(row, "reasoningEffort") or (str(row["reasoning_effort"]).strip().lower() if row.get("reasoning_effort") else None)
        group = model_groups.setdefault(
            (model, reasoning),
            {"model": model, "reasoningEffort": reasoning, "cacheWriteTokens": 0, "cacheReadTokens": 0, "outputTokens": 0, "costs": None},
        )
        group["cacheWriteTokens"] += cache_write
        group["cacheReadTokens"] += cache_read
        group["outputTokens"] += output
        cost = row.get("cost")
        if cost is not None:
            has_cost = True
            cost_fields = [row.get(key) for key in ("cost_input", "cost_cache_write", "cost_cache_read", "cost_output", "cost_reasoning")]
            if all(value is not None for value in cost_fields):
                for target, value in zip(("input", "cache_write", "cache_read", "output", "reasoning"), cost_fields, strict=True):
                    total_costs[target] += float(value or 0)
            else:
                total_costs["unknown"] += float(cost or 0)
            group_costs = group["costs"] or {key: 0.0 for key in ("input", "cacheWrite", "cacheRead", "output", "reasoning", "unknown")}
            if all(value is not None for value in cost_fields):
                for target, value in zip(("input", "cacheWrite", "cacheRead", "output", "reasoning"), cost_fields, strict=True):
                    group_costs[target] += float(value or 0)
            else:
                group_costs["unknown"] += float(cost or 0)
            group["costs"] = group_costs
    for group in model_groups.values():
        if group["costs"] is None:
            group.pop("costs")
        if group.get("reasoningEffort") is None:
            group.pop("reasoningEffort", None)
    result = dict(totals)
    if has_cost:
        result["costs"] = {
            "input": total_costs["input"],
            "cacheWrite": total_costs["cache_write"],
            "cacheRead": total_costs["cache_read"],
            "output": total_costs["output"],
            "reasoning": total_costs["reasoning"],
            "unknown": total_costs["unknown"],
        }
    result["models"] = []
    for key in sorted(model_groups, key=lambda item: (item[0], item[1] or "")):
        entry = model_groups[key]
        if any(
            entry.get(field, 0)
            for field in ("cacheWriteTokens", "cacheReadTokens", "outputTokens")
        ) or entry.get("costs"):
            result["models"].append(entry)
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
