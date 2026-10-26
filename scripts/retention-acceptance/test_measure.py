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
from unittest.mock import patch

from contract import (
    AcceptanceError,
    CLEANUP_RESERVE_SECONDS,
    COMPETITOR_TASK,
    CONTAINER_KIND_LABEL,
    CONTAINER_OWNER_LABEL,
    CONTAINER_RUN_ID_LABEL,
    CONTAINER_TRIAL_LABEL,
    DATASETS,
    SUMMARY_RECOVERY_TASK,
    TRIAL_EXECUTION_ORDER,
)
from fixture import (
    CONTROLLED_TASK_KEYS,
    _history_timestamp,
    prepare_competitor,
    seed_parallel_work_coverage,
)
from measure import (
    OwnedContainerLedger,
    cleanup_budget,
    cleanup_named_container,
    cleanup_path_if_safe,
    add_observed_input_identity_digests,
    observed_attempt_states,
    remaining_budget,
    run_command,
    run_foreground,
    start_container,
    task_state_snapshot,
    validate_task_snapshot,
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
    def test_negative_control_oracle_skips_verified_input_digest_binding(self) -> None:
        oracle = {
            "verified": False,
            "failure_class": "source_artifact_conservation",
        }
        add_observed_input_identity_digests(
            oracle,
            {"invocations": ["invoke-1"], "pool_upstream_attempts": ["attempt-1"]},
        )
        self.assertNotIn("datasets", oracle)

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
                "committed_bytes": 512 * 256 if task_key == COMPETITOR_TASK else 0,
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

    def test_competitor_partial_chunks_count_toward_fixed_workload(self) -> None:
        rows = [
            {
                "task_key": COMPETITOR_TASK,
                "trigger_kind": "schedule",
                "status": "skipped",
                "completion": "partial",
                "actual_started_at": f"2026-10-25T00:00:{index:02d}Z",
                "actual_finished_at": f"2026-10-25T00:00:{index + 1:02d}Z",
                "committed_work": 32,
                "details": {"filesChecked": 32, "filesReleased": 32, "failures": 0},
                "committed_bytes": 32 * 256,
                "error_detail": None,
            }
            for index in range(16)
        ] + [
            {
                "task_key": task_key,
                "trigger_kind": trigger_kind,
                "status": "success",
                "completion": "complete",
                "actual_started_at": f"2026-10-25T00:01:{index:02d}Z",
                "actual_finished_at": f"2026-10-25T00:01:{index + 1:02d}Z",
                "committed_work": 1000,
                "committed_bytes": 0,
                "details": {},
                "error_detail": None,
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

        validate_task_snapshot(rows)

    def test_recoverable_competitor_failure_counts_after_file_oracle_completion(self) -> None:
        rows = [
            {
                "task_key": COMPETITOR_TASK,
                "trigger_kind": "schedule",
                "status": "skipped",
                "completion": "partial",
                "actual_started_at": "2026-10-25T00:00:00Z",
                "actual_finished_at": "2026-10-25T00:00:01Z",
                "committed_work": 512,
                "committed_bytes": 512 * 256,
                "details": {"filesChecked": 512, "filesReleased": 512, "failures": 1, "hasMore": True},
                "error_detail": None,
            },
            *[
                {
                    "task_key": task_key,
                    "trigger_kind": trigger_kind,
                    "status": "success",
                    "completion": "complete",
                    "actual_started_at": f"2026-10-25T00:01:{index:02d}Z",
                    "actual_finished_at": f"2026-10-25T00:01:{index + 1:02d}Z",
                    "committed_work": 1000,
                    "details": {},
                    "error_detail": None,
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
            ],
        ]

        with self.assertRaisesRegex(AcceptanceError, "fixed workload"):
            validate_task_snapshot(rows)
        validate_task_snapshot(rows, allow_recoverable_competitor_failures=True)

    def test_fixture_history_timestamps_match_retention_local_time_contract(self) -> None:
        timestamp = _history_timestamp("2026-10-24 00:00:00", 0)

        self.assertEqual(timestamp, "2026-09-01 00:00:00")
        self.assertNotIn("T", timestamp)
        self.assertNotIn("Z", timestamp)

    def test_task_state_snapshot_missing_database_is_non_fatal(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            snapshot = task_state_snapshot(Path(temporary) / "missing.sqlite")

        self.assertIn("error", snapshot)
        self.assertIn("unable to open database file", snapshot["error"])

    def test_task_state_snapshot_preserves_terminal_details(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            database = Path(temporary) / "maintenance.sqlite"
            connection = sqlite3.connect(database)
            try:
                connection.execute(
                    "CREATE TABLE managed_tasks (task_key TEXT, enabled INTEGER, is_manual INTEGER, "
                    "interval_secs INTEGER, next_trigger_at TEXT, next_catchup_at TEXT, catchup_reason TEXT)"
                )
                connection.execute(
                    "CREATE TABLE managed_task_runs (id INTEGER, task_key TEXT, trigger_kind TEXT, status TEXT, "
                    "started_at TEXT, actual_started_at TEXT, finished_at TEXT, actual_finished_at TEXT, "
                    "summary TEXT, error_detail TEXT, processed_count INTEGER, updated_count INTEGER, "
                    "details TEXT, completion TEXT)"
                )
                connection.execute(
                    "CREATE TABLE maintenance_owner_retry_state (task_key TEXT, failure_count INTEGER, "
                    "next_eligible_at TEXT, reason TEXT)"
                )
                connection.execute(
                    "INSERT INTO managed_task_runs VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                    (
                        1,
                        "retention_archive",
                        "schedule",
                        "failed",
                        "2026-10-25T00:00:00.000Z",
                        "2026-10-25T00:00:00.100Z",
                        "2026-10-25T00:00:01.000Z",
                        "2026-10-25T00:00:01.100Z",
                        "retention_archive 执行失败",
                        "fatal_error;failure_fingerprint:abc",
                        0,
                        0,
                        '{"fatalError":"archive failed","waitReason":"owner_failed"}',
                        "failed",
                    ),
                )
                connection.commit()
            finally:
                connection.close()

            snapshot = task_state_snapshot(database)

        self.assertEqual(snapshot["managed_task_runs"][0]["details"]["fatalError"], "archive failed")
        self.assertEqual(snapshot["managed_task_runs"][0]["error_detail"], "fatal_error;failure_fingerprint:abc")

    def test_task_evidence_drain_uses_remaining_phase_budget(self) -> None:
        valid_runs = [
            {
                "task_key": "retention_archive",
                "trigger_kind": trigger_kind,
                "status": "success",
                "actual_started_at": (
                    "2026-10-25T00:00:02Z" if trigger_kind == "catchup" else "2026-10-25T00:00:00Z"
                ),
                "actual_finished_at": "2026-10-25T00:00:01Z",
            }
            for trigger_kind in ("schedule", "catchup")
        ] + [
            {
                "task_key": "pool_upstream_attempt_archive",
                "trigger_kind": trigger_kind,
                "status": "success",
                "actual_started_at": (
                    "2026-10-25T00:00:02Z" if trigger_kind == "catchup" else "2026-10-25T00:00:00Z"
                ),
                "actual_finished_at": "2026-10-25T00:00:01Z",
            }
            for trigger_kind in ("schedule", "catchup")
        ] + [
            {
                "task_key": "raw_orphan_sweep",
                "trigger_kind": "interval",
                "status": "success",
                "actual_started_at": "2026-10-25T00:00:00Z",
                "actual_finished_at": "2026-10-25T00:00:01Z",
                "committed_work": 512,
            }
        ]
        with (
            patch("measure.remaining_budget", return_value=120),
            patch("measure.time.monotonic", side_effect=[0, 0, 31, 31]),
            patch("measure.time.sleep"),
            patch("measure.task_runs", side_effect=[[], valid_runs, valid_runs]),
            patch("measure.quiesce_controlled_tasks") as quiesce_controlled,
            patch("measure.controlled_task_runs_idle", return_value=True) as task_runs_idle,
            patch(
                "measure.archive_proof_status",
                return_value={
                    "ready": True,
                    "reference_ready": True,
                    "datasets": {
                        "invocations": {
                            "dataset": "codex_invocations",
                            "ready": True,
                            "reference_ready": True,
                            "summary_proof_complete": True,
                            "summary_proof_missing": [],
                        },
                        "pool_upstream_attempts": {
                            "dataset": "pool_upstream_request_attempts",
                            "ready": True,
                            "reference_ready": True,
                            "summary_proof_complete": True,
                            "summary_proof_missing": [],
                        },
                    },
                },
            ),
        ):
            result = wait_for_task_evidence(
                Path("main.sqlite"),
                Path("maintenance.sqlite"),
                "2026-10-24 00:00:00",
                {"invocations": 81, "pool_upstream_attempts": 97},
                0,
            )

        self.assertEqual(result, valid_runs)
        quiesce_controlled.assert_called_once_with(Path("maintenance.sqlite"), 0)
        task_runs_idle.assert_called_once_with(Path("maintenance.sqlite"))

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
                {DATASETS[name]["task_key"] for name in DATASETS}
                | {COMPETITOR_TASK, SUMMARY_RECOVERY_TASK},
            )
            self.assertEqual(set(CONTROLLED_TASK_KEYS), {
                key for key, (enabled, next_trigger, next_catchup, _) in rows.items()
                if enabled == 1 and next_trigger is not None and next_catchup is None
            })
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
