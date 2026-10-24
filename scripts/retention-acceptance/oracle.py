"""Independently reconcile live SQLite rows with completed production archives."""

from __future__ import annotations

from collections import Counter
import gzip
import hashlib
from pathlib import Path
import re
import shutil
import sqlite3
from typing import Any

from contract import AcceptanceError, DATASETS


REPLAY_TARGETS = {
    "invocation_rollup_hourly",
    "upstream_account_stats_hourly",
    "upstream_account_usage_breakdown_hourly",
}
CONTAINER_ARCHIVE_ROOT = Path("/srv/app/data/archives")


def _columns(connection: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in connection.execute(f"PRAGMA table_info('{table}')")}


def _hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _archive_path(database: Path, archive_root: Path, stored_path: str, dataset: str, month_key: str) -> Path:
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
    parts = relative.parts
    expected = (dataset, month_key[:4], f"{dataset}-{month_key}.sqlite.gz")
    if parts != expected or not re.fullmatch(r"\d{4}-(0[1-9]|1[0-2])", month_key):
        raise AcceptanceError("archive manifest did not resolve to the canonical dataset/year/month path")
    if resolved.suffixes[-2:] != [".sqlite", ".gz"]:
        raise AcceptanceError("archive artifact is not a gzip SQLite file")
    return resolved


def _verify_summary_proof(connection: sqlite3.Connection, archive_id: int, manifest_sha: str, file_path: str) -> None:
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
    for index, page in enumerate(pages):
        page_index, start, end, row_count, payload, payload_bytes, page_sha, version = page
        payload = bytes(payload)
        if page_index != index or version != 2 or payload_bytes != len(payload):
            raise AcceptanceError("Summary Snapshot page order, format, or byte count is invalid")
        if hashlib.sha256(payload).hexdigest() != page_sha or row_count <= 0 or not start or not end:
            raise AcceptanceError("Summary Snapshot page checksum or coverage is invalid")
        semantic.update(int(page_index).to_bytes(8, "little", signed=True))
        semantic.update(page_sha.encode())
        semantic.update(int(row_count).to_bytes(8, "little", signed=True))
        semantic.update(start.encode())
        semantic.update(end.encode())
        total_rows += row_count
    if proof[1] != total_rows or proof[2] != pages[0][1] or proof[3] != pages[-1][2] or proof[4] != semantic.hexdigest():
        raise AcceptanceError("Summary Snapshot V2 semantic proof does not match its pages")
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
) -> dict[str, Any]:
    dataset = DATASETS[dataset_name]["manifest_dataset"]
    table = DATASETS[dataset_name]["table"]
    identity_column = DATASETS[dataset_name]["identity_column"]
    if seed_count < 0 or added_count < 0 or len(expected_ids) != seed_count + added_count:
        raise AcceptanceError(f"{dataset} expected identity set disagrees with its reported input counts")
    scratch.mkdir(parents=True, exist_ok=True)
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    archive_counts: Counter[str] = Counter()
    try:
        manifest_columns = _columns(connection, "archive_batches")
        required = {"id", "dataset", "month_key", "file_path", "sha256", "row_count", "status", "cleanup_state", "superseded_by"}
        if not required.issubset(manifest_columns):
            raise AcceptanceError(f"production archive manifest schema is incomplete for {dataset}")
        manifests = connection.execute(
            "SELECT id,dataset,month_key,file_path,sha256,row_count,status,cleanup_state,superseded_by,summary_source_kind FROM archive_batches WHERE dataset=? AND status='completed' AND cleanup_state='active' AND superseded_by IS NULL ORDER BY month_key,file_path,id",
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
                _verify_summary_proof(connection, manifest["id"], manifest["sha256"], manifest["file_path"])
            path = _archive_path(database, archive_root, manifest["file_path"], dataset, manifest["month_key"])
            if _hash_file(path) != manifest["sha256"]:
                raise AcceptanceError(f"{dataset} archive checksum does not match its manifest")
            decompressed = scratch / f"{dataset}-{manifest['id']}.sqlite"
            with gzip.open(path, "rb") as source, decompressed.open("wb") as target:
                shutil.copyfileobj(source, target, length=1024 * 1024)
            archive = sqlite3.connect(f"file:{decompressed}?mode=ro", uri=True)
            try:
                archived_columns = _columns(archive, dataset)
                if not {identity_column, "occurred_at"}.issubset(archived_columns):
                    raise AcceptanceError(f"{dataset} archive lacks source identity columns")
                rows = archive.execute(f"SELECT {identity_column},occurred_at FROM {dataset}")
                actual_count = 0
                for identity, occurred_at in rows:
                    if not isinstance(identity, str) or not isinstance(occurred_at, str):
                        raise AcceptanceError(f"{dataset} archive contains a malformed source row")
                    if occurred_at[:7] != manifest["month_key"]:
                        raise AcceptanceError(f"{dataset} archive contains a row outside its manifest month")
                    archive_counts[identity] += 1
                    actual_count += 1
                if actual_count != manifest["row_count"]:
                    raise AcceptanceError(f"{dataset} artifact row count disagrees with its completed manifest")
            finally:
                archive.close()
            decompressed.unlink()

        live_counts: Counter[str] = Counter()
        for (identity,) in connection.execute(f"SELECT {identity_column} FROM {table}"):
            if identity in expected_ids:
                live_counts[identity] += 1
            else:
                raise AcceptanceError(f"{dataset} live source contains an unrecognized row identity")
    finally:
        connection.close()

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
        "verified": True,
    }


def verify_trial(
    database: Path,
    archive_root: Path,
    added_ids: dict[str, list[str]],
    scratch: Path,
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
        )
        results[name] = result
    return {"verified": True, "datasets": results}
