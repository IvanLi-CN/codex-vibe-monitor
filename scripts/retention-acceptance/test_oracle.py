from __future__ import annotations

import gzip
import hashlib
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from contract import AcceptanceError, canonical_json, sha256_json
from oracle import REPLAY_TARGETS, _verify_summary_proof, verify_dataset


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

    def test_container_archive_mount_path_maps_into_the_trial_volume(self) -> None:
        path = self.make_archive([("seed-archived", f"{MONTH}-12T12:00:00Z")])
        self.add_manifest(
            path,
            1,
            stored_path=f"/srv/app/data/archives/{DATASET}/2026/{DATASET}-{MONTH}.sqlite.gz",
        )
        result = self.verify({"seed-archived", "seed-live"})
        self.assertEqual(result["converted_rows"], 1)

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


class SummaryProofTests(unittest.TestCase):
    def setUp(self) -> None:
        self.connection = sqlite3.connect(":memory:")
        self.archive = sqlite3.connect(":memory:")
        self.addCleanup(self.connection.close)
        self.addCleanup(self.archive.close)
        self.connection.executescript(
            "CREATE TABLE summary_archive_snapshot_v2_proof(archive_batch_id INTEGER,manifest_sha256 TEXT,page_count INTEGER,row_count INTEGER,coverage_start TEXT,coverage_end TEXT,semantic_sha256 TEXT);"
            "CREATE TABLE summary_archive_snapshot(archive_batch_id INTEGER,manifest_sha256 TEXT,page_index INTEGER,coverage_start TEXT,coverage_end TEXT,row_count INTEGER,payload BLOB,payload_bytes INTEGER,snapshot_sha256 TEXT,format_version INTEGER);"
            "CREATE TABLE hourly_rollup_archive_replay(target TEXT,dataset TEXT,file_path TEXT,archive_sha256 TEXT);"
        )
        self.archive.execute(
            """
            CREATE TABLE codex_invocations(
                id INTEGER, invoke_id TEXT, occurred_at TEXT, source TEXT, model TEXT,
                input_tokens INTEGER, output_tokens INTEGER, cache_input_tokens INTEGER,
                reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL, cost_input REAL,
                cost_cache_write REAL, cost_cache_read REAL, cost_output REAL, cost_reasoning REAL,
                status TEXT, error_message TEXT, failure_kind TEXT, failure_class TEXT,
                is_actionable INTEGER, payload TEXT
            )
            """
        )
        self.records = [
            {
                "id": index,
                "invokeId": f"seed-{index}",
                "occurredAt": "2026-08-01 00:00:00" if index == 10 else "2026-08-01T01:00:00Z",
                "source": "xy",
                "model": "gpt-5-codex",
                "responseModel": None,
                "inputTokens": 12,
                "outputTokens": 4,
                "cacheInputTokens": 0,
                "reasoningTokens": 0,
                "reasoningEffort": None,
                "totalTokens": 16,
                "cost": None,
                "costInput": None,
                "costCacheWrite": None,
                "costCacheRead": None,
                "costOutput": None,
                "costReasoning": None,
                "status": "success",
                "errorMessage": None,
                "failureKind": None,
                "failureClass": None,
                "isActionable": False,
                "upstreamAccountId": None,
            }
            for index in (10, 20, 30)
        ]
        self.archive.executemany(
            "INSERT INTO codex_invocations VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            [
                (
                    record["id"], record["invokeId"], record["occurredAt"], record["source"], record["model"],
                    record["inputTokens"], record["outputTokens"], record["cacheInputTokens"],
                    record["reasoningTokens"], record["totalTokens"], record["cost"], record["costInput"],
                    record["costCacheWrite"], record["costCacheRead"], record["costOutput"],
                    record["costReasoning"], record["status"], record["errorMessage"], record["failureKind"],
                    record["failureClass"], int(record["isActionable"]), None,
                )
                for record in reversed(self.records)
            ],
        )
        self.archive.commit()
        self.manifest_sha = "a" * 64
        self.file_path = "/archives/codex_invocations/2026/codex_invocations-2026-08.sqlite.gz"
        self.connection.executemany(
            "INSERT INTO hourly_rollup_archive_replay VALUES(?, 'codex_invocations', ?, ?)",
            [(target, self.file_path, self.manifest_sha) for target in REPLAY_TARGETS],
        )
        self.store_pages()

    def compress(self, raw: bytes) -> bytes:
        return subprocess.run(
            ["zstd", "-q", "-c"], input=raw, capture_output=True, check=True, timeout=5,
        ).stdout

    def store_pages(
        self,
        pages: list[list] | None = None,
        *,
        payloads: dict[int, bytes] | None = None,
        overrides: dict[int, dict] | None = None,
    ) -> None:
        pages = pages if pages is not None else [self.records[:1], self.records[1:]]
        self.connection.execute("DELETE FROM summary_archive_snapshot")
        self.connection.execute("DELETE FROM summary_archive_snapshot_v2_proof")
        semantic = hashlib.sha256()
        total = 0
        bounds = []
        for index, records in enumerate(pages):
            payload = (payloads or {}).get(index)
            if payload is None:
                payload = self.compress(canonical_json(records))
            start = records[0].get("occurredAt") if records and isinstance(records[0], dict) else self.records[0]["occurredAt"]
            end = records[-1].get("occurredAt") if records and isinstance(records[-1], dict) else self.records[-1]["occurredAt"]
            metadata = {"row_count": len(records), "coverage_start": start, "coverage_end": end}
            metadata.update((overrides or {}).get(index, {}))
            count, start, end = metadata["row_count"], metadata["coverage_start"], metadata["coverage_end"]
            page_sha = hashlib.sha256(payload).hexdigest()
            self.connection.execute(
                "INSERT INTO summary_archive_snapshot VALUES(1,?,?,?,?,?,?,?,?,2)",
                (self.manifest_sha, index, start, end, count, payload, len(payload), page_sha),
            )
            semantic.update(index.to_bytes(8, "little", signed=True))
            semantic.update(page_sha.encode())
            semantic.update(count.to_bytes(8, "little", signed=True))
            semantic.update((start or "").encode())
            semantic.update((end or "").encode())
            total += count
            bounds.append((start, end))
        self.connection.execute(
            "INSERT INTO summary_archive_snapshot_v2_proof VALUES(1,?,?,?,?,?,?)",
            (self.manifest_sha, len(pages), total, bounds[0][0], bounds[-1][1], semantic.hexdigest()),
        )
        self.connection.commit()

    def verify(self, manifest_row_count: int = 3) -> None:
        _verify_summary_proof(
            self.connection, 1, self.manifest_sha, self.file_path,
            archive=self.archive, manifest_row_count=manifest_row_count,
        )

    def test_real_zstd_json_pages_cover_actual_archive_rows(self) -> None:
        self.verify()

    def test_missing_summary_final_proof_is_rejected(self) -> None:
        self.connection.execute("DELETE FROM summary_archive_snapshot_v2_proof")
        with self.assertRaisesRegex(AcceptanceError, "missing its final Summary Snapshot V2 proof"):
            self.verify()

    def test_summary_semantic_proof_is_recomputed_from_pages_and_replay_hashes(self) -> None:
        self.connection.execute("UPDATE summary_archive_snapshot_v2_proof SET semantic_sha256='f'")
        with self.assertRaisesRegex(AcceptanceError, "semantic proof does not match"):
            self.verify()
        self.store_pages()
        self.connection.execute("UPDATE hourly_rollup_archive_replay SET archive_sha256='unrelated'")
        with self.assertRaisesRegex(AcceptanceError, "hash-bound hourly replay proofs"):
            self.verify()

    def test_self_consistent_hashes_cannot_prove_non_zstd_or_non_json_payloads(self) -> None:
        for payload, error in (
            (b"summary-page", "not a zstd frame"),
            (gzip.compress(canonical_json(self.records[:1])), "not a zstd frame"),
            (self.compress(canonical_json(self.records[:1]))[:-3], "decoded with zstd"),
            (self.compress(b"not JSON"), "not valid JSON"),
            (self.compress(b"[NaN]"), "not valid JSON"),
            (self.compress(b'[{"id":10,"id":20}]'), "not valid JSON"),
            (self.compress(b'"not a record list"'), "record list"),
            (self.compress(b"[]"), "record list"),
            (self.compress(b"[null]"), "malformed record"),
        ):
            with self.subTest(payload=payload):
                self.store_pages(payloads={0: payload})
                with self.assertRaisesRegex(AcceptanceError, error):
                    self.verify()

    def test_missing_decoder_fails_closed(self) -> None:
        with patch("oracle.subprocess.run", side_effect=FileNotFoundError("zstd")):
            with self.assertRaisesRegex(AcceptanceError, "decoded with zstd"):
                self.verify()

    def test_decoded_records_require_v2_semantic_fields(self) -> None:
        for field, value in (
            ("id", True), ("id", 2**63), ("invokeId", ""), ("source", None),
            ("status", " "), ("inputTokens", "12"), ("isActionable", 0),
            ("model", 1), ("upstreamAccountId", True), ("cost", "1.0"), ("cost", 1e309),
        ):
            with self.subTest(field=field):
                records = [dict(record) for record in self.records]
                records[0][field] = value
                self.store_pages([records[:1], records[1:]])
                with self.assertRaisesRegex(AcceptanceError, f"invalid {field}|not valid JSON"):
                    self.verify()

    def test_payload_row_count_is_checked_after_successful_decode(self) -> None:
        self.store_pages(overrides={0: {"row_count": 2}})
        with self.assertRaisesRegex(AcceptanceError, "payload row count does not match"):
            self.verify()

    def test_payload_identity_and_occurrence_must_match_actual_archive_rows(self) -> None:
        for field, value in (("id", 11), ("invokeId", "unrelated-invocation"), ("occurredAt", "2026-08-01T00:00:01Z")):
            with self.subTest(field=field):
                records = [dict(record) for record in self.records]
                records[0][field] = value
                self.store_pages([records[:1], records[1:]])
                with self.assertRaisesRegex(AcceptanceError, "differs from actual archived"):
                    self.verify()

    def test_payload_business_fields_must_match_actual_archive_rows(self) -> None:
        records = [dict(record) for record in self.records]
        records[0]["inputTokens"] = 999
        self.store_pages([records[:1], records[1:]])
        with self.assertRaisesRegex(AcceptanceError, "differs from actual archived"):
            self.verify()

    def test_duplicate_payload_identities_across_pages_are_rejected(self) -> None:
        self.store_pages([self.records[:1], [self.records[0], self.records[2]]])
        with self.assertRaisesRegex(AcceptanceError, "duplicate archive row identity"):
            self.verify()

    def test_payload_cannot_omit_or_add_archived_rows_even_with_matching_metadata(self) -> None:
        self.store_pages([self.records[:2]])
        with self.assertRaisesRegex(AcceptanceError, "payload count does not cover"):
            self.verify(manifest_row_count=2)
        extra = {**self.records[-1], "id": 40, "invokeId": "seed-40"}
        self.store_pages([self.records[:1], self.records[1:] + [extra]])
        with self.assertRaisesRegex(AcceptanceError, "differs from actual archived"):
            self.verify(manifest_row_count=4)
        self.store_pages()
        with self.assertRaisesRegex(AcceptanceError, "payload count does not cover"):
            self.verify(manifest_row_count=4)

    def test_self_consistent_page_coverage_must_match_decoded_records(self) -> None:
        for coverage in (
            {"coverage_start": "2026-07-31T23:59:59Z"},
            {"coverage_end": "2026-08-01T00:00:01Z"},
            {"coverage_start": "2026-08-01T00:00:01Z"},
            {"coverage_end": "invalid"},
        ):
            with self.subTest(coverage=coverage):
                self.store_pages(overrides={0: coverage})
                with self.assertRaisesRegex(AcceptanceError, "coverage"):
                    self.verify()

    def test_payload_order_must_match_archive_order_across_pages_at_equal_timestamps(self) -> None:
        self.store_pages([self.records[:1] + self.records[2:], self.records[1:2]])
        with self.assertRaisesRegex(AcceptanceError, "differs from actual archived"):
            self.verify()

    def test_dataset_verification_uses_the_verified_archive_for_summary_proof(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            database, template = root / "source.sqlite", root / "template.sqlite"
            for path in (database, template):
                target = sqlite3.connect(path)
                self.archive.backup(target)
                if path == template:
                    target.close()
                    continue
                target.execute("DELETE FROM codex_invocations")
                target.executescript(
                    "CREATE TABLE archive_batches(id INTEGER,dataset TEXT,month_key TEXT,file_path TEXT,sha256 TEXT,row_count INTEGER,status TEXT,cleanup_state TEXT,superseded_by INTEGER,summary_source_kind TEXT);"
                )
                target.commit()
                target.close()
            archive_root = root / "archives"
            path = archive_root / "codex_invocations" / "2026" / "codex_invocations-2026-08.sqlite.gz"
            path.parent.mkdir(parents=True)
            path.write_bytes(gzip.compress(template.read_bytes()))
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            self.connection.execute("UPDATE summary_archive_snapshot SET manifest_sha256=?", (digest,))
            self.connection.execute("UPDATE summary_archive_snapshot_v2_proof SET manifest_sha256=?", (digest,))
            self.connection.execute("UPDATE hourly_rollup_archive_replay SET archive_sha256=?,file_path=?", (digest, str(path)))
            target = sqlite3.connect(database)
            target.executescript("\n".join(self.connection.iterdump()))
            target.execute(
                "INSERT INTO archive_batches VALUES(1,'codex_invocations','2026-08',?,?,3,'completed','active',NULL,'authoritative')",
                (str(path), digest),
            )
            target.commit()
            target.close()
            arguments = (database, archive_root, "invocations", {record["invokeId"] for record in self.records}, 3, 0, root / "scratch")
            keywords = dict(template_database=template, seed_ids=arguments[3], added_attempt_invocations={}, cutoff="2026-09-01 00:00:00")
            result = verify_dataset(*arguments, **keywords)
            self.assertEqual(result["converted_rows"], 3)
            target = sqlite3.connect(database)
            target.execute("UPDATE summary_archive_snapshot SET payload=? WHERE page_index=0", (b"summary-page",))
            target.commit()
            target.close()
            with self.assertRaises(AcceptanceError):
                verify_dataset(*arguments, **keywords)


if __name__ == "__main__":
    unittest.main()
