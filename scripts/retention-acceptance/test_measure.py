from __future__ import annotations

from datetime import datetime, timezone
import json
import os
import shutil
import sqlite3
import subprocess
from pathlib import Path
from subprocess import CompletedProcess
import tempfile
import threading
import time
import unittest
from unittest.mock import ANY, patch

from contract import (
    AcceptanceError,
    CLEANUP_RESERVE_SECONDS,
    COMPETITOR_FILE_BYTES,
    COMPETITOR_FILE_COUNT,
    COMPETITOR_START_DELAY_SECONDS,
    COMPETITOR_TASK,
    CONTAINER_KIND_LABEL,
    CONTAINER_OWNER_LABEL,
    CONTAINER_RUN_ID_LABEL,
    CONTAINER_TRIAL_LABEL,
    DATASETS,
    TRIAL_EXECUTION_ORDER,
)
from fixture import ARCHIVE_TASK_KEYS, CONTROLLED_TASK_KEYS, prepare_competitor, seed_parallel_work_coverage
from measure import (
    OwnedContainerLedger,
    active_controlled_task_keys,
    cleanup_budget,
    cleanup_named_container,
    cleanup_path_if_safe,
    freeze_trial_for_oracle,
    observed_attempt_states,
    remaining_budget,
    run_command,
    run_foreground,
    reuse_fixture_data,
    resume_controlled_task_scheduling,
    suspend_controlled_task_scheduling,
    terminate_frozen_container,
    sqlite_schema_fingerprint,
    start_container,
    task_runs,
    competitor_file_oracle,
    validate_task_snapshot,
    wait_for_terminal_task_settlement,
    wait_for_task_evidence,
)


def docker_labels(command: list[str]) -> dict[str, str]:
    labels: dict[str, str] = {}
    for index, argument in enumerate(command[:-1]):
        if argument == "--label":
            key, value = command[index + 1].split("=", 1)
            labels[key] = value
    return labels


class MeasurementBudgetTests(unittest.TestCase):
    def test_oracle_freeze_pauses_owned_container_before_force_kill(self) -> None:
        run_id = "123456abcdef"
        container = f"retention-{run_id}-candidate-1"
        container_id = "a" * 64
        with tempfile.TemporaryDirectory() as temporary, patch("measure.RUN_ID", run_id):
            root = Path(temporary)

            owned = OwnedContainerLedger()
            owned.register(
                container,
                "application",
                root,
                root,
                root,
                version="candidate",
                trial_index=1,
            )
            owned.mark_started(container, container_id)
            labels = owned.record(container)["identity_labels"]
            commands: list[list[str]] = []

            def fake_docker(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                commands.append(command)
                if command[:3] == ["docker", "inspect", "--format"]:
                    return CompletedProcess(
                        command,
                        0,
                        f"{container_id} true {json.dumps(labels)}",
                        "",
                    )
                return CompletedProcess(command, 0, container_id, "")

            with patch("measure.run_command", side_effect=fake_docker):
                frozen_id = freeze_trial_for_oracle(
                    container,
                    owned,
                    time.monotonic(),
                    cleanup_reserve_seconds=5,
                )
                terminate_frozen_container(frozen_id, time.monotonic(), cleanup_reserve_seconds=5)

            self.assertEqual(
                [command[1] for command in commands],
                ["inspect", "pause", "unpause", "kill"],
            )

    def test_reuse_fixture_data_copies_only_controlled_fixture_tables(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source.sqlite"
            target = root / "target.sqlite"
            maintenance = root / "maintenance.sqlite"
            for database in (source, target):
                connection = sqlite3.connect(database)
                connection.executescript(
                    """
                    CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, value TEXT);
                    CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, value TEXT);
                    CREATE TABLE parallel_work_hourly_coverage (id INTEGER PRIMARY KEY, value TEXT);
                    CREATE TABLE target_only (id INTEGER PRIMARY KEY, value TEXT);
                    """
                )
                connection.commit()
                connection.close()

            connection = sqlite3.connect(source)
            connection.executescript(
                """
                INSERT INTO codex_invocations VALUES (1, 'source');
                INSERT INTO pool_upstream_request_attempts VALUES (1, 'source');
                INSERT INTO parallel_work_hourly_coverage VALUES (1, 'source');
                INSERT INTO target_only VALUES (1, 'source');
                """
            )
            connection.commit()
            connection.close()

            connection = sqlite3.connect(target)
            connection.executescript(
                """
                INSERT INTO codex_invocations VALUES (1, 'target');
                INSERT INTO target_only VALUES (1, 'target');
                """
            )
            connection.commit()
            connection.close()

            reuse_fixture_data(source, target, maintenance, time.monotonic())

            connection = sqlite3.connect(target)
            self.assertEqual(connection.execute("SELECT value FROM codex_invocations").fetchone()[0], "source")
            self.assertEqual(
                connection.execute("SELECT value FROM pool_upstream_request_attempts").fetchone()[0],
                "source",
            )
            self.assertEqual(
                connection.execute("SELECT value FROM parallel_work_hourly_coverage").fetchone()[0],
                "source",
            )
            self.assertEqual(connection.execute("SELECT value FROM target_only").fetchone()[0], "target")
            connection.close()

    def test_sqlite_schema_fingerprint_changes_when_schema_changes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            first = Path(temporary) / "first.sqlite"
            second = Path(temporary) / "second.sqlite"
            for database in (first, second):
                connection = sqlite3.connect(database)
                connection.execute("CREATE TABLE records (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
                connection.commit()
                connection.close()

            self.assertEqual(sqlite_schema_fingerprint(first), sqlite_schema_fingerprint(second))

            connection = sqlite3.connect(second)
            connection.execute("ALTER TABLE records ADD COLUMN note TEXT")
            connection.commit()
            connection.close()

            self.assertNotEqual(sqlite_schema_fingerprint(first), sqlite_schema_fingerprint(second))

    def test_competitor_chunks_and_file_phase_defers_are_counted_separately(self) -> None:
        rows = []
        for index, task_key in enumerate(("retention_archive", "pool_upstream_attempt_archive")):
            rows.extend([
                {
                    "task_key": task_key,
                    "trigger_kind": "schedule",
                    "status": "success",
                    "actual_started_at": f"2026-10-25T00:00:0{index}Z",
                    "actual_finished_at": f"2026-10-25T00:00:1{index}Z",
                    "committed_work": 100,
                    "details": {},
                },
                {
                    "task_key": task_key,
                    "trigger_kind": "catchup",
                    "status": "success",
                    "actual_started_at": f"2026-10-25T00:00:2{index}Z",
                    "actual_finished_at": f"2026-10-25T00:00:3{index}Z",
                    "committed_work": 100,
                    "details": {},
                },
            ])
        rows.extend([
            {
                "task_key": COMPETITOR_TASK,
                "trigger_kind": "schedule",
                "status": "skipped",
                "completion": "deferred",
                "actual_started_at": "2026-10-25T00:00:04Z",
                "actual_finished_at": "2026-10-25T00:00:05Z",
                "committed_work": None,
                "committed_bytes": None,
                "error_detail": None,
                "details": {
                    "admissionStage": "file_phase",
                    "waitReason": "file_phase_busy",
                    **{field: None for field in (
                        "filesChecked", "directoryEntriesChecked", "reconciliationRowsChecked",
                        "filesReleased", "bytesReleased", "referencedSkipped", "quarantined", "failures",
                    )},
                },
            },
            {
                "task_key": COMPETITOR_TASK,
                "trigger_kind": "schedule",
                "status": "skipped",
                "completion": "partial",
                "actual_started_at": "2026-10-25T00:00:06Z",
                "actual_finished_at": "2026-10-25T00:00:07Z",
                "committed_work": COMPETITOR_FILE_COUNT,
                "committed_bytes": COMPETITOR_FILE_COUNT * COMPETITOR_FILE_BYTES,
                "error_detail": None,
                "details": {
                    "filesReleased": COMPETITOR_FILE_COUNT,
                    "filesChecked": COMPETITOR_FILE_COUNT,
                    "failures": 0,
                },
            },
        ])

        validate_task_snapshot(rows)

    def test_recoverable_competitor_chunk_requires_later_success(self) -> None:
        rows = [
            {
                "task_key": task_key,
                "trigger_kind": trigger_kind,
                "status": "success",
                "actual_started_at": f"2026-10-25T00:00:{index:02d}Z",
                "actual_finished_at": f"2026-10-25T00:00:{index + 1:02d}Z",
                "committed_work": 100,
                "details": {},
            }
            for index, (task_key, trigger_kind) in enumerate(
                [
                    ("retention_archive", "schedule"),
                    ("retention_archive", "catchup"),
                    ("pool_upstream_attempt_archive", "schedule"),
                    ("pool_upstream_attempt_archive", "catchup"),
                ],
                start=1,
            )
        ]
        rows.extend([
            {
                "task_key": COMPETITOR_TASK,
                "trigger_kind": "schedule",
                "status": "skipped",
                "completion": "partial",
                "actual_started_at": "2026-10-25T00:00:05Z",
                "actual_finished_at": "2026-10-25T00:00:06Z",
                "committed_work": 1,
                "committed_bytes": COMPETITOR_FILE_BYTES,
                "error_detail": None,
                "details": {"filesReleased": 1, "filesChecked": 1, "failures": 1, "hasMore": True},
            },
            {
                "task_key": COMPETITOR_TASK,
                "trigger_kind": "schedule",
                "status": "success",
                "actual_started_at": "2026-10-25T00:00:07Z",
                "actual_finished_at": "2026-10-25T00:00:08Z",
                "committed_work": COMPETITOR_FILE_COUNT - 1,
                "committed_bytes": (COMPETITOR_FILE_COUNT - 1) * COMPETITOR_FILE_BYTES,
                "details": {"filesReleased": COMPETITOR_FILE_COUNT - 1, "filesChecked": COMPETITOR_FILE_COUNT - 1, "failures": 0},
            },
        ])

        with self.assertRaisesRegex(AcceptanceError, "fixed workload"):
            validate_task_snapshot(rows)
        validate_task_snapshot(rows, allow_recoverable_competitor_failures=True)

    def test_competitor_file_oracle_counts_only_the_fixed_cohort(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            raw_dir = Path(temporary)
            (raw_dir / "retention-orphan-0000.bin").write_bytes(b"x" * COMPETITOR_FILE_BYTES)
            (raw_dir / "unrelated.bin").write_bytes(b"x")
            oracle = competitor_file_oracle(raw_dir)

        self.assertEqual(oracle["expected_count"], COMPETITOR_FILE_COUNT)
        self.assertEqual(oracle["remaining_count"], 1)
        self.assertEqual(oracle["remaining_bytes"], COMPETITOR_FILE_BYTES)
        self.assertFalse(oracle["verified_empty"])

    def test_competitor_file_oracle_tolerates_a_concurrent_unlink(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            raw_dir = Path(temporary)
            orphan = raw_dir / "retention-orphan-0000.bin"
            orphan.write_bytes(b"x" * COMPETITOR_FILE_BYTES)
            orphan.unlink()

            oracle = competitor_file_oracle(raw_dir)

        self.assertEqual(oracle["remaining_count"], 0)
        self.assertEqual(oracle["remaining_bytes"], 0)
        self.assertTrue(oracle["verified_empty"])

    def test_seed_parallel_work_coverage_matches_the_retention_window(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            database = Path(temporary) / "coverage.sqlite"
            connection = sqlite3.connect(database)
            try:
                connection.execute(
                    """
                    CREATE TABLE parallel_work_hourly_coverage (
                        hour_start_epoch INTEGER NOT NULL,
                        source_scope TEXT NOT NULL,
                        minute_keys_complete INTEGER NOT NULL,
                        hourly_scalar_complete INTEGER NOT NULL,
                        PRIMARY KEY (hour_start_epoch, source_scope)
                    )
                    """
                )
                seed_parallel_work_coverage(connection, "2026-10-24 00:00:00")
                rows = connection.execute(
                    """
                    SELECT hour_start_epoch, source_scope, minute_keys_complete
                    FROM parallel_work_hourly_coverage
                    ORDER BY hour_start_epoch, source_scope
                    """
                ).fetchall()
            finally:
                connection.close()

        self.assertEqual(len(rows), 29 * 24 * 2)
        self.assertEqual({row[1] for row in rows}, {"all", "proxy_only"})
        self.assertEqual({row[2] for row in rows}, {1})
        self.assertEqual(rows[0][0], int(datetime(2026, 9, 24, 16, tzinfo=timezone.utc).timestamp()))
        self.assertEqual(rows[-1][0], int(datetime(2026, 10, 23, 15, tzinfo=timezone.utc).timestamp()))

    def test_task_runs_keeps_running_owner_without_persisted_actual_start(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            database = Path(temporary) / "maintenance.sqlite"
            connection = sqlite3.connect(database)
            try:
                connection.execute(
                    "CREATE TABLE managed_task_progress (task_key TEXT, total INTEGER, completed INTEGER, phase TEXT, checkpoint TEXT, eta_seconds REAL, updated_at TEXT, freshness TEXT, unit TEXT, source_scope TEXT, last_progress_at TEXT, wait_reason TEXT, next_retry_at TEXT, next_inspection_at TEXT, next_catchup_at TEXT, catchup_state TEXT, stages TEXT)"
                )
                connection.execute(
                    "CREATE TABLE managed_task_runs (id INTEGER PRIMARY KEY, task_key TEXT, trigger_kind TEXT, status TEXT, started_at TEXT, finished_at TEXT, actual_started_at TEXT, actual_finished_at TEXT, processed_count INTEGER, updated_count INTEGER, details TEXT, summary TEXT, error_detail TEXT, completion TEXT)"
                )
                connection.execute(
                    "INSERT INTO managed_task_runs VALUES (1,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                    (
                        "retention_archive",
                        "schedule",
                        "running",
                        "2026-10-25T00:00:00Z",
                        None,
                        None,
                        None,
                        0,
                        0,
                        "{}",
                        None,
                        None,
                        None,
                    ),
                )
                connection.execute(
                    "INSERT INTO managed_task_progress VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                    (
                        "retention_archive",
                        1000,
                        0,
                        "running",
                        "admitted",
                        None,
                        "2026-10-25T00:00:02Z",
                        "fresh",
                        "invocation rows",
                        "expired_invocations:retention_policy",
                        "2026-10-25T00:00:02Z",
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ),
                )
                connection.commit()
            finally:
                connection.close()

            rows = task_runs(database)

        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["status"], "running")
        self.assertIsNone(rows[0]["actual_started_at"])
        self.assertEqual(rows[0]["started_at"], "2026-10-25T00:00:00Z")
        self.assertEqual(rows[0]["activity_started_at"], "2026-10-25T00:00:02Z")

    def test_terminal_task_settlement_waits_for_actual_finish_timestamp(self) -> None:
        unsettled = [{
            "task_key": "pool_upstream_attempt_archive",
            "status": "skipped",
            "finished_at": "2026-10-25T00:00:02Z",
            "actual_finished_at": None,
        }]
        settled = [{
            **unsettled[0],
            "actual_finished_at": "2026-10-25T00:00:03Z",
        }]
        with patch("measure.task_runs", side_effect=[settled]) as runs, patch("measure.time.sleep") as sleep:
            result = wait_for_terminal_task_settlement(
                Path("maintenance.sqlite"),
                time.monotonic(),
                initial_rows=unsettled,
            )

        self.assertEqual(result, settled)
        runs.assert_called_once_with(Path("maintenance.sqlite"))
        sleep.assert_called_once_with(0.1)

    def test_legacy_partial_status_is_accepted_when_selected_batch_is_complete(self) -> None:
        archive_details = {
            "result": "partial",
            "fatalError": None,
            "errorChain": None,
            "timeoutCount": 0,
            "selectedScope": {"selected": 1000, "committed": 1000},
            "archiveBatches": [{"batchRows": 1000, "committedRows": 1000, "timeoutCount": 0}],
        }
        rows = [
            {
                "task_key": task_key,
                "trigger_kind": trigger_kind,
                "status": "skipped" if task_key == "retention_archive" and trigger_kind == "schedule" else "success",
                "completion": "partial" if task_key == "retention_archive" and trigger_kind == "schedule" else "complete",
                "actual_started_at": f"2026-10-25T00:00:{index:02d}Z",
                "actual_finished_at": f"2026-10-25T00:00:{index + 1:02d}Z",
                "processed_count": 1000 if task_key != COMPETITOR_TASK else None,
                "committed_work": 512 if task_key == COMPETITOR_TASK else 1000,
                "details": archive_details if task_key == "retention_archive" and trigger_kind == "schedule" else {},
                "error_detail": None,
            }
            for index, (task_key, trigger_kind) in enumerate(
                [
                    ("retention_archive", "schedule"),
                    ("retention_archive", "catchup"),
                    ("pool_upstream_attempt_archive", "schedule"),
                    ("pool_upstream_attempt_archive", "catchup"),
                    (COMPETITOR_TASK, "schedule"),
                ],
                start=1,
            )
        ]

        validate_task_snapshot(rows)

    def test_partial_status_is_rejected_when_selected_batch_is_incomplete(self) -> None:
        rows = [
            {
                "task_key": "retention_archive",
                "trigger_kind": "schedule",
                "status": "skipped",
                "completion": "partial",
                "actual_started_at": "2026-10-25T00:00:00Z",
                "actual_finished_at": "2026-10-25T00:00:01Z",
                "details": {
                    "result": "partial",
                    "selectedScope": {"selected": 1000, "committed": 999},
                    "archiveBatches": [{"batchRows": 1000, "committedRows": 999, "timeoutCount": 0}],
                },
                "error_detail": None,
            }
        ]

        with self.assertRaisesRegex(AcceptanceError, "both schedule and catch-up"):
            validate_task_snapshot(rows)

    def test_baseline_accepts_pressure_bounded_schedule_before_catchup(self) -> None:
        partial_details = {
            "result": "partial",
            "fatalError": None,
            "errorChain": None,
            "timeoutCount": 0,
            "backlogRemaining": 99186,
            "selectedScope": {"selected": 1000, "committed": 814, "hasMore": True},
            "archiveBatches": [{"batchRows": 1000, "committedRows": 814, "timeoutCount": 0}],
        }
        rows = [
            {
                "task_key": "retention_archive",
                "trigger_kind": "schedule",
                "status": "skipped",
                "completion": "partial",
                "actual_started_at": "2026-10-25T00:00:00Z",
                "actual_finished_at": "2026-10-25T00:00:01Z",
                "committed_work": 814,
                "details": partial_details,
                "error_detail": None,
            },
            {
                "task_key": "retention_archive",
                "trigger_kind": "catchup",
                "status": "success",
                "completion": "complete",
                "actual_started_at": "2026-10-25T00:00:02Z",
                "actual_finished_at": "2026-10-25T00:00:03Z",
                "committed_work": 1000,
                "details": {},
                "error_detail": None,
            },
            {
                "task_key": "pool_upstream_attempt_archive",
                "trigger_kind": "schedule",
                "status": "success",
                "completion": "complete",
                "actual_started_at": "2026-10-25T00:00:00Z",
                "actual_finished_at": "2026-10-25T00:00:01Z",
                "committed_work": 1000,
                "details": {},
                "error_detail": None,
            },
            {
                "task_key": "pool_upstream_attempt_archive",
                "trigger_kind": "catchup",
                "status": "success",
                "completion": "complete",
                "actual_started_at": "2026-10-25T00:00:02Z",
                "actual_finished_at": "2026-10-25T00:00:03Z",
                "committed_work": 1000,
                "details": {},
                "error_detail": None,
            },
            {
                "task_key": COMPETITOR_TASK,
                "trigger_kind": "schedule",
                "status": "success",
                "completion": "completed",
                "actual_started_at": "2026-10-25T00:00:04Z",
                "actual_finished_at": "2026-10-25T00:00:05Z",
                "committed_work": COMPETITOR_FILE_COUNT,
                "committed_bytes": COMPETITOR_FILE_COUNT * COMPETITOR_FILE_BYTES,
                "details": {"filesReleased": COMPETITOR_FILE_COUNT, "failures": 0},
                "error_detail": None,
            },
        ]

        validate_task_snapshot(rows, "baseline")
        with self.assertRaisesRegex(AcceptanceError, "both schedule and catch-up"):
            validate_task_snapshot(rows, "candidate")

    def test_baseline_bounded_active_owner_is_accepted_only_with_progress(self) -> None:
        progress = {
            "phase": "running",
            "checkpoint": "admitted",
            "last_progress_at": "2026-10-25T00:00:02Z",
        }
        rows = [
            {
                "task_key": task_key,
                "trigger_kind": "schedule",
                "status": "running",
                "started_at": "2026-10-25T00:00:00Z",
                "actual_started_at": "2026-10-25T00:00:01Z",
                "actual_finished_at": None,
                "committed_work": 0,
                "evidence_kind": "bounded_active",
                "activity_started_at": "2026-10-25T00:00:01Z",
                "observed_at": "2026-10-25T00:00:10Z",
                "progress": progress,
            }
            for task_key in ("retention_archive", "pool_upstream_attempt_archive")
        ]
        rows.append({
            "task_key": COMPETITOR_TASK,
            "trigger_kind": "schedule",
            "status": "success",
            "actual_started_at": "2026-10-25T00:00:03Z",
            "actual_finished_at": "2026-10-25T00:00:04Z",
            "committed_work": 512,
        })

        validate_task_snapshot(rows, "baseline")
        with self.assertRaisesRegex(AcceptanceError, "schedule and catch-up"):
            validate_task_snapshot(rows)

        rows[0]["progress"] = None
        with self.assertRaisesRegex(AcceptanceError, "schedule and catch-up"):
            validate_task_snapshot(rows, "baseline")

    def test_baseline_evidence_uses_reference_readiness_without_summary_proof(self) -> None:
        proof_status = {
            "ready": False,
            "reference_ready": True,
            "datasets": {},
        }
        frozen: list[dict[str, object]] = []
        with patch("measure.task_runs", return_value=[]), patch(
            "measure.validate_task_snapshot"
        ), patch("measure.archive_proof_status", return_value=proof_status) as proof:
            with patch("measure.suspend_controlled_task_scheduling"):
                self.assertEqual(
                    wait_for_task_evidence(
                        Path("main.sqlite"),
                        Path("maintenance.sqlite"),
                        "2026-10-23 00:00:00",
                        {name: 1 for name in DATASETS},
                        time.monotonic(),
                        "baseline",
                        timeout_seconds=1,
                        on_proof_ready=lambda status: frozen.append(status),
                    ),
                    [],
                )
        proof.assert_called_once_with(
            Path("main.sqlite"),
            "2026-10-23 00:00:00",
            {name: 1 for name in DATASETS},
            require_summary_proof=False,
        )
        self.assertEqual(frozen, [proof_status])

    def test_evidence_waits_for_controlled_owner_quiescence_before_freeze(self) -> None:
        proof_status = {
            "ready": False,
            "reference_ready": True,
            "datasets": {},
        }
        active_row = {
            "task_key": "pool_upstream_attempt_archive",
            "status": "running",
            "actual_finished_at": None,
        }
        frozen: list[dict[str, object]] = []
        with patch("measure.task_runs", side_effect=[[active_row], []]) as runs, patch(
            "measure.validate_task_snapshot"
        ), patch("measure.archive_proof_status", return_value=proof_status), patch(
            "measure.time.sleep"
        ) as sleep, patch("measure.suspend_controlled_task_scheduling") as suspend:
            suspend.side_effect = lambda _database, _started: self.assertEqual(runs.call_count, 1)
            self.assertEqual(
                wait_for_task_evidence(
                    Path("main.sqlite"),
                    Path("maintenance.sqlite"),
                    "2026-10-23 00:00:00",
                    {name: 1 for name in DATASETS},
                    time.monotonic(),
                    "baseline",
                    timeout_seconds=1,
                    on_proof_ready=lambda status: frozen.append(status),
                ),
                [],
            )

        self.assertEqual(runs.call_count, 2)
        suspend.assert_called_once_with(Path("maintenance.sqlite"), ANY)
        sleep.assert_called_once_with(0.5)
        self.assertEqual(frozen, [proof_status])
        self.assertEqual(active_controlled_task_keys([active_row]), ["pool_upstream_attempt_archive"])

    def test_evidence_does_not_freeze_active_owner_on_transient_proof_edge(self) -> None:
        ready = {"ready": False, "reference_ready": True, "datasets": {}}
        incomplete = {"ready": False, "reference_ready": False, "datasets": {}}
        active_row = {
            "task_key": "pool_upstream_attempt_archive",
            "status": "running",
            "actual_finished_at": None,
        }
        frozen: list[dict[str, object]] = []
        with patch("measure.task_runs", side_effect=[[active_row], [active_row], []]), patch(
            "measure.validate_task_snapshot",
            side_effect=[
                None,
                AcceptanceError(
                    "production dispatcher has not completed both schedule and catch-up runs "
                    "for pool_upstream_attempt_archive"
                ),
                None,
            ],
        ), patch("measure.archive_proof_status", side_effect=[ready, incomplete, ready]), patch(
            "measure.time.sleep"
        ), patch("measure.suspend_controlled_task_scheduling") as suspend, patch(
            "measure.resume_controlled_task_scheduling"
        ) as resume:
            self.assertEqual(
                wait_for_task_evidence(
                    Path("main.sqlite"),
                    Path("maintenance.sqlite"),
                    "2026-10-23 00:00:00",
                    {name: 1 for name in DATASETS},
                    time.monotonic(),
                    "baseline",
                    timeout_seconds=1,
                    on_proof_ready=lambda status: frozen.append(status),
                ),
                [],
            )

        self.assertEqual(suspend.call_count, 1)
        suspend.assert_called_once_with(Path("maintenance.sqlite"), ANY)
        resume.assert_not_called()
        self.assertEqual(frozen, [ready])

    def test_candidate_freezes_on_proven_catchup_progress_at_proof_edge(self) -> None:
        proof_status = {
            "ready": True,
            "reference_ready": True,
            "datasets": {},
        }
        active_row = {
            "task_key": "pool_upstream_attempt_archive",
            "trigger_kind": "catchup",
            "status": "running",
            "started_at": "2026-10-25T00:00:00Z",
            "actual_started_at": "2026-10-25T00:00:01Z",
            "actual_finished_at": None,
            "activity_started_at": "2026-10-25T00:00:01Z",
            "observed_at": "2026-10-25T00:00:10Z",
            "committed_work": 1000,
            "evidence_kind": "active",
            "progress": {
                "total": 1000,
                "completed": 0,
                "phase": "running",
                "checkpoint": "admitted",
                "freshness": "fresh",
                "wait_reason": "retention_write_admission",
                "last_progress_at": "2026-10-25T00:00:09Z",
            },
        }
        frozen: list[dict[str, object]] = []
        with patch("measure.task_runs", return_value=[active_row]), patch(
            "measure.validate_task_snapshot",
            side_effect=[AcceptanceError("catch-up is still active"), None],
        ) as validate, patch(
            "measure.archive_proof_status", return_value=proof_status
        ), patch("measure.suspend_controlled_task_scheduling") as suspend:
            result = wait_for_task_evidence(
                Path("main.sqlite"),
                Path("maintenance.sqlite"),
                "2026-10-23 00:00:00",
                {name: 1 for name in DATASETS},
                time.monotonic(),
                "candidate",
                timeout_seconds=1,
                on_proof_ready=lambda status: frozen.append(status),
            )

        self.assertEqual(result[0]["evidence_kind"], "proof_edge_active")
        self.assertEqual(frozen, [proof_status])
        suspend.assert_called_once_with(Path("maintenance.sqlite"), ANY)
        self.assertEqual(validate.call_count, 2)
        self.assertEqual(validate.call_args.args[0][0]["evidence_kind"], "proof_edge_active")
        self.assertTrue(validate.call_args.kwargs["allow_proof_edge_active"])

    def test_candidate_freezes_on_proven_catchup_waiting_at_proof_edge(self) -> None:
        proof_status = {
            "ready": True,
            "reference_ready": True,
            "datasets": {},
        }
        active_row = {
            "task_key": "pool_upstream_attempt_archive",
            "trigger_kind": "catchup",
            "status": "running",
            "started_at": "2026-10-25T00:00:00Z",
            "actual_started_at": "2026-10-25T00:00:01Z",
            "actual_finished_at": None,
            "activity_started_at": "2026-10-25T00:00:01Z",
            "observed_at": "2026-10-25T00:00:10Z",
            "committed_work": 1000,
            "evidence_kind": "active",
            "progress": {
                "total": 1000,
                "completed": 0,
                "phase": "waiting",
                "checkpoint": "waiting",
                "freshness": "fresh",
                "wait_reason": "retention_write_admission",
                "last_progress_at": "2026-10-25T00:00:09Z",
            },
        }
        frozen: list[dict[str, object]] = []
        with patch("measure.task_runs", return_value=[active_row]), patch(
            "measure.validate_task_snapshot",
            side_effect=[AcceptanceError("catch-up is still active"), None],
        ) as validate, patch(
            "measure.archive_proof_status", return_value=proof_status
        ), patch("measure.suspend_controlled_task_scheduling") as suspend:
            result = wait_for_task_evidence(
                Path("main.sqlite"),
                Path("maintenance.sqlite"),
                "2026-10-23 00:00:00",
                {name: 1 for name in DATASETS},
                time.monotonic(),
                "candidate",
                timeout_seconds=1,
                on_proof_ready=lambda status: frozen.append(status),
            )

        self.assertEqual(result[0]["evidence_kind"], "proof_edge_active")
        self.assertEqual(frozen, [proof_status])
        suspend.assert_called_once_with(Path("maintenance.sqlite"), ANY)
        self.assertEqual(validate.call_count, 2)
        self.assertTrue(validate.call_args.kwargs["allow_proof_edge_active"])

    def test_proof_edge_suspends_archive_before_competitor_files_are_empty(self) -> None:
        proof_status = {
            "ready": False,
            "reference_ready": True,
            "datasets": {},
        }
        frozen: list[dict[str, object]] = []
        with patch("measure.task_runs", side_effect=[[], []]), patch(
            "measure.validate_task_snapshot"
        ), patch("measure.archive_proof_status", return_value=proof_status), patch(
            "measure.competitor_file_oracle",
            side_effect=[
                {"verified_empty": False},
                {"verified_empty": True},
            ],
        ), patch("measure.time.sleep"), patch(
            "measure.suspend_controlled_task_scheduling"
        ) as suspend:
            result = wait_for_task_evidence(
                Path("main.sqlite"),
                Path("maintenance.sqlite"),
                "2026-10-23 00:00:00",
                {name: 1 for name in DATASETS},
                time.monotonic(),
                "baseline",
                timeout_seconds=1,
                competitor_raw_dir=Path("raw"),
                on_proof_ready=lambda status: frozen.append(status),
            )

        self.assertEqual(result, [])
        suspend.assert_called_once_with(Path("maintenance.sqlite"), ANY)
        self.assertEqual(frozen, [proof_status])

    def test_suspend_controlled_task_scheduling_only_changes_controlled_rows(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            database = Path(temporary) / "maintenance.sqlite"
            connection = sqlite3.connect(database)
            try:
                connection.execute(
                    "CREATE TABLE managed_tasks ("
                    "task_key TEXT PRIMARY KEY, enabled INTEGER, next_trigger_at TEXT, "
                    "next_catchup_at TEXT, catchup_reason TEXT, updated_at TEXT)"
                )
                rows = [
                    (key, 1, "old-trigger", "old-catchup", "old-reason", "old-updated")
                    for key in (*CONTROLLED_TASK_KEYS, "unrelated_task")
                ]
                connection.executemany("INSERT INTO managed_tasks VALUES (?,?,?,?,?,?)", rows)
                connection.commit()
            finally:
                connection.close()

            started = time.monotonic()
            suspend_controlled_task_scheduling(database, started)

            connection = sqlite3.connect(database)
            try:
                values = {
                    row[0]: row[1:]
                    for row in connection.execute(
                        "SELECT task_key,enabled,next_trigger_at,next_catchup_at,catchup_reason,updated_at "
                        "FROM managed_tasks"
                    )
                }
            finally:
                connection.close()

            for key in ARCHIVE_TASK_KEYS:
                self.assertEqual(values[key][:4], (0, None, None, None))
                self.assertNotEqual(values[key][4], "old-updated")
            self.assertEqual(values[COMPETITOR_TASK], (1, "old-trigger", "old-catchup", "old-reason", "old-updated"))
            self.assertEqual(values["unrelated_task"], (1, "old-trigger", "old-catchup", "old-reason", "old-updated"))

            resume_controlled_task_scheduling(database, started, (COMPETITOR_TASK,))

            connection = sqlite3.connect(database)
            try:
                values = {
                    row[0]: row[1:]
                    for row in connection.execute(
                        "SELECT task_key,enabled,next_trigger_at,next_catchup_at,catchup_reason,updated_at "
                        "FROM managed_tasks"
                    )
                }
            finally:
                connection.close()

            self.assertEqual(values[COMPETITOR_TASK][0], 1)
            self.assertIsNotNone(values[COMPETITOR_TASK][1])
            self.assertEqual(values[COMPETITOR_TASK][2:4], (None, None))
            for key in ARCHIVE_TASK_KEYS:
                self.assertEqual(values[key][:4], (0, None, None, None))
                self.assertNotEqual(values[key][4], "old-updated")
            self.assertNotEqual(values[COMPETITOR_TASK][4], "old-updated")
            self.assertEqual(values["unrelated_task"], (1, "old-trigger", "old-catchup", "old-reason", "old-updated"))

            resume_controlled_task_scheduling(database, started)

            connection = sqlite3.connect(database)
            try:
                values = {
                    row[0]: row[1:]
                    for row in connection.execute(
                        "SELECT task_key,enabled,next_trigger_at,next_catchup_at,catchup_reason,updated_at "
                        "FROM managed_tasks"
                    )
                }
            finally:
                connection.close()

            for key in CONTROLLED_TASK_KEYS:
                self.assertEqual(values[key][0], 1)
                self.assertIsNotNone(values[key][1])
                self.assertEqual(values[key][2], None)
                self.assertEqual(values[key][3], None)
                self.assertNotEqual(values[key][4], "old-updated")
            self.assertEqual(values["unrelated_task"], (1, "old-trigger", "old-catchup", "old-reason", "old-updated"))

    def test_prepare_competitor_isolates_the_controlled_task_set(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            main_database = root / "main.sqlite"
            maintenance_database = root / "maintenance.sqlite"
            raw_directory = root / "raw"
            main = sqlite3.connect(main_database)
            maintenance = sqlite3.connect(maintenance_database)
            try:
                main.execute(
                    """
                    CREATE TABLE retention_raw_reconciliation (
                        raw_path TEXT,
                        file_identity TEXT,
                        byte_size INTEGER,
                        quarantined_at TEXT,
                        release_pending INTEGER,
                        removal_evidence_persisted INTEGER
                    )
                    """
                )
                maintenance.execute(
                    """
                    CREATE TABLE managed_tasks (
                        task_key TEXT PRIMARY KEY,
                        enabled INTEGER NOT NULL,
                        next_trigger_at TEXT,
                        next_catchup_at TEXT,
                        catchup_reason TEXT,
                        updated_at TEXT NOT NULL
                    )
                    """
                )
                maintenance.execute("CREATE TABLE managed_task_runs (id INTEGER)")
                maintenance.execute("CREATE TABLE managed_task_work_runs (id INTEGER)")
                maintenance.execute(
                    """
                    CREATE TABLE maintenance_owner_retry_state (
                        task_key TEXT PRIMARY KEY,
                        failure_count INTEGER NOT NULL DEFAULT 0,
                        next_eligible_at TEXT,
                        reason TEXT
                    )
                    """
                )
                maintenance.execute(
                    """
                    CREATE TABLE startup_backfill_progress (
                        task_name TEXT PRIMARY KEY,
                        enabled INTEGER NOT NULL,
                        next_run_after TEXT,
                        next_probe_at TEXT
                    )
                    """
                )
                maintenance.executemany(
                    "INSERT INTO startup_backfill_progress VALUES (?,?,?,?)",
                    [
                        ("historical_rollups", 1, "old-run", "old-probe"),
                        ("prompt_cache_key", 1, "old-run", "old-probe"),
                    ],
                )
                task_rows = [
                    (key, 1, "old-trigger", "old-catchup", "old-reason", "old-updated")
                    for key in (
                        *CONTROLLED_TASK_KEYS,
                        "summary_snapshot",
                        "long_term_projection",
                    )
                ]
                maintenance.executemany(
                    "INSERT INTO managed_tasks VALUES (?,?,?,?,?,?)",
                    task_rows,
                )
                maintenance.executemany(
                    "INSERT INTO maintenance_owner_retry_state(task_key,failure_count,next_eligible_at,reason) VALUES(?,?,?,?)",
                    [
                        (CONTROLLED_TASK_KEYS[0], 3, "2099-01-01T00:00:00.000Z", "sqlite_pressure"),
                        ("summary_snapshot", 2, "2099-01-01T00:00:00.000Z", "retry_backoff"),
                    ],
                )
                main.commit()
                maintenance.commit()
            finally:
                main.close()
                maintenance.close()

            prepare_competitor(
                main_database,
                maintenance_database,
                raw_directory,
                "/data/proxy_raw_payloads",
                count=1,
            )

            connection = sqlite3.connect(maintenance_database)
            try:
                rows = {
                    row[0]: row[1:]
                    for row in connection.execute(
                        "SELECT task_key,enabled,next_trigger_at,next_catchup_at,catchup_reason FROM managed_tasks"
                    )
                }
                retry_rows = {
                    row[0]: row[1:]
                    for row in connection.execute(
                        "SELECT task_key,failure_count,next_eligible_at,reason FROM maintenance_owner_retry_state"
                    )
                }
            finally:
                connection.close()

            self.assertEqual(
                set(CONTROLLED_TASK_KEYS),
                {DATASETS[name]["task_key"] for name in DATASETS} | {COMPETITOR_TASK},
            )
            self.assertEqual(set(CONTROLLED_TASK_KEYS), {
                key for key, (enabled, next_trigger, next_catchup, _) in rows.items()
                if enabled == 1 and next_trigger is not None and next_catchup is None
            })
            target_triggers = [
                rows[DATASETS[name]["task_key"]][1]
                for name in DATASETS
            ]
            competitor_trigger = rows[COMPETITOR_TASK][1]
            target_time = datetime.fromisoformat(target_triggers[0].replace("Z", "+00:00"))
            competitor_time = datetime.fromisoformat(competitor_trigger.replace("Z", "+00:00"))
            self.assertTrue(all(trigger < competitor_trigger for trigger in target_triggers))
            self.assertGreaterEqual(
                (competitor_time - target_time).total_seconds(),
                COMPETITOR_START_DELAY_SECONDS - 1,
            )
            self.assertEqual(
                rows["summary_snapshot"],
                (0, None, None, None),
            )
            self.assertEqual(
                rows["long_term_projection"],
                (0, None, None, None),
            )
            self.assertNotIn(CONTROLLED_TASK_KEYS[0], retry_rows)
            self.assertEqual(
                retry_rows["summary_snapshot"],
                (2, "2099-01-01T00:00:00.000Z", "retry_backoff"),
            )
            progress_connection = sqlite3.connect(maintenance_database)
            try:
                progress_rows = progress_connection.execute(
                    "SELECT enabled,next_run_after,next_probe_at "
                    "FROM startup_backfill_progress ORDER BY task_name"
                ).fetchall()
            finally:
                progress_connection.close()
            self.assertEqual(progress_rows, [(0, None, None), (0, None, None)])

    def test_observed_attempt_states_counts_rows_from_five_column_query(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            database = Path(temporary) / "attempts.sqlite"
            connection = sqlite3.connect(database)
            try:
                connection.execute(
                    """
                    CREATE TABLE pool_upstream_request_attempts (
                        id INTEGER PRIMARY KEY,
                        attempt_public_id TEXT NOT NULL,
                        invoke_id TEXT NOT NULL,
                        attempt_index INTEGER NOT NULL,
                        status TEXT NOT NULL,
                        finished_at TEXT NOT NULL
                    )
                    """
                )
                connection.execute(
                    """
                    INSERT INTO pool_upstream_request_attempts
                        (attempt_public_id, invoke_id, attempt_index, status, finished_at)
                    VALUES (?, ?, ?, ?, ?)
                    """,
                    ("attempt-1", "invoke-1", 1, "success", "2026-10-25T00:00:00Z"),
                )
                connection.commit()
            finally:
                connection.close()

            states = observed_attempt_states(
                database,
                [{"invoke_id": "invoke-1", "sequence": 0}],
                retry_count=0,
                started=time.monotonic(),
            )

            self.assertEqual(states[0]["attempt_public_id"], "attempt-1")
            self.assertEqual(states[0]["invoke_id"], "invoke-1")

    def test_foreground_driver_lifecycle_uses_unique_trial_names_on_one_ledger(self) -> None:
        run_id = "123456abcdef"
        owned = OwnedContainerLedger()
        started = time.monotonic()
        run_names: list[str] = []
        inspect_names: list[str] = []
        labels_by_name: dict[str, dict[str, str]] = {}
        ids_by_name: dict[str, str] = {}

        def fake_docker(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
            if command[0:2] == ["docker", "run"]:
                name = command[command.index("--name") + 1]
                run_names.append(name)
                labels = {
                    command[index + 1].split("=", 1)[0]: command[index + 1].split("=", 1)[1]
                    for index, argument in enumerate(command[:-1])
                    if argument == "--label"
                }
                labels_by_name[name] = labels
                ids_by_name[name] = f"{len(run_names):064x}"
                return CompletedProcess(command, 0, "{}", "")
            if command[0:2] == ["docker", "rm"]:
                container_id = command[-1]
                name = next(name for name, item in ids_by_name.items() if item == container_id)
                del ids_by_name[name]
                del labels_by_name[name]
                return CompletedProcess(command, 0, container_id, "")
            if command[0:2] == ["docker", "inspect"]:
                target = command[-1]
                name = target if target in ids_by_name else next(
                    (name for name, item in ids_by_name.items() if item == target), None
                )
                if name is None:
                    return CompletedProcess(command, 1, "", f"Error: No such object: {target}")
                if target == name:
                    inspect_names.append(target)
                return CompletedProcess(
                    command,
                    0,
                    f"{ids_by_name[name]} false {json.dumps(labels_by_name[name])}",
                    "",
                )
            self.fail(f"unexpected fake Docker command: {command[1]}")

        expected_names = [
            f"retention-{run_id}-{step['version']}-{step['trial_index']}-driver"
            for step in TRIAL_EXECUTION_ORDER
        ]
        with patch("measure.RUN_ID", run_id), patch("measure.run_command", side_effect=fake_docker):
            for step in TRIAL_EXECUTION_ORDER:
                version = step["version"]
                index = step["trial_index"]
                trial_dir = Path(tempfile.gettempdir()) / f"retention-test-{version}-{index}"
                cleanup: dict[str, object] = {}
                run_foreground(
                    "retention-application",
                    "sha256:" + "1" * 64,
                    started,
                    version,
                    index,
                    cleanup,
                    owned=owned,
                    root=trial_dir.parent,
                    recovery_path=trial_dir,
                )
                self.assertEqual(cleanup["container_name"], expected_names[len(run_names) - 1])
                self.assertEqual(cleanup["status"], "removed")
                self.assertEqual(cleanup["container_id"], f"{len(run_names):064x}")
                self.assertEqual(cleanup["identity_labels"], owned.record(expected_names[len(run_names) - 1])["identity_labels"])

        self.assertEqual(run_names, expected_names)
        self.assertEqual(inspect_names, expected_names)
        records = owned.records()
        self.assertEqual(
            [record["container_name"] for record in records],
            sorted(expected_names),
        )
        self.assertEqual([record["status"] for record in records], ["removed"] * 6)

    def test_init_and_application_creation_labels_bind_run_and_trial_identity(self) -> None:
        run_id = "123456abcdef"
        with tempfile.TemporaryDirectory() as temporary, patch("measure.RUN_ID", run_id):
            for kind, version, trial_index in (
                ("init", "baseline", None),
                ("application", "candidate", 2),
            ):
                owned = OwnedContainerLedger()
                data_dir = Path(temporary) / f"{kind}-{version}"
                name = (
                    f"retention-{run_id}-init-{version}"
                    if trial_index is None
                    else f"retention-{run_id}-{version}-{trial_index}"
                )
                command_seen: list[list[str]] = []

                def docker_run(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                    command_seen.append(command)
                    return CompletedProcess(command, 0, "a" * 64, "")

                with patch("measure.run_command", side_effect=docker_run):
                    result = start_container(
                        "test-image",
                        data_dir,
                        name,
                        time.monotonic(),
                        owned=owned,
                        kind=kind,
                        root=Path(temporary),
                        version=version,
                        trial_index=trial_index,
                    )

                self.assertEqual(result, "a" * 64)
                self.assertEqual(
                    command_seen[0][command_seen[0].index("--user") + 1],
                    f"{os.getuid()}:{os.getgid()}",
                )
                self.assertEqual(docker_labels(command_seen[0]), owned.record(name)["identity_labels"])
                identity = owned.record(name)["identity_labels"]
                self.assertEqual(identity[CONTAINER_RUN_ID_LABEL], run_id)
                self.assertEqual(identity[CONTAINER_KIND_LABEL], kind)
                self.assertEqual(
                    identity[CONTAINER_TRIAL_LABEL],
                    "init" if trial_index is None else str(trial_index),
                )

    def test_unconfirmed_name_collisions_preserve_foreign_init_app_and_driver_containers(self) -> None:
        run_id = "123456abcdef"
        foreign_id = "f" * 64
        with tempfile.TemporaryDirectory() as temporary, patch("measure.RUN_ID", run_id):
            for kind in ("init", "application", "foreground"):
                with self.subTest(kind=kind):
                    root = Path(temporary) / kind
                    recovery_path = root / "trial"
                    recovery_path.mkdir(parents=True)
                    sentinel = recovery_path / "database.sqlite"
                    sentinel.write_bytes(b"preserve while container ownership is unknown")
                    owned = OwnedContainerLedger()
                    trial_index = None if kind == "init" else 1
                    version = "baseline"
                    name = (
                        f"retention-{run_id}-init-{version}"
                        if kind == "init"
                        else f"retention-{run_id}-{version}-{trial_index}"
                        + ("-driver" if kind == "foreground" else "")
                    )
                    commands: list[list[str]] = []

                    def docker_conflict(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                        commands.append(command)
                        if command[0:2] == ["docker", "run"]:
                            raise RuntimeError("container name already exists")
                        if command[0:2] == ["docker", "inspect"]:
                            labels = dict(owned.record(name)["identity_labels"])
                            labels[CONTAINER_RUN_ID_LABEL] = "fedcba987654"
                            labels[CONTAINER_OWNER_LABEL] = "e" * 32
                            return CompletedProcess(
                                command,
                                0,
                                f"{foreign_id} true {json.dumps(labels)}",
                                "",
                            )
                        self.fail(f"foreign container must not be stopped or removed: {command}")

                    with patch("measure.run_command", side_effect=docker_conflict), patch(
                        "measure.bounded_rmtree"
                    ) as remove:
                        if kind == "foreground":
                            cleanup: dict[str, object] = {}
                            with self.assertRaisesRegex(AcceptanceError, "unexpected run identity"):
                                run_foreground(
                                    "retention-application",
                                    "sha256:" + "1" * 64,
                                    time.monotonic(),
                                    version,
                                    1,
                                    cleanup,
                                    owned=owned,
                                    root=root,
                                    recovery_path=recovery_path,
                                )
                        else:
                            with self.assertRaisesRegex(RuntimeError, "container name already exists"):
                                start_container(
                                    "test-image",
                                    recovery_path,
                                    name,
                                    time.monotonic(),
                                    owned=owned,
                                    kind=kind,
                                    root=root,
                                    version=version,
                                    trial_index=trial_index,
                                )
                            result, errors = cleanup_path_if_safe(
                                recovery_path,
                                root,
                                owned,
                                time.monotonic(),
                                cleanup_reserve_seconds=5,
                            )
                            self.assertFalse(result["removed"])
                            self.assertTrue(errors)

                    self.assertTrue(sentinel.exists())
                    self.assertEqual([command[1] for command in commands if command[0] == "docker"], ["run", "inspect"])
                    remove.assert_not_called()

    def test_creation_timeouts_recover_owned_init_application_and_driver_containers(self) -> None:
        run_id = "123456abcdef"
        container_id = "a" * 64
        with tempfile.TemporaryDirectory() as temporary, patch("measure.RUN_ID", run_id):
            for kind in ("init", "application", "foreground"):
                with self.subTest(kind=kind):
                    root = Path(temporary) / kind
                    recovery_path = root / "recovery"
                    recovery_path.mkdir(parents=True)
                    sentinel = recovery_path / "database.sqlite"
                    sentinel.write_bytes(b"recover until confirmed cleanup")
                    version = "baseline"
                    trial_index = None if kind == "init" else 1
                    name = (
                        f"retention-{run_id}-init-{version}"
                        if kind == "init"
                        else f"retention-{run_id}-{version}-{trial_index}"
                        + ("-driver" if kind == "foreground" else "")
                    )
                    owned = OwnedContainerLedger()
                    commands: list[list[str]] = []
                    removed = False

                    def docker_timeout(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                        nonlocal removed
                        commands.append(command)
                        if command[0:2] == ["docker", "run"]:
                            raise subprocess.TimeoutExpired(command, 60)
                        if command[0:2] == ["docker", "inspect"] and command[-1] == name:
                            if removed:
                                return CompletedProcess(command, 1, "", f"Error: No such object: {name}")
                            labels = owned.record(name)["identity_labels"]
                            return CompletedProcess(command, 0, f"{container_id} true {json.dumps(labels)}", "")
                        if command[0:2] == ["docker", "stop"]:
                            return CompletedProcess(command, 0, container_id, "")
                        if command[0:2] == ["docker", "rm"]:
                            removed = True
                            return CompletedProcess(command, 0, container_id, "")
                        if command[0:2] == ["docker", "inspect"]:
                            return CompletedProcess(command, 1, "", f"Error: No such object: {command[-1]}")
                        self.fail(f"unexpected fake Docker command: {command[1]}")

                    with patch("measure.run_command", side_effect=docker_timeout), patch(
                        "measure.bounded_rmtree", side_effect=lambda path, *_: shutil.rmtree(path)
                    ):
                        if kind == "foreground":
                            cleanup_evidence: dict[str, object] = {}
                            with self.assertRaises(subprocess.TimeoutExpired):
                                run_foreground(
                                    "retention-application",
                                    "sha256:" + "1" * 64,
                                    time.monotonic(),
                                    version,
                                    1,
                                    cleanup_evidence,
                                    owned=owned,
                                    root=root,
                                    recovery_path=recovery_path,
                                )
                            self.assertEqual(cleanup_evidence["status"], "removed")
                            self.assertEqual(cleanup_evidence["container_id"], container_id)
                            self.assertTrue(sentinel.exists())
                        else:
                            with self.assertRaises(subprocess.TimeoutExpired):
                                start_container(
                                    "test-image",
                                    recovery_path,
                                    name,
                                    time.monotonic(),
                                    owned=owned,
                                    kind=kind,
                                    root=root,
                                    version=version,
                                    trial_index=trial_index,
                                )
                            result, errors = cleanup_path_if_safe(
                                recovery_path,
                                root,
                                owned,
                                time.monotonic(),
                                cleanup_reserve_seconds=5,
                            )
                            self.assertTrue(result["removed"])
                            self.assertFalse(errors)
                            self.assertFalse(recovery_path.exists())

                    self.assertEqual(owned.record(name)["container_id"], container_id)
                    self.assertEqual(owned.record(name)["status"], "removed")
                    self.assertEqual(
                        owned.record(name)["verified_identity_labels"],
                        owned.record(name)["identity_labels"],
                    )
                    self.assertEqual(
                        [command[1] for command in commands],
                        ["run", "inspect", "stop", "rm", "inspect", "inspect"],
                    )

    def test_unconfirmed_foreground_cleanup_refuses_to_remove_trial_data(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "acceptance-root"
            trial_dir = root / "trial-baseline-1"
            trial_dir.mkdir(parents=True)
            sentinel = trial_dir / "database.sqlite"
            sentinel.write_bytes(b"preserve until driver absence is confirmed")
            owned = OwnedContainerLedger()
            cleanup: dict[str, object] = {}
            run_id = "123456abcdef"
            started = time.monotonic()

            def daemon_failure(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                if command[0:2] == ["docker", "run"]:
                    return CompletedProcess(command, 0, "{}", "")
                if command[0:2] == ["docker", "inspect"]:
                    return CompletedProcess(command, 1, "", "Cannot connect to the Docker daemon")
                self.fail(f"unexpected fake Docker command: {command[1]}")

            with patch("measure.RUN_ID", run_id), patch(
                "measure.run_command", side_effect=daemon_failure
            ), patch("measure.bounded_rmtree") as remove:
                with self.assertRaisesRegex(RuntimeError, "could not inspect named container"):
                    run_foreground(
                        "retention-application",
                        "sha256:" + "1" * 64,
                        started,
                        "baseline",
                        1,
                        cleanup,
                        owned=owned,
                        root=root,
                        recovery_path=trial_dir,
                    )
                result, errors = cleanup_path_if_safe(
                    root,
                    root,
                    owned,
                    started,
                    cleanup_reserve_seconds=5,
                )

            self.assertEqual(cleanup["status"], "failed")
            self.assertFalse(result["removed"])
            self.assertTrue(errors)
            self.assertEqual(len(result["recovery"]["unresolved_containers"]), 1)
            self.assertEqual(result["recovery"]["unresolved_containers"][0]["container_name"],
                             f"retention-{run_id}-baseline-1-driver")
            self.assertTrue(sentinel.exists())
            remove.assert_not_called()

    def test_work_budget_reserves_time_for_cleanup(self) -> None:
        with patch("measure.time.monotonic", return_value=1_000.0):
            self.assertEqual(remaining_budget(200.0), 900.0 - 800.0 - CLEANUP_RESERVE_SECONDS)
            self.assertEqual(cleanup_budget(200.0), 100.0)
        with patch("measure.time.monotonic", return_value=200.0 + 900.0 - CLEANUP_RESERVE_SECONDS):
            with self.assertRaisesRegex(TimeoutError, "work cutoff"):
                remaining_budget(200.0)

    def test_run_command_caps_timeout_at_the_work_deadline(self) -> None:
        with patch("measure.time.monotonic", return_value=1_000.0), patch("measure.subprocess.run") as run:
            run.return_value = CompletedProcess(["true"], 0, "", "")
            run_command(["true"], timeout=300, phase_started=200.0)
        self.assertEqual(run.call_args.kwargs["timeout"], 900.0 - 800.0 - CLEANUP_RESERVE_SECONDS)

    def test_missing_container_is_distinguished_from_docker_failure(self) -> None:
        missing = CompletedProcess(["docker"], 1, "", "Error: No such object: retention-test-driver")
        with patch("measure.run_command", return_value=missing):
            result = cleanup_named_container("retention-test-driver", time.monotonic(), 30)
        self.assertEqual(result["status"], "already_absent")

        unavailable = CompletedProcess(["docker"], 1, "", "Cannot connect to the Docker daemon")
        with patch("measure.run_command", return_value=unavailable):
            with self.assertRaisesRegex(RuntimeError, "could not inspect named container"):
                cleanup_named_container("retention-test-driver", time.monotonic(), 30)

    def test_daemon_failure_preserves_trial_and_root_across_outer_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "acceptance-root"
            trial = root / "trial-baseline-1"
            template = root / "template-baseline"
            trial.mkdir(parents=True)
            template.mkdir()
            (trial / "database.sqlite").write_bytes(b"recovery data")
            name = "retention-123456abcdef-baseline-1"
            container_id = "a" * 64
            owned = OwnedContainerLedger()
            owned.register(name, "application", root, trial, trial, version="baseline", trial_index=1)
            owned.mark_started(name, container_id)
            init_name = "retention-123456abcdef-init-baseline"
            init_id = "d" * 64
            owned.register(init_name, "init", root, template, template, version="baseline", trial_index=None)
            owned.mark_started(init_name, init_id)
            entered = threading.Event()
            release = threading.Event()
            result: dict[str, object] = {}

            def daemon_failure(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                entered.set()
                release.wait(timeout=2)
                return CompletedProcess(command, 1, "", "Cannot connect to the Docker daemon")

            def cleanup_trial() -> None:
                result["cleanup"] = cleanup_path_if_safe(
                    trial, root, owned, time.monotonic(), cleanup_reserve_seconds=5
                )

            with patch("measure.run_command", side_effect=daemon_failure), patch("measure.bounded_rmtree") as remove:
                worker = threading.Thread(target=cleanup_trial)
                worker.start()
                self.assertTrue(entered.wait(timeout=2), "fake Docker inspection did not reach its barrier")
                self.assertTrue(root.is_dir())
                self.assertTrue(trial.is_dir())
                release.set()
                worker.join(timeout=2)
                self.assertFalse(worker.is_alive(), "cleanup did not finish after the barrier was released")
                root_result, root_errors = cleanup_path_if_safe(
                    root, root, owned, time.monotonic(), cleanup_reserve_seconds=5
                )

            trial_result, trial_errors = result["cleanup"]
            self.assertFalse(trial_result["removed"])
            self.assertTrue(trial_errors)
            self.assertFalse(root_result["removed"])
            self.assertTrue(root_errors)
            unresolved_ids = {
                record["container_id"]
                for record in root_result["recovery"]["unresolved_containers"]
            }
            self.assertEqual(unresolved_ids, {container_id, init_id})
            self.assertEqual(
                set(root_result["recovery"]["preserved_paths"]),
                {str(root), str(trial), str(template)},
            )
            self.assertTrue(root.is_dir())
            self.assertTrue(trial.is_dir())
            remove.assert_not_called()

    def test_inspect_timeout_preserves_the_recovery_root(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "acceptance-root"
            root.mkdir()
            (root / "database.sqlite").write_bytes(b"recovery data")
            name = "retention-123456abcdef-driver"
            owned = OwnedContainerLedger()
            owned.register(
                name,
                "foreground",
                root,
                root,
                root / "foreground.py",
                version="candidate",
                trial_index=1,
            )
            owned.mark_started(name, "b" * 64)
            timeout = subprocess.TimeoutExpired(["docker", "inspect", name], 3)
            with patch("measure.run_command", side_effect=timeout), patch("measure.bounded_rmtree") as remove:
                cleanup, errors = cleanup_path_if_safe(
                    root, root, owned, time.monotonic(), cleanup_reserve_seconds=5
                )

            self.assertFalse(cleanup["removed"])
            self.assertTrue(errors)
            self.assertEqual(cleanup["recovery"]["unresolved_containers"][0]["container_id"], "b" * 64)
            self.assertTrue(root.is_dir())
            remove.assert_not_called()

    def test_confirmed_container_removal_allows_directory_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "acceptance-root"
            trial = root / "trial-baseline-1"
            trial.mkdir(parents=True)
            (trial / "database.sqlite").write_bytes(b"temporary data")
            name = "retention-123456abcdef-baseline-1"
            container_id = "c" * 64
            owned = OwnedContainerLedger()
            owned.register(name, "application", root, trial, trial, version="baseline", trial_index=1)
            owned.mark_started(name, container_id)
            commands: list[list[str]] = []
            removed = False

            def docker_success(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                nonlocal removed
                commands.append(command)
                if command[:3] == ["docker", "inspect", "--format"]:
                    if command[-1] == name:
                        if removed:
                            return CompletedProcess(command, 1, "", f"Error: No such object: {name}")
                        labels = owned.record(name)["identity_labels"]
                        return CompletedProcess(command, 0, f"{container_id} true {json.dumps(labels)}", "")
                    return CompletedProcess(command, 1, "", f"Error: No such object: {command[-1]}")
                if command[:2] == ["docker", "rm"]:
                    removed = True
                return CompletedProcess(command, 0, container_id, "")

            def remove_directory(path: Path, *_: object) -> None:
                shutil.rmtree(path)

            with patch("measure.run_command", side_effect=docker_success), patch(
                "measure.bounded_rmtree", side_effect=remove_directory
            ):
                cleanup, errors = cleanup_path_if_safe(
                    trial, root, owned, time.monotonic(), cleanup_reserve_seconds=5
                )

            self.assertTrue(cleanup["removed"])
            self.assertFalse(errors)
            self.assertFalse(trial.exists())
            self.assertEqual(owned.record(name)["status"], "removed")
            self.assertEqual([command[1] for command in commands], ["inspect", "stop", "rm", "inspect", "inspect"])

    def test_renamed_owned_container_is_removed_by_its_confirmed_id(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "acceptance-root"
            trial = root / "trial-baseline-1"
            trial.mkdir(parents=True)
            (trial / "database.sqlite").write_bytes(b"keep mounted until container removal")
            name = "retention-123456abcdef-baseline-1"
            container_id = "a" * 64
            owned = OwnedContainerLedger()
            owned.register(name, "application", root, trial, trial, version="baseline", trial_index=1)
            owned.mark_started(name, container_id)
            commands: list[list[str]] = []
            removed = False

            def docker_renamed(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                nonlocal removed
                commands.append(command)
                if command[:2] == ["docker", "inspect"]:
                    target = command[-1]
                    if target == name:
                        return CompletedProcess(command, 1, "", f"Error: No such object: {target}")
                    if target == container_id and not removed:
                        labels = owned.record(name)["identity_labels"]
                        return CompletedProcess(command, 0, f"{container_id} true {json.dumps(labels)}", "")
                    if target == container_id:
                        return CompletedProcess(command, 1, "", f"Error: No such object: {target}")
                if command[:2] == ["docker", "stop"]:
                    self.assertEqual(command[-1], container_id)
                    return CompletedProcess(command, 0, container_id, "")
                if command[:2] == ["docker", "rm"]:
                    self.assertEqual(command[-1], container_id)
                    removed = True
                    return CompletedProcess(command, 0, container_id, "")
                self.fail(f"unexpected fake Docker command: {command[1]}")

            with patch("measure.run_command", side_effect=docker_renamed), patch(
                "measure.bounded_rmtree", side_effect=lambda path, *_: shutil.rmtree(path)
            ):
                result, errors = cleanup_path_if_safe(
                    trial,
                    root,
                    owned,
                    time.monotonic(),
                    cleanup_reserve_seconds=5,
                )

            self.assertTrue(result["removed"])
            self.assertFalse(errors)
            self.assertTrue(removed)
            self.assertFalse(trial.exists())
            self.assertEqual(owned.record(name)["container_id"], container_id)
            self.assertEqual(owned.record(name)["status"], "removed")
            self.assertEqual(
                [command[-1] for command in commands if command[:2] == ["docker", "inspect"]],
                [name, container_id, container_id, name],
            )

    def test_missing_name_is_confirmed_absent_by_the_known_container_id(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "acceptance-root"
            trial = root / "trial-baseline-1"
            trial.mkdir(parents=True)
            (trial / "database.sqlite").write_bytes(b"remove only after ID absence is confirmed")
            name = "retention-123456abcdef-baseline-1"
            container_id = "b" * 64
            owned = OwnedContainerLedger()
            owned.register(name, "application", root, trial, trial, version="baseline", trial_index=1)
            owned.mark_started(name, container_id)
            commands: list[list[str]] = []

            def docker_absent(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                commands.append(command)
                target = command[-1]
                return CompletedProcess(command, 1, "", f"Error: No such object: {target}")

            with patch("measure.run_command", side_effect=docker_absent), patch(
                "measure.bounded_rmtree", side_effect=lambda path, *_: shutil.rmtree(path)
            ):
                result, errors = cleanup_path_if_safe(
                    trial,
                    root,
                    owned,
                    time.monotonic(),
                    cleanup_reserve_seconds=5,
                )

            self.assertTrue(result["removed"])
            self.assertFalse(errors)
            self.assertFalse(trial.exists())
            self.assertEqual([command[-1] for command in commands], [name, container_id, name])
            self.assertEqual(owned.record(name)["status"], "already_absent")
            self.assertEqual(owned.record(name)["verified_identity_labels"], owned.record(name)["identity_labels"])

    def test_replacement_same_name_preserves_data_and_never_removes_foreign_id(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "acceptance-root"
            trial = root / "trial-baseline-1"
            trial.mkdir(parents=True)
            sentinel = trial / "database.sqlite"
            sentinel.write_bytes(b"preserve while replacement ownership is unknown")
            name = "retention-123456abcdef-baseline-1"
            owned_id = "c" * 64
            foreign_id = "d" * 64
            owned = OwnedContainerLedger()
            owned.register(name, "application", root, trial, trial, version="baseline", trial_index=1)
            owned.mark_started(name, owned_id)
            commands: list[list[str]] = []
            name_inspections = 0

            def docker_replacement(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                nonlocal name_inspections
                commands.append(command)
                if command[:2] == ["docker", "inspect"]:
                    target = command[-1]
                    if target == name:
                        name_inspections += 1
                        if name_inspections == 1:
                            return CompletedProcess(command, 1, "", f"Error: No such object: {name}")
                        foreign_labels = {CONTAINER_OWNER_LABEL: "e" * 32}
                        return CompletedProcess(command, 0, f"{foreign_id} true {json.dumps(foreign_labels)}", "")
                    if target == owned_id:
                        return CompletedProcess(command, 1, "", f"Error: No such object: {owned_id}")
                    foreign_labels = {CONTAINER_OWNER_LABEL: "e" * 32}
                    return CompletedProcess(command, 0, f"{foreign_id} true {json.dumps(foreign_labels)}", "")
                self.fail(f"replacement container must not be stopped or removed: {command}")

            with patch("measure.run_command", side_effect=docker_replacement), patch(
                "measure.bounded_rmtree"
            ) as remove:
                result, errors = cleanup_path_if_safe(
                    trial,
                    root,
                    owned,
                    time.monotonic(),
                    cleanup_reserve_seconds=5,
                )

            self.assertFalse(result["removed"])
            self.assertTrue(errors)
            self.assertTrue(sentinel.exists())
            self.assertEqual([command[1] for command in commands], ["inspect", "inspect", "inspect"])
            self.assertEqual([command[-1] for command in commands], [name, owned_id, name])
            self.assertEqual(owned.record(name)["status"], "unconfirmed")
            remove.assert_not_called()

    def test_name_replaced_after_owned_removal_preserves_mounted_data(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "acceptance-root"
            trial = root / "trial-baseline-1"
            trial.mkdir(parents=True)
            sentinel = trial / "database.sqlite"
            sentinel.write_bytes(b"preserve if a replacement may mount this directory")
            name = "retention-123456abcdef-baseline-1"
            owned_id = "f" * 64
            foreign_id = "e" * 64
            owned = OwnedContainerLedger()
            owned.register(name, "application", root, trial, trial, version="baseline", trial_index=1)
            owned.mark_started(name, owned_id)
            commands: list[list[str]] = []
            removed = False

            def docker_replaced_after_remove(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                nonlocal removed
                commands.append(command)
                if command[:2] == ["docker", "inspect"]:
                    target = command[-1]
                    if target == name and not removed:
                        labels = owned.record(name)["identity_labels"]
                        return CompletedProcess(command, 0, f"{owned_id} true {json.dumps(labels)}", "")
                    if target == name:
                        foreign_labels = {CONTAINER_OWNER_LABEL: "e" * 32}
                        return CompletedProcess(command, 0, f"{foreign_id} true {json.dumps(foreign_labels)}", "")
                    return CompletedProcess(command, 1, "", f"Error: No such object: {target}")
                if command[:2] == ["docker", "stop"]:
                    self.assertEqual(command[-1], owned_id)
                    return CompletedProcess(command, 0, owned_id, "")
                if command[:2] == ["docker", "rm"]:
                    self.assertEqual(command[-1], owned_id)
                    removed = True
                    return CompletedProcess(command, 0, owned_id, "")
                self.fail(f"replacement container must not be stopped or removed: {command}")

            with patch("measure.run_command", side_effect=docker_replaced_after_remove), patch(
                "measure.bounded_rmtree"
            ) as remove_directory:
                result, errors = cleanup_path_if_safe(
                    trial,
                    root,
                    owned,
                    time.monotonic(),
                    cleanup_reserve_seconds=5,
                )

            self.assertTrue(removed)
            self.assertFalse(result["removed"])
            self.assertTrue(errors)
            self.assertTrue(sentinel.exists())
            self.assertEqual(
                [command[1] for command in commands],
                ["inspect", "stop", "rm", "inspect", "inspect"],
            )
            self.assertEqual(owned.record(name)["status"], "unconfirmed")
            remove_directory.assert_not_called()

    def test_creation_timeout_finds_renamed_container_by_owner_labels(self) -> None:
        run_id = "123456abcdef"
        container_id = "e" * 64
        with tempfile.TemporaryDirectory() as temporary, patch("measure.RUN_ID", run_id):
            root = Path(temporary) / "acceptance-root"
            trial = root / "trial-baseline-1"
            trial.mkdir(parents=True)
            (trial / "database.sqlite").write_bytes(b"recover until owner labels are confirmed")
            name = f"retention-{run_id}-baseline-1"
            owned = OwnedContainerLedger()
            commands: list[list[str]] = []
            removed = False

            def docker_timeout_renamed(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                nonlocal removed
                commands.append(command)
                if command[:2] == ["docker", "run"]:
                    raise subprocess.TimeoutExpired(command, 60)
                if command[:2] == ["docker", "inspect"] and command[-1] == name:
                    return CompletedProcess(command, 1, "", f"Error: No such object: {name}")
                if command[:2] == ["docker", "ps"]:
                    self.assertEqual(command[1:5], ["ps", "--all", "--quiet", "--no-trunc"])
                    expected = owned.record(name)["identity_labels"]
                    filters: dict[str, str] = {}
                    for index, argument in enumerate(command[:-1]):
                        if argument == "--filter":
                            key, value = command[index + 1].removeprefix("label=").split("=", 1)
                            filters[key] = value
                    self.assertEqual(filters, expected)
                    return CompletedProcess(command, 0, container_id + "\n", "")
                if command[:2] == ["docker", "inspect"] and command[-1] == container_id and not removed:
                    labels = owned.record(name)["identity_labels"]
                    return CompletedProcess(command, 0, f"{container_id} true {json.dumps(labels)}", "")
                if command[:2] == ["docker", "inspect"] and command[-1] == container_id:
                    return CompletedProcess(command, 1, "", f"Error: No such object: {container_id}")
                if command[:2] == ["docker", "stop"]:
                    self.assertEqual(command[-1], container_id)
                    return CompletedProcess(command, 0, container_id, "")
                if command[:2] == ["docker", "rm"]:
                    self.assertEqual(command[-1], container_id)
                    removed = True
                    return CompletedProcess(command, 0, container_id, "")
                self.fail(f"unexpected fake Docker command: {command[1]}")

            with patch("measure.run_command", side_effect=docker_timeout_renamed), patch(
                "measure.bounded_rmtree", side_effect=lambda path, *_: shutil.rmtree(path)
            ):
                with self.assertRaises(subprocess.TimeoutExpired):
                    start_container(
                        "test-image",
                        trial,
                        name,
                        time.monotonic(),
                        owned=owned,
                        kind="application",
                        root=root,
                        version="baseline",
                        trial_index=1,
                    )
                result, errors = cleanup_path_if_safe(
                    trial,
                    root,
                    owned,
                    time.monotonic(),
                    cleanup_reserve_seconds=5,
                )

            self.assertTrue(result["removed"])
            self.assertFalse(errors)
            self.assertTrue(removed)
            self.assertFalse(trial.exists())
            self.assertEqual(owned.record(name)["container_id"], container_id)
            self.assertEqual(owned.record(name)["status"], "removed")
            self.assertEqual(
                [command[1] for command in commands],
                ["run", "inspect", "ps", "inspect", "stop", "rm", "inspect", "inspect"],
            )

    def test_unconfirmed_creation_timeout_without_owner_match_preserves_recovery_data(self) -> None:
        run_id = "123456abcdef"
        with tempfile.TemporaryDirectory() as temporary, patch("measure.RUN_ID", run_id):
            root = Path(temporary) / "acceptance-root"
            trial = root / "trial-baseline-1"
            trial.mkdir(parents=True)
            sentinel = trial / "database.sqlite"
            sentinel.write_bytes(b"preserve while timed-out creation is unconfirmed")
            name = f"retention-{run_id}-baseline-1"
            owned = OwnedContainerLedger()
            commands: list[list[str]] = []

            def docker_timeout_without_match(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                commands.append(command)
                if command[:2] == ["docker", "run"]:
                    raise subprocess.TimeoutExpired(command, 60)
                if command[:2] == ["docker", "inspect"] and command[-1] == name:
                    return CompletedProcess(command, 1, "", f"Error: No such object: {name}")
                if command[:2] == ["docker", "ps"]:
                    return CompletedProcess(command, 0, "", "")
                self.fail(f"unexpected fake Docker command: {command[1]}")

            with patch("measure.run_command", side_effect=docker_timeout_without_match), patch(
                "measure.bounded_rmtree"
            ) as remove:
                with self.assertRaises(subprocess.TimeoutExpired):
                    start_container(
                        "test-image",
                        trial,
                        name,
                        time.monotonic(),
                        owned=owned,
                        kind="application",
                        root=root,
                        version="baseline",
                        trial_index=1,
                    )
                result, errors = cleanup_path_if_safe(
                    trial,
                    root,
                    owned,
                    time.monotonic(),
                    cleanup_reserve_seconds=5,
                )

            self.assertFalse(result["removed"])
            self.assertEqual(result["status"], "preserved")
            self.assertTrue(errors)
            self.assertTrue(sentinel.exists())
            self.assertEqual(
                [record["status"] for record in result["recovery"]["unresolved_containers"]],
                ["unconfirmed"],
            )
            self.assertEqual([command[1] for command in commands], ["run", "inspect", "ps", "inspect"])
            remove.assert_not_called()


if __name__ == "__main__":
    unittest.main()
