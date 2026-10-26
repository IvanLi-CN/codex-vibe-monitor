from __future__ import annotations

import gzip
import hashlib
from datetime import timedelta
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

from contract import AcceptanceError, sha256_json
from oracle import (
    _archive_path,
    _archive_projection_columns,
    _parse_production_local_time,
    _validated_archive_columns,
    _verify_summary_proof,
    verify_dataset,
)


DATASET = "pool_upstream_request_attempts"
MONTH = "2026-08"


class OracleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.database = self.root / "codex_vibe_monitor.db"
        self.template_database = self.root / "template.db"
        self.archive_root = self.root / "archives"
        self.scratch = self.root / "scratch"
        self.archive_dir = self.archive_root / DATASET / "2026"
        self.archive_dir.mkdir(parents=True)
        for path, with_manifest in ((self.database, True), (self.template_database, False)):
            connection = sqlite3.connect(path)
            if with_manifest:
                connection.executescript(
                    """
                    CREATE TABLE archive_batches (
                        id INTEGER PRIMARY KEY,
                        dataset TEXT NOT NULL,
                        month_key TEXT NOT NULL,
                        file_path TEXT NOT NULL,
                        sha256 TEXT NOT NULL,
                        row_count INTEGER NOT NULL,
                        status TEXT NOT NULL,
                        cleanup_state TEXT NOT NULL,
                        superseded_by INTEGER,
                        summary_source_kind TEXT NOT NULL
                    );
                    """
                )
            connection.execute(
                "CREATE TABLE pool_upstream_request_attempts(id INTEGER,attempt_public_id TEXT NOT NULL,invoke_id TEXT NOT NULL,occurred_at TEXT NOT NULL,status TEXT NOT NULL,request_summary_json TEXT NOT NULL)"
            )
            connection.commit()
            connection.close()
        self.add_seed("seed-live", "2026-09-01T00:00:00Z", source=True)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def make_archive(
        self,
        rows: list[tuple[str, str]],
        month_key: str = MONTH,
        stored_path: Path | None = None,
        row_overrides: dict[str, dict[str, object]] | None = None,
    ) -> Path:
        filename = f"{DATASET}-{month_key}.sqlite.gz"
        path = stored_path or self.archive_root / DATASET / month_key[:4] / filename
        path.parent.mkdir(parents=True, exist_ok=True)
        sqlite_path = self.root / "archive.sqlite"
        archive = sqlite3.connect(sqlite_path)
        archive.execute(
            "CREATE TABLE pool_upstream_request_attempts(id INTEGER,attempt_public_id TEXT,invoke_id TEXT,occurred_at TEXT,status TEXT,request_summary_json TEXT)"
        )
        archive_rows = []
        for identity, occurred_at in rows:
            self.add_seed(identity, occurred_at)
            template = sqlite3.connect(self.template_database)
            template.row_factory = sqlite3.Row
            row = dict(template.execute(
                "SELECT * FROM pool_upstream_request_attempts WHERE attempt_public_id=?",
                (identity,),
            ).fetchone())
            template.close()
            row.update((row_overrides or {}).get(identity, {}))
            archive_rows.append(tuple(row[column] for column in (
                "id", "attempt_public_id", "invoke_id", "occurred_at", "status", "request_summary_json"
            )))
        archive.executemany("INSERT INTO pool_upstream_request_attempts VALUES(?,?,?,?,?,?)", archive_rows)
        archive.commit()
        archive.close()
        with sqlite_path.open("rb") as source, gzip.open(path, "wb") as target:
            target.write(source.read())
        sqlite_path.unlink()
        return path

    def add_manifest(
        self,
        path: Path,
        row_count: int,
        month_key: str = MONTH,
        dataset: str = DATASET,
        stored_path: str | None = None,
    ) -> None:
        connection = sqlite3.connect(self.database)
        with path.open("rb") as handle:
            digest = hashlib.file_digest(handle, "sha256").hexdigest()
        connection.execute(
            "INSERT INTO archive_batches(dataset,month_key,file_path,sha256,row_count,status,cleanup_state,superseded_by,summary_source_kind) VALUES(?,?,?,?,?,'completed','active',NULL,'unknown')",
            (dataset, month_key, stored_path or str(path), digest, row_count),
        )
        connection.commit()
        connection.close()

    def add_seed(self, identity: str, occurred_at: str, source: bool = False) -> None:
        template = sqlite3.connect(self.template_database)
        existing = template.execute(
            "SELECT 1 FROM pool_upstream_request_attempts WHERE attempt_public_id=?",
            (identity,),
        ).fetchone()
        if existing is None:
            template.execute(
                "INSERT INTO pool_upstream_request_attempts VALUES(?,?,?,?,?,?)",
                (identity, identity, f"invoke-{identity}", occurred_at, "success", f'{{"identity":"{identity}"}}'),
            )
        template.commit()
        template.close()
        if source:
            database = sqlite3.connect(self.database)
            database.execute(
                "INSERT INTO pool_upstream_request_attempts VALUES(?,?,?,?,?,?)",
                (identity, identity, f"invoke-{identity}", occurred_at, "success", f'{{"identity":"{identity}"}}'),
            )
            database.commit()
            database.close()

    def verify(
        self,
        expected: set[str],
        seed_count: int = 2,
        added_count: int = 0,
        attempt_invocations: dict[str, str] | None = None,
    ) -> dict:
        attempt_invocations = attempt_invocations or {}
        seed_ids = expected - set(attempt_invocations)
        for identity in seed_ids:
            self.add_seed(identity, f"{MONTH}-01T00:00:00Z")
        return verify_dataset(
            self.database,
            self.archive_root,
            "pool_upstream_attempts",
            expected,
            seed_count,
            added_count,
            self.scratch,
            template_database=self.template_database,
            seed_ids=seed_ids,
            added_attempt_invocations=attempt_invocations,
            cutoff=f"{MONTH}-01 00:00:00",
        )

    def add_live_attempt(self, identity: str, invoke_id: str, occurred_at: str) -> None:
        connection = sqlite3.connect(self.database)
        connection.execute(
            "INSERT INTO pool_upstream_request_attempts VALUES(?,?,?,?,?,?)",
            (100, identity, invoke_id, occurred_at, "success", f'{{"identity":"{identity}"}}'),
        )
        connection.commit()
        connection.close()

    def test_artifact_and_live_rows_reconcile(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(path, 1)
        result = self.verify({"seed-archived", "seed-live"})
        self.assertEqual(result["converted_rows"], 1)
        self.assertEqual(result["live_rows"], 1)
        self.assertEqual(result["lost_rows"], 0)

    def test_archive_projection_excludes_known_generated_or_unarchived_columns(self) -> None:
        self.assertEqual(
            _archive_projection_columns(
                "codex_invocations",
                ["id", "invoke_id", "timeline_json", "occurred_at"],
            ),
            ["id", "invoke_id", "occurred_at"],
        )
        self.assertEqual(
            _archive_projection_columns(
                "pool_upstream_request_attempts",
                ["id", "occurred_epoch_ms", "attempt_public_id"],
            ),
            ["id", "attempt_public_id"],
        )

    def test_production_local_timestamps_accept_naive_sqlite_values(self) -> None:
        local_time = _parse_production_local_time("2026-08-12 12:00:00")
        utc_time = _parse_production_local_time("2026-08-12T04:00:00Z")
        self.assertEqual(local_time.utcoffset(), timedelta(hours=8))
        self.assertEqual(local_time, utc_time)

    def test_archive_columns_accept_current_and_legacy_known_shapes_only(self) -> None:
        source_columns = [
            "id",
            "attempt_public_id",
            "invoke_id",
            "occurred_at",
            "occurred_epoch_ms",
            "status",
            "request_summary_json",
        ]
        self.assertEqual(
            _validated_archive_columns("pool_upstream_request_attempts", source_columns, source_columns),
            source_columns,
        )
        legacy_columns = [column for column in source_columns if column != "occurred_epoch_ms"]
        self.assertEqual(
            _validated_archive_columns("pool_upstream_request_attempts", source_columns, legacy_columns),
            legacy_columns,
        )
        with self.assertRaisesRegex(AcceptanceError, "missing=.*status"):
            _validated_archive_columns(
                "pool_upstream_request_attempts",
                source_columns,
                [column for column in source_columns if column != "status"],
            )
        with self.assertRaisesRegex(AcceptanceError, "extra=.*unexpected"):
            _validated_archive_columns(
                "pool_upstream_request_attempts",
                source_columns,
                [*source_columns, "unexpected"],
            )

    def test_container_archive_mount_path_maps_into_the_trial_volume(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(
            path,
            1,
            stored_path=f"/srv/app/data/archives/{DATASET}/2026/{DATASET}-{MONTH}.sqlite.gz",
        )
        result = self.verify({"seed-archived", "seed-live"})
        self.assertEqual(result["converted_rows"], 1)

    def test_invocation_segment_v1_archive_path_maps_into_the_trial_volume(self) -> None:
        part_name = "part-0000000000000001-0000000000000001-0123456789abcdef.sqlite.gz"
        path = self.archive_root / "codex_invocations" / "2026" / "08" / "12" / part_name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b"segment")
        resolved = _archive_path(
            self.database,
            self.archive_root,
            f"/srv/app/data/archives/codex_invocations/2026/08/12/{part_name}",
            "codex_invocations",
            MONTH,
        )
        self.assertEqual(resolved, path.resolve())

    def test_archive_full_row_content_must_match_the_pristine_source(self) -> None:
        path = self.make_archive(
            [("seed-archived", f"{MONTH}-12T12:00:00Z")],
            row_overrides={"seed-archived": {"request_summary_json": "tampered"}},
        )
        self.add_manifest(path, 1)
        with self.assertRaisesRegex(AcceptanceError, "changed full source row content"):
            self.verify({"seed-archived", "seed-live"})

    def test_live_attempt_invocation_association_must_match_observed_http_evidence(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(path, 1)
        self.add_live_attempt("added-attempt", "wrong-invocation", "2026-08-12T12:00:00Z")
        with self.assertRaisesRegex(AcceptanceError, "changed the measured attempt-to-invocation association"):
            self.verify(
                {"seed-live", "seed-archived", "added-attempt"},
                seed_count=2,
                added_count=1,
                attempt_invocations={"added-attempt": "expected-invocation"},
            )

    def test_oracle_reports_the_observed_attempt_invocation_map(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(path, 1)
        self.add_live_attempt("added-attempt", "expected-invocation", "2026-08-12T12:00:00Z")
        result = self.verify(
            {"seed-live", "seed-archived", "added-attempt"},
            seed_count=2,
            added_count=1,
            attempt_invocations={"added-attempt": "expected-invocation"},
        )
        self.assertEqual(
            result["attempt_invocation_map_sha256"],
            sha256_json([["added-attempt", "expected-invocation"]]),
        )

    def test_wrong_dataset_month_and_checksum_are_rejected(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(path, 1, dataset="different_dataset")
        with self.assertRaisesRegex(AcceptanceError, "no active completed"):
            self.verify({"seed-archived", "seed-live"})

        connection = sqlite3.connect(self.database)
        connection.execute("DELETE FROM archive_batches")
        connection.commit()
        connection.close()
        self.add_manifest(path, 1, month_key="2026-07")
        with self.assertRaisesRegex(AcceptanceError, "canonical dataset/year/month"):
            self.verify({"seed-archived", "seed-live"})

        connection = sqlite3.connect(self.database)
        connection.execute("DELETE FROM archive_batches")
        connection.commit()
        connection.close()
        self.add_manifest(path, 1)
        with path.open("ab") as handle:
            handle.write(b"tampered")
        with self.assertRaisesRegex(AcceptanceError, "checksum"):
            self.verify({"seed-archived", "seed-live"})

    def test_replaced_archive_with_equivalent_rows_and_different_gzip_is_rejected(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(path, 1)
        original_bytes = path.read_bytes()
        replacement_bytes = gzip.compress(gzip.decompress(original_bytes), mtime=1)
        self.assertNotEqual(replacement_bytes, original_bytes)
        self.assertEqual(gzip.decompress(replacement_bytes), gzip.decompress(original_bytes))
        real_gzip_file = gzip.GzipFile

        def replace_before_read(*args, **kwargs):
            replacement = path.with_suffix(".replacement.gz")
            replacement.write_bytes(replacement_bytes)
            replacement.replace(path)
            return real_gzip_file(*args, **kwargs)

        with patch("oracle.gzip.GzipFile", side_effect=replace_before_read):
            with self.assertRaisesRegex(AcceptanceError, "identity or metadata changed"):
                self.verify({"seed-archived", "seed-live"})

    def test_in_place_archive_change_during_read_is_rejected(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(path, 1)
        original_bytes = path.read_bytes()
        replacement_bytes = gzip.compress(gzip.decompress(original_bytes), mtime=1)
        self.assertNotEqual(replacement_bytes, original_bytes)
        real_gzip_file = gzip.GzipFile

        def rewrite_before_read(*args, **kwargs):
            with path.open("r+b") as handle:
                handle.write(replacement_bytes)
                handle.truncate()
                handle.flush()
            return real_gzip_file(*args, **kwargs)

        with patch("oracle.gzip.GzipFile", side_effect=rewrite_before_read):
            with self.assertRaisesRegex(AcceptanceError, "identity or metadata changed"):
                self.verify({"seed-archived", "seed-live"})

    def test_missing_duplicate_and_unexpected_rows_fail_closed(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(path, 1)
        with self.assertRaisesRegex(AcceptanceError, "lost=1"):
            self.verify({"seed-archived", "seed-live", "missing"}, seed_count=3)

        path.unlink()
        path = self.make_archive([
            ("seed-archived", f"{MONTH}-12T12:00:00Z"),
            ("seed-archived", f"{MONTH}-13T12:00:00Z"),
        ])
        self.add_seed("seed-archived", f"{MONTH}-12T12:00:00Z", source=True)
        connection = sqlite3.connect(self.database)
        connection.execute("DELETE FROM archive_batches")
        connection.commit()
        connection.close()
        self.add_manifest(path, 2)
        with self.assertRaisesRegex(AcceptanceError, "duplicates=2"):
            self.verify({"seed-live", "seed-archived"})

    def test_untrusted_count_and_archive_path_escape_are_rejected(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(path, 1)
        with self.assertRaisesRegex(AcceptanceError, "identity set disagrees"):
            self.verify({"seed-archived", "seed-live"}, seed_count=2, added_count=1)

        outside = self.root / f"{DATASET}-{MONTH}.sqlite.gz"
        outside.write_bytes(path.read_bytes())
        connection = sqlite3.connect(self.database)
        connection.execute("UPDATE archive_batches SET file_path=?", (str(outside),))
        connection.commit()
        connection.close()
        with self.assertRaisesRegex(AcceptanceError, "escaped the configured archive root"):
            self.verify({"seed-archived", "seed-live"})

    def test_missing_summary_final_proof_is_rejected(self) -> None:
        connection = sqlite3.connect(self.database)
        connection.executescript(
            "CREATE TABLE summary_archive_snapshot_v2_proof(archive_batch_id INTEGER,manifest_sha256 TEXT,page_count INTEGER,row_count INTEGER,coverage_start TEXT,coverage_end TEXT,semantic_sha256 TEXT);"
            "CREATE TABLE summary_archive_snapshot(archive_batch_id INTEGER,manifest_sha256 TEXT,page_index INTEGER,coverage_start TEXT,coverage_end TEXT,row_count INTEGER,payload BLOB,payload_bytes INTEGER,snapshot_sha256 TEXT,format_version INTEGER);"
            "CREATE TABLE hourly_rollup_archive_replay(target TEXT,dataset TEXT,file_path TEXT,archive_sha256 TEXT);"
        )
        connection.commit()
        with self.assertRaisesRegex(AcceptanceError, "missing its final Summary Snapshot V2 proof"):
            _verify_summary_proof(connection, 1, "a" * 64, "/unused")
        connection.close()

    def test_live_mirror_invocation_manifest_is_not_an_authoritative_archive(self) -> None:
        for database in (self.database, self.template_database):
            connection = sqlite3.connect(database)
            connection.execute(
                "CREATE TABLE codex_invocations(invoke_id TEXT NOT NULL,occurred_at TEXT NOT NULL,raw_response TEXT NOT NULL)"
            )
            connection.commit()
            connection.close()
        connection = sqlite3.connect(self.database)
        connection.execute(
            "INSERT INTO archive_batches(dataset,month_key,file_path,sha256,row_count,status,cleanup_state,superseded_by,summary_source_kind) "
            "VALUES('codex_invocations',?,?,?,?, 'completed','active',NULL,'live_mirror')",
            (MONTH, "/unused", "a" * 64, 0),
        )
        connection.commit()
        connection.close()
        with self.assertRaisesRegex(AcceptanceError, "no active completed codex_invocations manifests"):
            verify_dataset(
                self.database,
                self.archive_root,
                "invocations",
                set(),
                0,
                0,
                self.scratch,
                template_database=self.template_database,
                seed_ids=set(),
                added_attempt_invocations={},
                cutoff=f"{MONTH}-01 00:00:00",
            )

    def test_summary_semantic_proof_is_recomputed_from_pages_and_replay_hashes(self) -> None:
        connection = sqlite3.connect(self.database)
        connection.executescript(
            "CREATE TABLE summary_archive_snapshot_v2_proof(archive_batch_id INTEGER,manifest_sha256 TEXT,page_count INTEGER,row_count INTEGER,coverage_start TEXT,coverage_end TEXT,semantic_sha256 TEXT);"
            "CREATE TABLE summary_archive_snapshot(archive_batch_id INTEGER,manifest_sha256 TEXT,page_index INTEGER,coverage_start TEXT,coverage_end TEXT,row_count INTEGER,payload BLOB,payload_bytes INTEGER,snapshot_sha256 TEXT,format_version INTEGER);"
            "CREATE TABLE hourly_rollup_archive_replay(target TEXT,dataset TEXT,file_path TEXT,archive_sha256 TEXT);"
        )
        manifest_sha = "a" * 64
        file_path = "/archives/codex_invocations/2026/codex_invocations-2026-08.sqlite.gz"
        payload = b"summary-page"
        page_sha = hashlib.sha256(payload).hexdigest()
        start = "2026-08-01T00:00:00Z"
        end = "2026-08-31T23:59:59Z"
        semantic = hashlib.sha256()
        semantic.update((0).to_bytes(8, "little", signed=True))
        semantic.update(page_sha.encode())
        semantic.update((5).to_bytes(8, "little", signed=True))
        semantic.update(start.encode())
        semantic.update(end.encode())
        connection.execute(
            "INSERT INTO summary_archive_snapshot VALUES(1,?,0,?,?,5,?,?,?,2)",
            (manifest_sha, start, end, payload, len(payload), page_sha),
        )
        connection.execute(
            "INSERT INTO summary_archive_snapshot_v2_proof VALUES(1,?,1,5,?,?,?)",
            (manifest_sha, start, end, semantic.hexdigest()),
        )
        from oracle import REPLAY_TARGETS

        connection.executemany(
            "INSERT INTO hourly_rollup_archive_replay VALUES(?, 'codex_invocations', ?, ?)",
            [(target, file_path, manifest_sha) for target in REPLAY_TARGETS],
        )
        connection.commit()
        _verify_summary_proof(connection, 1, manifest_sha, file_path)
        connection.execute("UPDATE summary_archive_snapshot_v2_proof SET semantic_sha256='f'")
        connection.commit()
        with self.assertRaisesRegex(AcceptanceError, "semantic proof does not match"):
            _verify_summary_proof(connection, 1, manifest_sha, file_path)
        connection.close()


if __name__ == "__main__":
    unittest.main()
