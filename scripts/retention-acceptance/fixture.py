"""Build the deterministic disposable SQLite input used by retention acceptance."""

from __future__ import annotations

from datetime import date, datetime, timedelta, timezone
import hashlib
import json
from pathlib import Path
import sqlite3
from typing import Any, Callable, Iterable

from contract import COMPETITOR_FILE_COUNT, COMPETITOR_TASK, DATASETS, RETENTION_DAYS, SEED, canonical_json


CONTROLLED_TASK_KEYS = tuple(
    [DATASETS[name]["task_key"] for name in DATASETS] + [COMPETITOR_TASK]
)


def shanghai_cutoff(now: datetime | None = None) -> str:
    shanghai = timezone(timedelta(hours=8))
    current = now or datetime.now(timezone.utc)
    local_day = current.astimezone(shanghai).date()
    return f"{local_day - timedelta(days=RETENTION_DAYS):%Y-%m-%d} 00:00:00"


def _previous_month(year: int, month: int, offset: int) -> tuple[int, int]:
    month_index = year * 12 + month - 1 - offset
    return month_index // 12, month_index % 12 + 1


def _history_timestamp(cutoff: str, index: int) -> str:
    cutoff_day = date.fromisoformat(cutoff[:10])
    month_offset = 1 + (index % 2)
    year, month = _previous_month(cutoff_day.year, cutoff_day.month, month_offset)
    first = date(year, month, 1)
    next_month_year, next_month = _previous_month(year, month, -1)
    last_day = (date(next_month_year, next_month, 1) - timedelta(days=1)).day
    day = 1 + (index * 17 + index // 97) % last_day
    second = (index * 7919) % 86_400
    return f"{first.replace(day=day):%Y-%m-%d}T{second // 3600:02d}:{second // 60 % 60:02d}:{second % 60:02d}.000Z"


def _payload(dataset: str, index: int) -> str:
    body = {
        "request": {
            "model": "gpt-5-codex",
            "input": [{"role": "user", "content": f"acceptance fixture {dataset} {index:06d}"}],
            "metadata": {"tenant": f"tenant-{index % 32:02d}", "seed": SEED},
        },
        "usage": {"input_tokens": 640 + index % 512, "output_tokens": 96 + index % 256},
    }
    return json.dumps(body, sort_keys=True, separators=(",", ":"))


def _record_fingerprint(hasher: Any, dataset: str, identity: str, occurred_at: str, payload: str) -> None:
    hasher.update(canonical_json([dataset, identity, occurred_at, payload]))
    hasher.update(b"\n")


def _insert_many(connection: sqlite3.Connection, table: str, rows: Iterable[dict[str, Any]]) -> None:
    batch = list(rows)
    if not batch:
        return
    columns = list(batch[0])
    sql = f"INSERT INTO {table} ({','.join(columns)}) VALUES ({','.join('?' for _ in columns)})"
    connection.executemany(sql, (tuple(row[column] for column in columns) for row in batch))


def _invocation_record(identity: str, occurred_at: str, payload: str) -> dict[str, Any]:
    return {
        "invoke_id": identity,
        "occurred_at": occurred_at,
        "source": "xy",
        "model": "gpt-5-codex",
        "input_tokens": 640,
        "output_tokens": 96,
        "total_tokens": 736,
        "status": "success",
        "payload": payload,
        "raw_response": json.dumps({"id": identity, "status": "completed", "output": []}, separators=(",", ":")),
    }


def _attempt_record(identity: str, occurred_at: str, payload: str, index: int) -> dict[str, Any]:
    return {
        "attempt_public_id": identity,
        "invoke_id": identity,
        "occurred_at": occurred_at,
        "endpoint": "/v1/responses",
        "route_mode": "sticky",
        "sticky_key": "retention-hot-key" if index % 10 else f"retention-key-{index % 97:03d}",
        "routing_source": "retention-acceptance",
        "request_model": "gpt-5-codex",
        "attempt_index": 1,
        "distinct_account_index": 1,
        "same_account_retry_index": 0,
        "started_at": occurred_at,
        "finished_at": occurred_at,
        "status": "success",
        "request_summary_json": payload,
        "response_summary_json": json.dumps({"status": "completed", "usage": {"total_tokens": 736}}, separators=(",", ":")),
    }


def _existing_columns(connection: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in connection.execute(f"PRAGMA table_info('{table}')")}


def _filter_record(connection: sqlite3.Connection, table: str, record: dict[str, Any], columns: set[str] | None = None) -> dict[str, Any]:
    columns = columns or _existing_columns(connection, table)
    selected = {key: value for key, value in record.items() if key in columns}
    required = {
        "codex_invocations": {"invoke_id", "occurred_at", "raw_response"},
        "pool_upstream_request_attempts": {
            "attempt_public_id", "invoke_id", "occurred_at", "endpoint", "route_mode", "attempt_index",
            "distinct_account_index", "same_account_retry_index", "status",
        },
    }[table]
    missing = required - columns
    if missing:
        raise RuntimeError(f"production table {table} is missing required columns: {sorted(missing)}")
    return selected


def populate_template(database: Path, cutoff: str, budget_check: Callable[[], float] | None = None) -> str:
    hasher = hashlib.sha256()
    timeout = min(30.0, budget_check()) if budget_check is not None else 30.0
    connection = sqlite3.connect(database, timeout=timeout)
    connection.execute(f"PRAGMA busy_timeout={int(timeout * 1000)}")
    try:
        connection.execute("BEGIN IMMEDIATE")
        for name, table in (("invocations", "codex_invocations"), ("pool_upstream_attempts", "pool_upstream_request_attempts")):
            count = DATASETS[name]["seed_rows"]
            columns = _existing_columns(connection, table)
            for start in range(1, count + 1, 500):
                if budget_check is not None:
                    budget_check()
                rows = []
                for index in range(start, min(count + 1, start + 500)):
                    identity = f"retention-seed-{name}-{index:06d}"
                    occurred_at = _history_timestamp(cutoff, index - 1)
                    payload = _payload(name, index)
                    record = _invocation_record(identity, occurred_at, payload) if name == "invocations" else _attempt_record(identity, occurred_at, payload, index)
                    rows.append(_filter_record(connection, table, record, columns))
                    _record_fingerprint(hasher, name, identity, occurred_at, payload)
                _insert_many(connection, table, rows)
        connection.commit()
    except BaseException:
        connection.rollback()
        raise
    finally:
        connection.close()
    return hasher.hexdigest()


def prepare_competitor(
    main_database: Path,
    maintenance_database: Path,
    raw_directory: Path,
    container_raw_directory: str,
    count: int = COMPETITOR_FILE_COUNT,
    budget_check: Callable[[], float] | None = None,
) -> None:
    raw_directory.mkdir(parents=True, exist_ok=True)
    timeout = min(30.0, budget_check()) if budget_check is not None else 30.0
    main = sqlite3.connect(main_database, timeout=timeout)
    maintenance = sqlite3.connect(maintenance_database, timeout=timeout)
    try:
        if "retention_raw_reconciliation" not in {row[0] for row in main.execute("SELECT name FROM sqlite_master WHERE type='table'")}:
            raise RuntimeError("production raw-orphan reconciliation table is missing")
        for index in range(count):
            if budget_check is not None and index % 32 == 0:
                budget_check()
            name = f"retention-orphan-{index:04d}.bin"
            path = raw_directory / name
            path.write_bytes(hashlib.sha256(name.encode()).digest() * 8)
            stat = path.stat()
            identity = f"{stat.st_dev}:{stat.st_ino}:{stat.st_mtime_ns // 1_000_000_000}:{stat.st_mtime_ns % 1_000_000_000}:{stat.st_ctime_ns // 1_000_000_000}:{stat.st_ctime_ns % 1_000_000_000}"
            container_path = str(Path(container_raw_directory) / name)
            main.execute(
                "INSERT INTO retention_raw_reconciliation(raw_path,file_identity,byte_size,quarantined_at,release_pending,removal_evidence_persisted) VALUES(?,?,?,?,0,0)",
                (container_path, identity, stat.st_size, "2000-01-01T00:00:00Z"),
            )
        main.commit()
        table_names = {row[0] for row in maintenance.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        if "managed_task_runs" not in table_names or "managed_tasks" not in table_names:
            raise RuntimeError("production maintenance dispatcher tables are missing")
        for table in ("managed_task_work_runs", "managed_task_runs"):
            if table in table_names:
                maintenance.execute(f"DELETE FROM {table}")
        now = datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")
        placeholders = ",".join("?" for _ in CONTROLLED_TASK_KEYS)
        maintenance.execute(
            f"UPDATE managed_tasks "
            f"SET enabled=0,next_trigger_at=NULL,next_catchup_at=NULL,catchup_reason=NULL,updated_at=? "
            f"WHERE task_key NOT IN ({placeholders})",
            (now, *CONTROLLED_TASK_KEYS),
        )
        task_keys = CONTROLLED_TASK_KEYS
        for key in task_keys:
            cursor = maintenance.execute(
                "UPDATE managed_tasks SET next_trigger_at=?,next_catchup_at=NULL,updated_at=? WHERE task_key=? AND enabled!=0",
                (now, now, key),
            )
            if cursor.rowcount != 1:
                raise RuntimeError(f"production managed task {key} is missing or disabled")
        maintenance.commit()
    except BaseException:
        main.rollback()
        maintenance.rollback()
        raise
    finally:
        main.close()
        maintenance.close()
