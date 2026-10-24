from __future__ import annotations

import gzip
import hashlib
from pathlib import Path
import sqlite3
import tempfile
import unittest

from contract import AcceptanceError
from oracle import _verify_summary_proof, verify_dataset


DATASET = "pool_upstream_request_attempts"
MONTH = "2026-08"


class OracleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.database = self.root / "codex_vibe_monitor.db"
        self.archive_root = self.root / "archives"
        self.scratch = self.root / "scratch"
        self.archive_dir = self.archive_root / DATASET / "2026"
        self.archive_dir.mkdir(parents=True)
        connection = sqlite3.connect(self.database)
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
            CREATE TABLE pool_upstream_request_attempts (
                attempt_public_id TEXT NOT NULL,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL
            );
            """
        )
        connection.execute(
            "INSERT INTO pool_upstream_request_attempts VALUES(?,?,?)",
            ("seed-live", "seed-invocation", "2026-09-01T00:00:00Z"),
        )
        connection.commit()
        connection.close()

    def tearDown(self) -> None:
        self.temp.cleanup()

    def make_archive(
        self,
        rows: list[tuple[str, str]],
        month_key: str = MONTH,
        stored_path: Path | None = None,
    ) -> Path:
        filename = f"{DATASET}-{month_key}.sqlite.gz"
        path = stored_path or self.archive_root / DATASET / month_key[:4] / filename
        path.parent.mkdir(parents=True, exist_ok=True)
        sqlite_path = self.root / "archive.sqlite"
        archive = sqlite3.connect(sqlite_path)
        archive.execute("CREATE TABLE pool_upstream_request_attempts(attempt_public_id TEXT, invoke_id TEXT, occurred_at TEXT)")
        archive.executemany(
            "INSERT INTO pool_upstream_request_attempts VALUES(?,?,?)",
            [(identity, f"invoke-{identity}", occurred_at) for identity, occurred_at in rows],
        )
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

    def verify(self, expected: set[str], seed_count: int = 2, added_count: int = 0) -> dict:
        return verify_dataset(
            self.database,
            self.archive_root,
            "pool_upstream_attempts",
            expected,
            seed_count,
            added_count,
            self.scratch,
        )

    def test_artifact_and_live_rows_reconcile(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(path, 1)
        result = self.verify({"seed-archived", "seed-live"})
        self.assertEqual(result["converted_rows"], 1)
        self.assertEqual(result["live_rows"], 1)
        self.assertEqual(result["lost_rows"], 0)

    def test_container_archive_mount_path_maps_into_the_trial_volume(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(
            path,
            1,
            stored_path=f"/srv/app/data/archives/{DATASET}/2026/{DATASET}-{MONTH}.sqlite.gz",
        )
        result = self.verify({"seed-archived", "seed-live"})
        self.assertEqual(result["converted_rows"], 1)

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

    def test_missing_duplicate_and_unexpected_rows_fail_closed(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(path, 1)
        with self.assertRaisesRegex(AcceptanceError, "lost=1"):
            self.verify({"seed-archived", "seed-live", "missing"}, seed_count=3)

        path.unlink()
        path = self.make_archive([
            ("seed-live", f"{MONTH}-12T12:00:00Z"),
            ("seed-live", f"{MONTH}-13T12:00:00Z"),
        ])
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
