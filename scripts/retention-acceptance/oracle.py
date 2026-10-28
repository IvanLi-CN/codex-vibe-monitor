"""Independently reconcile live SQLite rows with completed production archives."""

from __future__ import annotations

from collections import Counter
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import re
import sqlite3
import stat
import subprocess
import tempfile
from typing import Any, Callable, Iterator

from contract import AcceptanceError, DATASETS, canonical_json, parse_time, sha256_json


REPLAY_TARGETS = {
    "invocation_rollup_hourly",
    "upstream_account_stats_hourly",
    "upstream_account_usage_breakdown_hourly",
}
CONTAINER_ARCHIVE_ROOT = Path("/srv/app/data/archives")
SUMMARY_PAGE_MAX_BYTES = 64 * 1024 * 1024
SUMMARY_PAGE_MAX_RECORDS = 400


def _columns(connection: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in connection.execute(f"PRAGMA table_info('{table}')")}


def _table_columns(connection: sqlite3.Connection, table: str) -> list[str]:
    return [row[1] for row in connection.execute(f"PRAGMA table_xinfo('{table}')") if row[6] != 1]


def _row_digest(row: tuple[Any, ...]) -> str:
    values = [{"blob": value.hex()} if isinstance(value, bytes) else value for value in row]
    return hashlib.sha256(canonical_json(values)).hexdigest()


def _hash_handle(handle: Any, budget_check: Callable[[], float] | None = None) -> str:
    digest = hashlib.sha256()
    handle.seek(0)
    for index, chunk in enumerate(iter(lambda: handle.read(1024 * 1024), b"")):
        if budget_check is not None and index % 8 == 0:
            budget_check()
        digest.update(chunk)
    return digest.hexdigest()


def _file_signature(metadata: os.stat_result) -> tuple[int, int, int, int, int, int, int]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_mode,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
        metadata.st_nlink,
    )


def _path_signature(path: Path) -> tuple[int, int, int, int, int, int, int]:
    return _file_signature(path.stat(follow_symlinks=False))


def _assert_archive_unchanged(
    path: Path,
    source: Any,
    initial_signature: tuple[int, int, int, int, int, int, int],
    expected_sha: str,
    dataset: str,
    budget_check: Callable[[], float] | None,
) -> None:
    source_signature = _file_signature(os.fstat(source.fileno()))
    try:
        path_signature = _path_signature(path)
    except OSError as error:
        raise AcceptanceError(f"{dataset} archive path changed during verification") from error
    if source_signature != initial_signature or path_signature != initial_signature:
        raise AcceptanceError(f"{dataset} archive identity or metadata changed during verification")
    if _hash_handle(source, budget_check) != expected_sha:
        raise AcceptanceError(f"{dataset} archive bytes changed during verification")
    if _file_signature(os.fstat(source.fileno())) != initial_signature:
        raise AcceptanceError(f"{dataset} archive changed while its source checksum was rechecked")
    try:
        final_path_signature = _path_signature(path)
    except OSError as error:
        raise AcceptanceError(f"{dataset} archive path changed during verification") from error
    if final_path_signature != initial_signature:
        raise AcceptanceError(f"{dataset} archive path identity changed during verification")


@contextmanager
def _verified_archive_snapshot(
    path: Path,
    expected_sha: str,
    scratch: Path,
    dataset: str,
    budget_check: Callable[[], float] | None = None,
) -> Iterator[Any]:
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    source = os.fdopen(os.open(path, flags), "rb")
    with source:
        initial_metadata = os.fstat(source.fileno())
        if not stat.S_ISREG(initial_metadata.st_mode):
            raise AcceptanceError(f"{dataset} archive is not a regular file")
        initial_signature = _file_signature(initial_metadata)
        try:
            if _path_signature(path) != initial_signature:
                raise AcceptanceError(f"{dataset} archive path identity changed before verification")
        except OSError as error:
            raise AcceptanceError(f"{dataset} archive path changed before verification") from error

        copied_digest = hashlib.sha256()
        with tempfile.TemporaryFile(mode="w+b", dir=scratch) as snapshot:
            for index, chunk in enumerate(iter(lambda: source.read(1024 * 1024), b"")):
                if budget_check is not None and index % 8 == 0:
                    budget_check()
                snapshot.write(chunk)
                copied_digest.update(chunk)
            snapshot.flush()
            if copied_digest.hexdigest() != expected_sha:
                raise AcceptanceError(f"{dataset} archive checksum does not match its manifest")
            _assert_archive_unchanged(path, source, initial_signature, expected_sha, dataset, budget_check)

            snapshot_signature = _file_signature(os.fstat(snapshot.fileno()))
            snapshot.seek(0)
            try:
                yield snapshot
            finally:
                if _file_signature(os.fstat(snapshot.fileno())) != snapshot_signature:
                    raise AcceptanceError(f"{dataset} private archive snapshot changed while being read")
                if _hash_handle(snapshot, budget_check) != expected_sha:
                    raise AcceptanceError(f"{dataset} private archive snapshot checksum changed while being read")
                _assert_archive_unchanged(path, source, initial_signature, expected_sha, dataset, budget_check)


def _archive_path(
    database: Path,
    archive_root: Path,
    stored_path: str,
    dataset: str,
    month_key: str,
    *,
    day_key: str | None = None,
    part_key: str | None = None,
    layout: str | None = None,
    codec: str | None = None,
) -> Path:
    path = Path(stored_path)
    resolved_root = archive_root.resolve(strict=True)
    if path.is_absolute():
        try:
            resolved = path.resolve(strict=True)
        except OSError:
            resolved = None
        if resolved is not None:
            try:
                relative = resolved.relative_to(resolved_root)
            except ValueError:
                relative = None
            if relative is None:
                try:
                    relative = path.relative_to(CONTAINER_ARCHIVE_ROOT)
                except ValueError as error:
                    raise AcceptanceError("archive manifest escaped the configured archive root") from error
                resolved = (resolved_root / relative).resolve(strict=True)
        else:
            try:
                relative = path.relative_to(CONTAINER_ARCHIVE_ROOT)
            except ValueError as error:
                raise AcceptanceError("archive manifest escaped the configured archive root") from error
            resolved = (resolved_root / relative).resolve(strict=True)
    else:
        resolved = (database.parent / path).resolve(strict=True)
    try:
        relative = resolved.relative_to(resolved_root)
    except ValueError as error:
        raise AcceptanceError("archive manifest escaped the configured archive root") from error
    if not re.fullmatch(r"\d{4}-(0[1-9]|1[0-2])", month_key):
        raise AcceptanceError("archive manifest did not resolve to the canonical dataset/year/month path")
    if codec is not None and codec != "gzip":
        raise AcceptanceError("archive manifest uses an unsupported archive codec")

    parts = relative.parts
    expected_month_path = (dataset, month_key[:4], f"{dataset}-{month_key}.sqlite.gz")
    if parts == expected_month_path:
        if layout is not None and layout != "legacy_month":
            raise AcceptanceError("legacy archive path has a non-legacy manifest layout")
        if day_key is not None or part_key is not None:
            raise AcceptanceError("legacy archive path has segment manifest identity")
    elif len(parts) == 5 and parts[0] == dataset:
        _, year, month, day, filename = parts
        actual_day_key = f"{year}-{month}-{day}"
        if (
            not re.fullmatch(r"\d{4}", year)
            or not re.fullmatch(r"(?:0[1-9]|1[0-2])", month)
            or not re.fullmatch(r"(?:0[1-9]|[12]\d|3[01])", day)
            or f"{year}-{month}" != month_key
            or not filename.startswith("part-")
            or not filename.endswith(".sqlite.gz")
        ):
            raise AcceptanceError("archive manifest did not resolve to the canonical dataset/year/month path")
        if layout is not None and layout != "segment_v1":
            raise AcceptanceError("segment archive path has a non-segment manifest layout")
        if day_key is not None and day_key != actual_day_key:
            raise AcceptanceError("segment archive day identity disagrees with its manifest path")
        actual_part_key = filename.removesuffix(".sqlite.gz")
        if part_key is not None and part_key != actual_part_key:
            raise AcceptanceError("segment archive part identity disagrees with its manifest path")
    else:
        raise AcceptanceError("archive manifest did not resolve to the canonical dataset/year/month path")
    if resolved.suffixes[-2:] != [".sqlite", ".gz"]:
        raise AcceptanceError("archive artifact is not a gzip SQLite file")
    return resolved


def _decode_summary_payload(payload: bytes, budget_check: Callable[[], float] | None) -> list[dict[str, Any]]:
    if not payload.startswith(b"\x28\xb5\x2f\xfd"):
        raise AcceptanceError("Summary Snapshot V2 payload is not a zstd frame")
    timeout = min(30.0, budget_check()) if budget_check is not None else 30.0
    try:
        with tempfile.TemporaryFile() as decoded:
            subprocess.run(
                ["zstd", "-q", "-d", "-c"],
                input=payload,
                stdout=decoded,
                stderr=subprocess.PIPE,
                check=True,
                timeout=timeout,
            )
            decoded.seek(0)
            raw = decoded.read(SUMMARY_PAGE_MAX_BYTES + 1)
    except (OSError, subprocess.SubprocessError) as error:
        raise AcceptanceError("Summary Snapshot V2 payload could not be decoded with zstd") from error
    if len(raw) > SUMMARY_PAGE_MAX_BYTES:
        raise AcceptanceError("Summary Snapshot V2 decoded payload exceeds its byte budget")

    def json_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON field")
            result[key] = value
        return result

    def invalid_constant(value: str) -> Any:
        raise ValueError(f"invalid JSON constant: {value}")

    try:
        records = json.loads(raw.decode("utf-8"), object_pairs_hook=json_object, parse_constant=invalid_constant)
    except (ValueError, UnicodeDecodeError, RecursionError) as error:
        raise AcceptanceError("Summary Snapshot V2 payload is not valid JSON") from error
    if not isinstance(records, list) or not 0 < len(records) <= SUMMARY_PAGE_MAX_RECORDS:
        raise AcceptanceError("Summary Snapshot V2 payload must contain a nonempty bounded record list")
    for record in records:
        if not isinstance(record, dict):
            raise AcceptanceError("Summary Snapshot V2 payload contains a malformed record")
        for field in ("id", "inputTokens", "outputTokens", "cacheInputTokens", "reasoningTokens", "totalTokens"):
            if not isinstance(record.get(field), int) or isinstance(record[field], bool) or not -(2**63) <= record[field] < 2**63:
                raise AcceptanceError(f"Summary Snapshot V2 record has an invalid {field}")
        for field in ("invokeId", "occurredAt", "source", "status"):
            if not isinstance(record.get(field), str) or not record[field].strip():
                raise AcceptanceError(f"Summary Snapshot V2 record has an invalid {field}")
        if not isinstance(record.get("isActionable"), bool):
            raise AcceptanceError("Summary Snapshot V2 record has an invalid isActionable")
        for field in ("model", "responseModel", "reasoningEffort", "errorMessage", "failureKind", "failureClass"):
            if record.get(field) is not None and not isinstance(record[field], str):
                raise AcceptanceError(f"Summary Snapshot V2 record has an invalid {field}")
        account = record.get("upstreamAccountId")
        if account is not None and (not isinstance(account, int) or isinstance(account, bool) or not -(2**63) <= account < 2**63):
            raise AcceptanceError("Summary Snapshot V2 record has an invalid upstreamAccountId")
        for field in ("cost", "costInput", "costCacheWrite", "costCacheRead", "costOutput", "costReasoning"):
            value = record.get(field)
            if value is None:
                continue
            try:
                valid = isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)
            except OverflowError:
                valid = False
            if not valid:
                raise AcceptanceError(f"Summary Snapshot V2 record has an invalid {field}")
    return records


def _summary_time(value: Any) -> datetime:
    if not isinstance(value, str) or not value:
        raise AcceptanceError("Summary Snapshot V2 coverage timestamp is missing")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise AcceptanceError("Summary Snapshot V2 coverage timestamp is invalid") from error
    # Production archives also use SQLite's UTC timestamps without an explicit offset.
    return parsed.replace(tzinfo=timezone.utc) if parsed.tzinfo is None else parsed.astimezone(timezone.utc)


def _summary_record_from_archive_row(columns: list[str], row: tuple[Any, ...]) -> dict[str, Any]:
    values = dict(zip(columns, row, strict=True))
    payload: dict[str, Any] = {}
    raw_payload = values.get("payload")
    if isinstance(raw_payload, str):
        try:
            parsed_payload = json.loads(raw_payload)
        except (ValueError, UnicodeDecodeError):
            parsed_payload = None
        if isinstance(parsed_payload, dict):
            payload = parsed_payload

    def payload_string(field: str) -> str | None:
        value = payload.get(field)
        return value if isinstance(value, str) else None

    model = values.get("model")
    if model is None:
        model = payload_string("model")
    upstream_account_id = values.get("upstream_account_id")
    if upstream_account_id is None:
        candidate = payload.get("upstreamAccountId")
        if isinstance(candidate, int) and not isinstance(candidate, bool):
            upstream_account_id = candidate

    return {
        "id": values["id"],
        "invokeId": values["invoke_id"],
        "occurredAt": values["occurred_at"],
        "source": values["source"],
        "model": model,
        "responseModel": payload_string("responseModel"),
        "inputTokens": values.get("input_tokens") if values.get("input_tokens") is not None else 0,
        "outputTokens": values.get("output_tokens") if values.get("output_tokens") is not None else 0,
        "cacheInputTokens": values.get("cache_input_tokens") if values.get("cache_input_tokens") is not None else 0,
        "reasoningTokens": values.get("reasoning_tokens") if values.get("reasoning_tokens") is not None else 0,
        "reasoningEffort": payload_string("reasoningEffort"),
        "totalTokens": values.get("total_tokens") if values.get("total_tokens") is not None else 0,
        "cost": values.get("cost"),
        "costInput": values.get("cost_input"),
        "costCacheWrite": values.get("cost_cache_write"),
        "costCacheRead": values.get("cost_cache_read"),
        "costOutput": values.get("cost_output"),
        "costReasoning": values.get("cost_reasoning"),
        "status": values["status"] if values.get("status") is not None else "unknown",
        "errorMessage": values.get("error_message"),
        "failureKind": values.get("failure_kind"),
        "failureClass": values.get("failure_class"),
        "isActionable": bool(values.get("is_actionable") or 0),
        "upstreamAccountId": upstream_account_id,
    }


def _verify_summary_proof(
    connection: sqlite3.Connection,
    archive_id: int,
    manifest_sha: str,
    file_path: str,
    *,
    archive: sqlite3.Connection,
    manifest_row_count: int,
    budget_check: Callable[[], float] | None = None,
) -> None:
    proof = connection.execute(
        "SELECT page_count,row_count,coverage_start,coverage_end,semantic_sha256 FROM summary_archive_snapshot_v2_proof WHERE archive_batch_id=? AND manifest_sha256=?",
        (archive_id, manifest_sha),
    ).fetchone()
    if proof is None:
        raise AcceptanceError("invocation archive is missing its final Summary Snapshot V2 proof")
    pages = connection.execute(
        "SELECT page_index,coverage_start,coverage_end,row_count,payload,payload_bytes,snapshot_sha256,format_version FROM summary_archive_snapshot WHERE archive_batch_id=? AND manifest_sha256=? ORDER BY page_index",
        (archive_id, manifest_sha),
    ).fetchall()
    if not pages or len(pages) != proof[0]:
        raise AcceptanceError("Summary Snapshot V2 proof page count does not match stored pages")
    semantic = hashlib.sha256()
    total_rows = 0
    archive_columns = _table_columns(archive, "codex_invocations")
    archived_rows = iter(archive.execute("SELECT * FROM codex_invocations ORDER BY julianday(occurred_at),id"))
    seen_ids: set[int] = set()
    seen_invocations: set[str] = set()
    previous_key: tuple[datetime, int] | None = None
    previous_end: datetime | None = None
    for index, page in enumerate(pages):
        if budget_check is not None:
            budget_check()
        page_index, start, end, row_count, payload, payload_bytes, page_sha, version = page
        if not isinstance(payload, bytes) or not 0 < len(payload) <= SUMMARY_PAGE_MAX_BYTES:
            raise AcceptanceError("Summary Snapshot page payload is missing or exceeds its byte budget")
        payload = bytes(payload)
        if page_index != index or version != 2 or payload_bytes != len(payload):
            raise AcceptanceError("Summary Snapshot page order, format, or byte count is invalid")
        if hashlib.sha256(payload).hexdigest() != page_sha or row_count <= 0 or not start or not end:
            raise AcceptanceError("Summary Snapshot page checksum or coverage is invalid")
        records = _decode_summary_payload(payload, budget_check)
        if len(records) != row_count:
            raise AcceptanceError("Summary Snapshot V2 payload row count does not match its page")
        coverage_start, coverage_end = _summary_time(start), _summary_time(end)
        if (
            coverage_start > coverage_end
            or (previous_end is not None and coverage_start < previous_end)
            or coverage_start != _summary_time(records[0]["occurredAt"])
            or coverage_end != _summary_time(records[-1]["occurredAt"])
        ):
            raise AcceptanceError("Summary Snapshot V2 page coverage does not match its payload records")
        for record in records:
            identity, invoke_id, occurred_at = record["id"], record["invokeId"], record["occurredAt"]
            key = (_summary_time(occurred_at), identity)
            if identity in seen_ids or invoke_id in seen_invocations:
                raise AcceptanceError("Summary Snapshot V2 payload contains a duplicate archive row identity")
            if (previous_key is not None and key <= previous_key) or not coverage_start <= key[0] <= coverage_end:
                raise AcceptanceError("Summary Snapshot V2 record order or coverage is invalid")
            archived_row = next(archived_rows, None)
            if archived_row is None or _summary_record_from_archive_row(archive_columns, tuple(archived_row)) != record:
                raise AcceptanceError("Summary Snapshot V2 payload differs from actual archived codex_invocations row content")
            seen_ids.add(identity)
            seen_invocations.add(invoke_id)
            previous_key = key
        previous_end = coverage_end
        semantic.update(int(page_index).to_bytes(8, "little", signed=True))
        semantic.update(page_sha.encode())
        semantic.update(int(row_count).to_bytes(8, "little", signed=True))
        semantic.update(start.encode())
        semantic.update(end.encode())
        total_rows += row_count
    if proof[1] != total_rows or proof[2] != pages[0][1] or proof[3] != pages[-1][2] or proof[4] != semantic.hexdigest():
        raise AcceptanceError("Summary Snapshot V2 semantic proof does not match its pages")
    if total_rows != manifest_row_count or next(archived_rows, None) is not None:
        raise AcceptanceError("Summary Snapshot V2 payload count does not cover the actual archived codex_invocations rows")
    replayed = {
        row[0]
        for row in connection.execute(
            "SELECT target FROM hourly_rollup_archive_replay WHERE dataset='codex_invocations' AND file_path=? AND archive_sha256=?",
            (file_path, manifest_sha),
        )
    }
    if not REPLAY_TARGETS.issubset(replayed):
        raise AcceptanceError("invocation archive is missing one or more required hash-bound hourly replay proofs")


def verify_dataset(
    database: Path,
    archive_root: Path,
    dataset_name: str,
    expected_ids: set[str],
    seed_count: int,
    added_count: int,
    scratch: Path,
    *,
    template_database: Path,
    seed_ids: set[str],
    added_attempt_invocations: dict[str, str],
    cutoff: str,
    budget_check: Callable[[], float] | None = None,
    require_summary_proof: bool = True,
) -> dict[str, Any]:
    dataset = DATASETS[dataset_name]["manifest_dataset"]
    table = DATASETS[dataset_name]["table"]
    identity_column = DATASETS[dataset_name]["identity_column"]
    added_ids = expected_ids - seed_ids
    if (
        seed_count < 0
        or added_count < 0
        or len(seed_ids) != seed_count
        or len(added_ids) != added_count
        or len(expected_ids) != seed_count + added_count
    ):
        raise AcceptanceError(f"{dataset} expected identity set disagrees with its reported input counts")
    if dataset == "pool_upstream_request_attempts":
        if set(added_attempt_invocations) != added_ids:
            raise AcceptanceError("measured upstream attempt-to-invocation mapping is incomplete")
    elif added_attempt_invocations:
        raise AcceptanceError(f"{dataset} received an unexpected upstream attempt mapping")
    scratch.mkdir(parents=True, exist_ok=True)
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    template = sqlite3.connect(f"file:{template_database}?mode=ro", uri=True)
    archive_counts: Counter[str] = Counter()
    content_records: list[tuple[str, str, str]] = []
    summary_proof_verified = True
    try:
        source_columns = _table_columns(connection, table)
        template_columns = _table_columns(template, table)
        if not source_columns or source_columns != template_columns:
            raise AcceptanceError(f"{dataset} pristine template schema differs from the trial source schema")
        template_column_list = ",".join(template_columns)
        identity_index = template_columns.index(identity_column)
        seed_fingerprints: dict[str, str] = {}
        for index, row in enumerate(template.execute(f"SELECT {template_column_list} FROM {table}")):
            if budget_check is not None and index % 256 == 0:
                budget_check()
            identity = row[identity_index]
            if identity not in seed_ids:
                continue
            if identity in seed_fingerprints:
                raise AcceptanceError(f"{dataset} pristine template contains a duplicate seed identity")
            seed_fingerprints[identity] = _row_digest(tuple(row))
        if set(seed_fingerprints) != seed_ids:
            raise AcceptanceError(f"{dataset} pristine template does not contain every seed source row")

        manifest_columns = _columns(connection, "archive_batches")
        required = {"id", "dataset", "month_key", "file_path", "sha256", "row_count", "status", "cleanup_state", "superseded_by"}
        if not required.issubset(manifest_columns):
            raise AcceptanceError(f"production archive manifest schema is incomplete for {dataset}")
        manifest_select = [
            "id",
            "dataset",
            "month_key",
            "file_path",
            "sha256",
            "row_count",
            "status",
            "cleanup_state",
            "superseded_by",
            "summary_source_kind",
        ]
        for optional_column in ("day_key", "part_key", "layout", "codec"):
            if optional_column in manifest_columns:
                manifest_select.append(optional_column)
        manifests = connection.execute(
            f"SELECT {','.join(manifest_select)} FROM archive_batches WHERE dataset=? AND status='completed' AND cleanup_state='active' AND superseded_by IS NULL ORDER BY month_key,file_path,id",
            (dataset,),
        ).fetchall()
        if not manifests:
            raise AcceptanceError(f"no active completed {dataset} manifests were published")
        for manifest in manifests:
            if manifest["dataset"] != dataset or not manifest["month_key"] or not manifest["sha256"]:
                raise AcceptanceError(f"{dataset} manifest identity is incomplete")
            if dataset == "codex_invocations":
                if manifest["summary_source_kind"] != "authoritative":
                    raise AcceptanceError("invocation archive manifest is not authoritative")
            manifest_keys = manifest.keys()
            path = _archive_path(
                database,
                archive_root,
                manifest["file_path"],
                dataset,
                manifest["month_key"],
                day_key=manifest["day_key"] if "day_key" in manifest_keys else None,
                part_key=manifest["part_key"] if "part_key" in manifest_keys else None,
                layout=manifest["layout"] if "layout" in manifest_keys else None,
                codec=manifest["codec"] if "codec" in manifest_keys else None,
            )
            decompressed = scratch / f"{dataset}-{manifest['id']}.sqlite"
            with _verified_archive_snapshot(path, manifest["sha256"], scratch, dataset, budget_check) as snapshot:
                with gzip.GzipFile(fileobj=snapshot, mode="rb") as source, decompressed.open("wb") as target:
                    while chunk := source.read(1024 * 1024):
                        if budget_check is not None:
                            budget_check()
                        target.write(chunk)
            archive = sqlite3.connect(f"file:{decompressed}?mode=ro", uri=True)
            try:
                archived_columns = _table_columns(archive, dataset)
                missing_columns = set(source_columns) - set(archived_columns)
                if missing_columns:
                    raise AcceptanceError(
                        f"{dataset} archive is missing complete source row columns: {sorted(missing_columns)}"
                    )
                if dataset == "codex_invocations":
                    proof_exists = connection.execute(
                        "SELECT 1 FROM summary_archive_snapshot_v2_proof WHERE archive_batch_id=? AND manifest_sha256=?",
                        (manifest["id"], manifest["sha256"]),
                    ).fetchone() is not None
                    if proof_exists or require_summary_proof:
                        _verify_summary_proof(
                            connection,
                            manifest["id"],
                            manifest["sha256"],
                            manifest["file_path"],
                            archive=archive,
                            manifest_row_count=manifest["row_count"],
                            budget_check=budget_check,
                        )
                    else:
                        summary_proof_verified = False
                archived_identity_index = archived_columns.index(identity_column)
                occurred_at_index = archived_columns.index("occurred_at")
                selected_columns = ",".join(source_columns)
                rows = archive.execute(f"SELECT {selected_columns} FROM {dataset}")
                actual_count = 0
                for index, row in enumerate(rows):
                    if budget_check is not None and index % 256 == 0:
                        budget_check()
                    identity = row[archived_identity_index]
                    occurred_at = row[occurred_at_index]
                    if not isinstance(identity, str) or not isinstance(occurred_at, str):
                        raise AcceptanceError(f"{dataset} archive contains a malformed source row")
                    if occurred_at[:7] != manifest["month_key"]:
                        raise AcceptanceError(f"{dataset} archive contains a row outside its manifest month")
                    manifest_day = manifest["day_key"] if "day_key" in manifest_keys else None
                    if manifest_day is not None and occurred_at[:10] != manifest_day:
                        raise AcceptanceError(f"{dataset} archive contains a row outside its manifest day")
                    if identity not in expected_ids:
                        raise AcceptanceError(f"{dataset} archive contains rows outside the seeded and measured input")
                    if identity not in seed_ids:
                        raise AcceptanceError(f"{dataset} archive converted a measured input row past the fixed retention cutoff")
                    row_fingerprint = _row_digest(tuple(row))
                    if row_fingerprint != seed_fingerprints[identity]:
                        raise AcceptanceError(f"{dataset} archive changed full source row content for {identity}")
                    archive_counts[identity] += 1
                    content_records.append((identity, "archive", row_fingerprint))
                    actual_count += 1
                if actual_count != manifest["row_count"]:
                    raise AcceptanceError(f"{dataset} artifact row count disagrees with its completed manifest")
            finally:
                archive.close()
            decompressed.unlink()

        live_counts: Counter[str] = Counter()
        source_identity_index = source_columns.index(identity_column)
        source_occurred_at_index = source_columns.index("occurred_at")
        attempt_invoke_index = source_columns.index("invoke_id") if dataset == "pool_upstream_request_attempts" else None
        cutoff_at = datetime.fromisoformat(cutoff).replace(tzinfo=timezone(timedelta(hours=8)))
        selected_columns = ",".join(source_columns)
        for index, row in enumerate(connection.execute(f"SELECT {selected_columns} FROM {table}")):
            if budget_check is not None and index % 256 == 0:
                budget_check()
            identity = row[source_identity_index]
            if not isinstance(identity, str) or identity not in expected_ids:
                raise AcceptanceError(f"{dataset} live source contains an unrecognized row identity")
            row_fingerprint = _row_digest(tuple(row))
            if identity in seed_ids and row_fingerprint != seed_fingerprints[identity]:
                raise AcceptanceError(f"{dataset} live source changed full seed row content for {identity}")
            if identity in added_ids:
                occurred_at = row[source_occurred_at_index]
                if not isinstance(occurred_at, str):
                    raise AcceptanceError(f"{dataset} measured source row has no valid occurrence time")
                if parse_time(occurred_at) < cutoff_at:
                    raise AcceptanceError(f"{dataset} measured source row predates the fixed retention cutoff")
                if attempt_invoke_index is not None and row[attempt_invoke_index] != added_attempt_invocations[identity]:
                    raise AcceptanceError(f"{dataset} live source changed the measured attempt-to-invocation association")
            live_counts[identity] += 1
            content_records.append((identity, "live", row_fingerprint))
    finally:
        connection.close()
        template.close()

    combined = archive_counts + live_counts
    lost_rows = sum(identity not in combined for identity in expected_ids)
    duplicate_rows = sum(max(0, combined[identity] - 1) for identity in expected_ids)
    unexpected_archived = set(archive_counts) - expected_ids
    if unexpected_archived:
        raise AcceptanceError(f"{dataset} archive contains rows outside the seeded and measured input")
    distinct_archived = sum(archive_counts[identity] == 1 and live_counts[identity] == 0 for identity in expected_ids)
    live_rows = sum(live_counts[identity] for identity in expected_ids)
    if lost_rows or duplicate_rows:
        raise AcceptanceError(f"{dataset} source/artifact conservation failed: lost={lost_rows}, duplicates={duplicate_rows}")
    if sum(combined[identity] for identity in expected_ids) != len(expected_ids):
        raise AcceptanceError(f"{dataset} source/artifact total does not balance")
    content_digest = hashlib.sha256()
    for record in sorted(content_records):
        content_digest.update(canonical_json(record))
        content_digest.update(b"\n")
    if len(content_records) != len(expected_ids):
        raise AcceptanceError(f"{dataset} full source row content was not checked exactly once")
    return {
        "dataset": dataset,
        "seed_rows": seed_count,
        "actual_added_rows": added_count,
        "converted_rows": distinct_archived,
        "distinct_archived_rows": distinct_archived,
        "live_rows": live_rows,
        "lost_rows": lost_rows,
        "duplicate_rows": duplicate_rows,
        "archive_manifest_count": len(manifests),
        "row_content_columns": len(source_columns),
        "row_content_rows_checked": len(content_records),
        "row_content_sha256": content_digest.hexdigest(),
        "summary_proof_required": require_summary_proof,
        "summary_proof_verified": summary_proof_verified,
        **({"attempt_invocation_map_sha256": sha256_json(sorted([[attempt_id, invoke_id] for attempt_id, invoke_id in added_attempt_invocations.items()]))} if dataset == "pool_upstream_request_attempts" else {}),
        "verified": True,
    }


def verify_trial(
    database: Path,
    archive_root: Path,
    added_ids: dict[str, list[str]],
    attempt_states: list[dict[str, Any]],
    template_database: Path,
    cutoff: str,
    scratch: Path,
    budget_check: Callable[[], float] | None = None,
    require_summary_proof: bool = True,
) -> dict[str, Any]:
    scratch.mkdir(parents=True, exist_ok=True)
    results = {}
    for name, definition in DATASETS.items():
        seed_ids = {
            f"retention-seed-{name}-{index:06d}"
            for index in range(1, definition["seed_rows"] + 1)
        }
        measured = added_ids[name]
        if len(measured) != len(set(measured)):
            raise AcceptanceError(f"foreground input generated duplicate {name} identities")
        expected_ids = seed_ids.union(measured)
        result = verify_dataset(
            database,
            archive_root,
            name,
            expected_ids,
            definition["seed_rows"],
            len(measured),
            scratch,
            template_database=template_database,
            seed_ids=seed_ids,
            added_attempt_invocations={
                row["attempt_public_id"]: row["invoke_id"]
                for row in attempt_states
            } if name == "pool_upstream_attempts" else {},
            cutoff=cutoff,
            require_summary_proof=require_summary_proof,
            budget_check=budget_check,
        )
        results[name] = result
    return {"verified": True, "datasets": results}
