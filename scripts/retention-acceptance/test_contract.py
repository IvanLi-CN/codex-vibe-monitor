from __future__ import annotations

from datetime import datetime, timedelta, timezone
import hashlib
import math
import unittest

from contract import (
    BASELINE_SHA,
    COMPETITOR_FILE_BYTES,
    COMPETITOR_FILE_COUNT,
    CONTAINER_KIND_LABEL,
    CONTAINER_OWNER_LABEL,
    CONTAINER_RUN_ID_LABEL,
    CONTAINER_TRIAL_LABEL,
    CONTAINER_VERSION_LABEL,
    DATASETS,
    DRIVER_IMAGE_DIGEST,
    DRIVER_IMAGE_REF,
    SCHEMA_VERSION,
    TRIAL_EXECUTION_ORDER,
    AcceptanceError,
    sha256_json,
    task_run_is_accepted_competitor_chunk,
    task_run_is_accepted_zero_contribution_competitor_defer,
    task_run_is_bounded_baseline_observation,
    task_run_is_recoverable_competitor_chunk,
    task_run_is_accepted_terminal,
    validate_evidence,
)


RUN_ID = "123456abcdef"


def timestamp(value: datetime) -> str:
    return value.isoformat(timespec="milliseconds").replace("+00:00", "Z")


def histogram_evidence() -> tuple[list[dict], dict[str, int]]:
    before = {
        operation: {"0.1": 0, "0.25": 0, "0.5": 0, "+Inf": 0}
        for operation in ("invocation_archive", "timestamped_archive")
    }
    after = {
        operation: {"0.1": 10, "0.25": 20, "0.5": 20, "+Inf": 20}
        for operation in ("invocation_archive", "timestamped_archive")
    }
    delta = {"0.1": 20, "0.25": 40, "0.5": 40, "+Inf": 40}
    return [{"scope": "pre-input", "bucket_counts": before}, {"scope": "after-drain", "bucket_counts": after}], delta


def oracle_result(name: str, additions: int) -> dict:
    definition = DATASETS[name]
    converted = additions * 12
    identities = [f"retention-input-{name}-{index:06d}" for index in range(1, additions + 1)]
    return {
        "dataset": definition["manifest_dataset"],
        "seed_rows": definition["seed_rows"],
        "actual_added_rows": additions,
        "converted_rows": converted,
        "distinct_archived_rows": converted,
        "live_rows": definition["seed_rows"] + additions - converted,
        "lost_rows": 0,
        "duplicate_rows": 0,
        "added_ids_sha256": hashlib.sha256("\n".join(identities).encode()).hexdigest(),
        "row_content_columns": 8,
        "row_content_rows_checked": definition["seed_rows"] + additions,
        "row_content_sha256": "c" * 64,
        "legacy_missing_source_columns": [],
        "sustained_window_input_seconds": 120.0,
        "sustained_window_seconds": 123.0,
        "sustained_convergence_factor": (converted / 123.0) / (additions / 120.0),
    }


def container_identity_labels(kind: str, version: str, trial_index: int) -> dict[str, str]:
    token = hashlib.sha256(f"{kind}:{version}:{trial_index}".encode()).hexdigest()[:32]
    return {
        CONTAINER_RUN_ID_LABEL: RUN_ID,
        CONTAINER_KIND_LABEL: kind,
        CONTAINER_VERSION_LABEL: version,
        CONTAINER_TRIAL_LABEL: str(trial_index),
        CONTAINER_OWNER_LABEL: token,
    }


def task_runs(start: datetime) -> list[dict]:
    times = {
        "retention_archive": (start + timedelta(seconds=3), start + timedelta(seconds=8)),
        "pool_upstream_attempt_archive": (start + timedelta(seconds=4), start + timedelta(seconds=9)),
        "raw_orphan_sweep": (start + timedelta(seconds=5), start + timedelta(seconds=10)),
    }
    rows = []
    for key, (first_start, first_end) in times.items():
        rows.append({
            "task_key": key,
            "trigger_kind": "schedule",
            "actual_started_at": timestamp(first_start),
            "actual_finished_at": timestamp(first_end),
            "status": "success",
            "committed_work": COMPETITOR_FILE_COUNT if key == "raw_orphan_sweep" else 100,
            "committed_bytes": COMPETITOR_FILE_COUNT * COMPETITOR_FILE_BYTES if key == "raw_orphan_sweep" else 0,
        })
        if key != "raw_orphan_sweep":
            rows.append({
                "task_key": key,
                "trigger_kind": "catchup",
                "actual_started_at": timestamp(first_end + timedelta(seconds=1)),
                "actual_finished_at": timestamp(first_end + timedelta(seconds=2)),
                "status": "success",
                "committed_work": 100,
            })
    return rows


def sample_rows(start: datetime, finish: datetime, count: int, elapsed: float, path: str) -> list[dict]:
    span = max(1, int((finish - start).total_seconds()))
    return [
        {
            "observed_at": timestamp(start + timedelta(seconds=(index % span))),
            "elapsed_ms": elapsed,
            "transport": "http",
            "path": path,
        }
        for index in range(count)
    ]


def write_sample_row(start: datetime, sequence: int, invoke_id: str, request_ms: float = 5.0, lag_ms: float = 0.0) -> dict:
    scheduled = sequence / DATASETS["invocations"]["arrival_rate"] * 1000
    dispatched = scheduled + lag_ms
    completed = dispatched + request_ms
    return {
        "sequence": sequence,
        "invoke_id": invoke_id,
        "observed_at": timestamp(start + timedelta(milliseconds=completed)),
        "elapsed_ms": completed - scheduled,
        "scheduled_offset_ms": scheduled,
        "dispatched_offset_ms": dispatched,
        "completed_offset_ms": completed,
        "dispatch_lag_ms": lag_ms,
        "request_elapsed_ms": request_ms,
        "transport": "http",
        "path": "/v1/responses",
    }


def trial_evidence(version: str, index: int, start: datetime, previous_end: datetime | None) -> tuple[dict, datetime]:
    input_start = start + timedelta(seconds=1)
    input_end = input_start + timedelta(seconds=120)
    cleanup = input_end + timedelta(seconds=2)
    write_scrapes, write_buckets = histogram_evidence()
    write_scrapes[0]["observed_at"] = timestamp(start + timedelta(milliseconds=500))
    write_scrapes[1]["observed_at"] = timestamp(cleanup - timedelta(milliseconds=500))
    identities = {
        name: [f"retention-input-{name}-{sequence:06d}" for sequence in range(1, math.ceil(definition["arrival_rate"] * 120) + 1)]
        for name, definition in DATASETS.items()
    }
    input_counts = {name: len(values) for name, values in identities.items()}
    reads = sample_rows(input_start, input_end, 24, 10.0, "/api/pool/upstream-accounts")
    writes = [write_sample_row(input_start, sequence, invoke_id) for sequence, invoke_id in enumerate(identities["invocations"])]
    retry_count = input_counts["pool_upstream_attempts"] - input_counts["invocations"]
    retry_sequences = {
        math.floor(retry * input_counts["invocations"] / retry_count)
        for retry in range(retry_count)
    } if retry_count else set()
    attempt_states = []
    attempt_cursor = 0
    for sequence, invoke_id in enumerate(identities["invocations"]):
        statuses = ["http_failure", "success"] if sequence in retry_sequences else ["success"]
        for attempt_index, status in enumerate(statuses, start=1):
            attempt_states.append({
                "attempt_public_id": identities["pool_upstream_attempts"][attempt_cursor],
                "invoke_id": invoke_id,
                "attempt_index": attempt_index,
                "status": status,
                "finished_at": timestamp(input_start + timedelta(seconds=sequence + 1)),
            })
            attempt_cursor += 1
    attempt_map_digest = sha256_json(sorted(
        [row["attempt_public_id"], row["invoke_id"]]
        for row in attempt_states
    ))
    oracle_datasets = {name: oracle_result(name, input_counts[name]) for name in DATASETS}
    oracle_datasets["pool_upstream_attempts"]["attempt_invocation_map_sha256"] = attempt_map_digest
    for result in oracle_datasets.values():
        result["summary_proof_required"] = version == "candidate"
        result["summary_proof_verified"] = version == "candidate"
        result["legacy_source_artifact_duplicates_allowed"] = version == "baseline"
    if version == "baseline":
        oracle_datasets["pool_upstream_attempts"]["legacy_missing_source_columns"] = ["occurred_epoch_ms"]
    archive_proof_status = {
        "ready": version == "candidate",
        "reference_ready": True,
        "datasets": {
            name: {
                "dataset": DATASETS[name]["manifest_dataset"],
                "ready": version == "candidate",
                "reference_ready": True,
                "summary_proof_complete": version == "candidate",
                "summary_proof_missing": [] if version == "candidate" else ["summary proof:baseline-reference"],
            }
            for name in DATASETS
        },
    }
    return {
        "trial_index": index,
        "source_sha": BASELINE_SHA if version == "baseline" else "a" * 40,
        "image_id": f"sha256:{('1' if version == 'baseline' else '2') * 64}",
        "input_fingerprint": "b" * 64,
        "cutoff": "2026-10-23 00:00:00",
        "window": {
            "setup_started_at": timestamp(start),
            "input_started_at": timestamp(input_start),
            "input_finished_at": timestamp(input_end),
            "cleanup_finished_at": timestamp(cleanup),
            "input_duration_seconds": 120.0,
            "full_duration_seconds": 123.0,
            "gap_before_seconds": 0.0 if previous_end is None else (start - previous_end).total_seconds(),
        },
        "input_counts": input_counts,
        "added_ids": identities,
        "attempt_states": attempt_states,
        "container_id": "4" * 64,
        "application_container_cleanup": {
            "container_name": f"retention-{RUN_ID}-{version}-{index}",
            "container_id": "4" * 64,
            "status": "removed",
            "identity_labels": container_identity_labels("application", version, index),
        },
        "foreground_driver_cleanup": {
            "container_name": f"retention-{RUN_ID}-{version}-{index}-driver",
            "container_id": ("5" if version == "baseline" else "6") * 64,
            "status": "already_absent",
            "identity_labels": container_identity_labels("foreground", version, index),
        },
        "latency_samples_ms": {"read": reads, "write": writes},
        "foreground_workload": {
            "invocation_requests": input_counts["invocations"],
            "pool_read_requests": len(reads),
            "mock_upstream": {
                "request_count": input_counts["pool_upstream_attempts"],
                "retryable_failures": retry_count,
                "successful_responses": input_counts["invocations"],
            },
        },
        "task_runs": task_runs(start),
        "competitor_file_oracle": {
            "expected_count": COMPETITOR_FILE_COUNT,
            "expected_bytes": COMPETITOR_FILE_COUNT * COMPETITOR_FILE_BYTES,
            "remaining_count": 0,
            "remaining_bytes": 0,
            "verified_empty": True,
        },
        "archive_proof_status": archive_proof_status,
        "write_hold_samples": write_scrapes,
        "write_hold_buckets": write_buckets,
        "oracle": {
            "verified": True,
            "datasets": oracle_datasets,
        },
    }, cleanup


def valid_evidence() -> dict:
    phase_start = datetime(2026, 10, 24, tzinfo=timezone.utc)
    phase_finish = phase_start + timedelta(seconds=800)
    evidence = {
        "schema_version": SCHEMA_VERSION,
        "run_id": RUN_ID,
        "status": "completed",
        "test_phase": {
            "started_at": timestamp(phase_start),
            "finished_at": timestamp(phase_finish),
            "elapsed_seconds": 800.0,
        },
        "comparison_window": {
            "started_at": timestamp(phase_start),
            "finished_at": timestamp(phase_finish),
            "elapsed_seconds": 800.0,
        },
        "contract": {
            "baseline_sha": BASELINE_SHA,
            "candidate_sha": "a" * 40,
            "driver_image_id": "sha256:" + "3" * 64,
            "driver_image_ref": DRIVER_IMAGE_REF,
            "driver_image_digest": DRIVER_IMAGE_DIGEST,
            "seed": 20261024,
            "retention_days": 1,
            "cutoff": "2026-10-23 00:00:00",
            "seed_rows": {name: definition["seed_rows"] for name, definition in DATASETS.items()},
            "input_fingerprint": "b" * 64,
            "input_rates_rows_per_second": {name: definition["arrival_rate"] for name, definition in DATASETS.items()},
        },
        "trial_execution_order": list(TRIAL_EXECUTION_ORDER),
        "versions": {},
    }
    previous_end = None
    cursor = phase_start
    for version in ("baseline", "candidate"):
        evidence["versions"][version] = {
            "image": {
                "source_sha": BASELINE_SHA if version == "baseline" else "a" * 40,
                "image_id": f"sha256:{('1' if version == 'baseline' else '2') * 64}",
            },
            "host": {"runner_image": "ubuntu-24.04", "cpu_model": "test host"},
            "resource_limits": {"cpus": 2, "memory_bytes": 4_294_967_296},
            "trial_span": {},
            "trials": [],
        }
    for step in TRIAL_EXECUTION_ORDER:
        version = step["version"]
        index = step["trial_index"]
        group = evidence["versions"][version]
        if previous_end is not None:
            cursor = previous_end + timedelta(seconds=5)
        if not group["trial_span"]:
            group["trial_span"]["started_at"] = timestamp(cursor)
        trial, previous_end = trial_evidence(version, index, cursor, previous_end)
        group["trials"].append(trial)
        group["trial_span"]["finished_at"] = timestamp(previous_end)
        group["trial_span"]["elapsed_seconds"] = (
            previous_end - datetime.fromisoformat(group["trial_span"]["started_at"].replace("Z", "+00:00"))
        ).total_seconds()
    return evidence


class ContractTests(unittest.TestCase):
    def test_competitor_chunk_contract_distinguishes_progress_and_retry(self) -> None:
        partial = {
            "status": "skipped",
            "completion": "partial",
            "error_detail": None,
            "details": {
                "filesReleased": 3,
                "filesChecked": 4,
                "failures": 0,
            },
            "committed_work": 3,
            "committed_bytes": 3 * COMPETITOR_FILE_BYTES,
        }
        recoverable = {
            **partial,
            "details": {
                "filesReleased": 3,
                "filesChecked": 4,
                "failures": 1,
                "hasMore": True,
            },
        }
        self.assertTrue(task_run_is_accepted_competitor_chunk(partial))
        self.assertFalse(task_run_is_accepted_competitor_chunk(recoverable))
        self.assertTrue(task_run_is_recoverable_competitor_chunk(recoverable))

    def test_competitor_file_phase_defer_has_no_contribution(self) -> None:
        row = {
            "status": "skipped",
            "completion": "deferred",
            "error_detail": None,
            "committed_work": None,
            "committed_bytes": None,
            "details": {
                "admissionStage": "file_phase",
                "waitReason": "file_phase_busy",
                "filesChecked": None,
                "directoryEntriesChecked": None,
                "reconciliationRowsChecked": None,
                "filesReleased": None,
                "bytesReleased": None,
                "referencedSkipped": None,
                "quarantined": None,
                "failures": None,
            },
        }
        self.assertTrue(task_run_is_accepted_zero_contribution_competitor_defer(row))

    def test_fixed_competitor_file_oracle_is_required(self) -> None:
        evidence = valid_evidence()
        del evidence["versions"]["candidate"]["trials"][0]["competitor_file_oracle"]
        with self.assertRaisesRegex(AcceptanceError, "fixed competitor file oracle"):
            validate_evidence(evidence)

    def test_fixed_competitor_bytes_are_bound(self) -> None:
        evidence = valid_evidence()
        row = next(
            row
            for row in evidence["versions"]["candidate"]["trials"][0]["task_runs"]
            if row["task_key"] == "raw_orphan_sweep"
        )
        row["committed_bytes"] -= 1
        with self.assertRaisesRegex(AcceptanceError, "unexpected byte count"):
            validate_evidence(evidence)

    def test_legacy_partial_task_run_requires_full_selected_batch(self) -> None:
        row = {
            "status": "skipped",
            "completion": "partial",
            "error_detail": None,
            "details": {
                "result": "partial",
                "fatalError": None,
                "errorChain": None,
                "timeoutCount": 0,
                "selectedScope": {"selected": 1000, "committed": 1000},
                "archiveBatches": [{"batchRows": 1000, "committedRows": 1000, "timeoutCount": 0}],
            },
        }
        self.assertTrue(task_run_is_accepted_terminal(row))
        row["details"]["selectedScope"]["committed"] = 999
        self.assertFalse(task_run_is_accepted_terminal(row))

    def test_bounded_baseline_observation_requires_progress_and_backlog(self) -> None:
        row = {
            "trigger_kind": "schedule",
            "status": "skipped",
            "completion": "partial",
            "actual_finished_at": "2026-10-28T01:00:00Z",
            "error_detail": None,
            "details": {
                "result": "partial",
                "fatalError": None,
                "errorChain": None,
                "timeoutCount": 0,
                "backlogRemaining": 99186,
                "selectedScope": {"selected": 1000, "committed": 814, "hasMore": True},
                "archiveBatches": [{"batchRows": 1000, "committedRows": 814, "timeoutCount": 0}],
            },
        }
        self.assertTrue(task_run_is_bounded_baseline_observation(row))
        row["details"]["selectedScope"]["committed"] = 999
        row["details"]["archiveBatches"][0]["committedRows"] = 999
        self.assertTrue(task_run_is_bounded_baseline_observation(row))
        row["details"]["selectedScope"]["hasMore"] = False
        self.assertFalse(task_run_is_bounded_baseline_observation(row))

    def test_evidence_validator_accepts_legacy_partial_status_for_full_batch(self) -> None:
        evidence = valid_evidence()
        row = next(
            item
            for item in evidence["versions"]["baseline"]["trials"][0]["task_runs"]
            if item["task_key"] == "retention_archive" and item["trigger_kind"] == "schedule"
        )
        row.update(
            {
                "status": "skipped",
                "completion": "partial",
                "error_detail": None,
                "details": {
                    "result": "partial",
                    "fatalError": None,
                    "errorChain": None,
                    "timeoutCount": 0,
                    "selectedScope": {"selected": 1000, "committed": 1000},
                    "archiveBatches": [{"batchRows": 1000, "committedRows": 1000, "timeoutCount": 0}],
                },
            }
        )
        self.assertEqual(validate_evidence(evidence)["status"], "passed")

    def test_valid_evidence_passes(self) -> None:
        result = validate_evidence(valid_evidence())
        self.assertEqual(result["status"], "passed")

    def test_candidate_requires_complete_summary_proof(self) -> None:
        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        trial["archive_proof_status"]["ready"] = False
        trial["archive_proof_status"]["datasets"]["invocations"]["ready"] = False
        trial["archive_proof_status"]["datasets"]["invocations"]["summary_proof_complete"] = False
        trial["archive_proof_status"]["datasets"]["invocations"]["summary_proof_missing"] = ["summary proof:1"]
        with self.assertRaisesRegex(AcceptanceError, "candidate archive proof status"):
            validate_evidence(evidence)

    def test_failed_measurement_status_is_rejected(self) -> None:
        evidence = valid_evidence()
        evidence["status"] = "failed"
        with self.assertRaisesRegex(AcceptanceError, "not complete"):
            validate_evidence(evidence)

    def test_baseline_and_candidate_image_identity_are_bound(self) -> None:
        evidence = valid_evidence()
        evidence["versions"]["candidate"]["trials"][0]["source_sha"] = "c" * 40
        with self.assertRaisesRegex(AcceptanceError, "trial SHA drift"):
            validate_evidence(evidence)

    def test_candidate_cannot_reuse_the_baseline_sha(self) -> None:
        evidence = valid_evidence()
        evidence["contract"]["candidate_sha"] = BASELINE_SHA
        evidence["versions"]["candidate"]["image"]["source_sha"] = BASELINE_SHA
        for trial in evidence["versions"]["candidate"]["trials"]:
            trial["source_sha"] = BASELINE_SHA
        with self.assertRaisesRegex(AcceptanceError, "must differ from the baseline SHA"):
            validate_evidence(evidence)

    def test_driver_image_digest_is_bound_to_the_pinned_reference(self) -> None:
        evidence = valid_evidence()
        evidence["contract"]["driver_image_digest"] = "sha256:" + "f" * 64
        with self.assertRaisesRegex(AcceptanceError, "image digest drift"):
            validate_evidence(evidence)

    def test_trial_order_is_balanced_and_contract_bound(self) -> None:
        evidence = valid_evidence()
        evidence["trial_execution_order"][1], evidence["trial_execution_order"][2] = (
            evidence["trial_execution_order"][2],
            evidence["trial_execution_order"][1],
        )
        with self.assertRaisesRegex(AcceptanceError, "execution order drift"):
            validate_evidence(evidence)

    def test_foreground_driver_cleanup_must_be_terminal(self) -> None:
        evidence = valid_evidence()
        evidence["versions"]["candidate"]["trials"][0]["foreground_driver_cleanup"]["status"] = "running"
        with self.assertRaisesRegex(AcceptanceError, "driver cleanup is incomplete"):
            validate_evidence(evidence)

    def test_foreground_driver_cleanup_must_belong_to_its_trial(self) -> None:
        evidence = valid_evidence()
        evidence["versions"]["candidate"]["trials"][0]["foreground_driver_cleanup"]["container_name"] = (
            "retention-123456abcdef-candidate-2-driver"
        )
        with self.assertRaisesRegex(AcceptanceError, "belongs to another trial"):
            validate_evidence(evidence)

    def test_container_cleanup_identities_must_match_one_run_across_trials(self) -> None:
        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][1]
        trial["application_container_cleanup"]["container_name"] = "retention-fedcba987654-candidate-2"
        trial["application_container_cleanup"]["identity_labels"][CONTAINER_RUN_ID_LABEL] = "fedcba987654"
        trial["foreground_driver_cleanup"]["container_name"] = "retention-fedcba987654-candidate-2-driver"
        trial["foreground_driver_cleanup"]["identity_labels"][CONTAINER_RUN_ID_LABEL] = "fedcba987654"
        with self.assertRaisesRegex(AcceptanceError, "application container cleanup is incomplete"):
            validate_evidence(evidence)

    def test_driver_cleanup_run_id_must_match_its_application_trial(self) -> None:
        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        trial["foreground_driver_cleanup"]["identity_labels"][CONTAINER_RUN_ID_LABEL] = "fedcba987654"
        with self.assertRaisesRegex(AcceptanceError, "identity labels do not match its trial"):
            validate_evidence(evidence)

    def test_app_and_driver_must_have_distinct_generated_ownership_identities(self) -> None:
        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        trial["foreground_driver_cleanup"]["identity_labels"][CONTAINER_OWNER_LABEL] = (
            trial["application_container_cleanup"]["identity_labels"][CONTAINER_OWNER_LABEL]
        )
        with self.assertRaisesRegex(AcceptanceError, "ownership token was reused"):
            validate_evidence(evidence)

    def test_application_container_cleanup_must_be_terminal_and_identity_bound(self) -> None:
        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        trial["application_container_cleanup"]["status"] = "running"
        with self.assertRaisesRegex(AcceptanceError, "application container cleanup is incomplete"):
            validate_evidence(evidence)

        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        trial["application_container_cleanup"]["container_id"] = "5" * 64
        with self.assertRaisesRegex(AcceptanceError, "application container cleanup is incomplete"):
            validate_evidence(evidence)

    def test_attempt_state_must_be_terminal_and_match_the_invocation_map(self) -> None:
        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        trial["attempt_states"][0]["status"] = "pending"
        with self.assertRaisesRegex(AcceptanceError, "did not reach a terminal state"):
            validate_evidence(evidence)

        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        trial["attempt_states"][0]["invoke_id"] = "unexpected-invocation"
        with self.assertRaisesRegex(AcceptanceError, "invocation is unexpected"):
            validate_evidence(evidence)

    def test_runner_and_resource_drift_are_rejected(self) -> None:
        evidence = valid_evidence()
        evidence["versions"]["candidate"]["resource_limits"]["cpus"] = 4
        with self.assertRaisesRegex(AcceptanceError, "resource limits drifted"):
            validate_evidence(evidence)

    def test_inter_version_gap_cannot_be_omitted(self) -> None:
        evidence = valid_evidence()
        evidence["versions"]["candidate"]["trials"][0]["window"]["gap_before_seconds"] = 0.0
        with self.assertRaisesRegex(AcceptanceError, "inter-run gap was omitted"):
            validate_evidence(evidence)

    def test_evidence_validator_accepts_bounded_active_baseline_owners(self) -> None:
        evidence = valid_evidence()
        for trial in evidence["versions"]["baseline"]["trials"]:
            input_end = datetime.fromisoformat(trial["window"]["input_finished_at"].replace("Z", "+00:00"))
            observed_at = input_end - timedelta(seconds=1)
            progress_at = observed_at - timedelta(seconds=1)
            retained = []
            for row in trial["task_runs"]:
                if row["task_key"] not in {definition["task_key"] for definition in DATASETS.values()}:
                    retained.append(row)
                    continue
                if row["trigger_kind"] != "schedule":
                    continue
                row.update(
                    {
                        "status": "running",
                        "actual_finished_at": None,
                        "committed_work": 0,
                        "evidence_kind": "bounded_active",
                        "started_at": timestamp(observed_at - timedelta(seconds=3)),
                        "activity_started_at": timestamp(observed_at - timedelta(seconds=2)),
                        "observed_at": timestamp(observed_at),
                        "progress": {
                            "phase": "running",
                            "checkpoint": "admitted",
                            "last_progress_at": timestamp(progress_at),
                        },
                    }
                )
                retained.append(row)
            trial["task_runs"] = retained

        self.assertEqual(validate_evidence(evidence)["status"], "passed")

        evidence = valid_evidence()
        trial = evidence["versions"]["baseline"]["trials"][0]
        active = next(row for row in trial["task_runs"] if row["task_key"] == "retention_archive")
        active.update({"status": "running", "actual_finished_at": None, "evidence_kind": "bounded_active", "started_at": active["actual_started_at"], "activity_started_at": active["actual_started_at"]})
        with self.assertRaisesRegex(AcceptanceError, "bounded progress snapshot"):
            validate_evidence(evidence)

    def test_evidence_validator_accepts_candidate_proof_edge_catchup_owner(self) -> None:
        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        input_start = datetime.fromisoformat(trial["window"]["input_started_at"].replace("Z", "+00:00"))
        input_end = datetime.fromisoformat(trial["window"]["input_finished_at"].replace("Z", "+00:00"))
        observed_at = input_end - timedelta(seconds=1)
        progress_at = observed_at - timedelta(seconds=1)
        active = next(
            row
            for row in trial["task_runs"]
            if row["task_key"] == "pool_upstream_attempt_archive" and row["trigger_kind"] == "catchup"
        )
        active.update(
            {
                "status": "running",
                "actual_finished_at": None,
                "committed_work": 1000,
                "evidence_kind": "proof_edge_active",
                "started_at": timestamp(input_start + timedelta(seconds=5)),
                "activity_started_at": timestamp(input_start + timedelta(seconds=6)),
                "observed_at": timestamp(observed_at),
                "progress": {
                    "total": 1000,
                    "completed": 0,
                    "phase": "running",
                    "checkpoint": "admitted",
                    "freshness": "fresh",
                    "wait_reason": "retention_write_admission",
                    "last_progress_at": timestamp(progress_at),
                },
            }
        )

        self.assertEqual(validate_evidence(evidence)["status"], "passed")

        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        active = next(
            row
            for row in trial["task_runs"]
            if row["task_key"] == "pool_upstream_attempt_archive" and row["trigger_kind"] == "catchup"
        )
        active.update(
            {
                "status": "running",
                "actual_finished_at": None,
                "evidence_kind": "proof_edge_active",
                "started_at": trial["window"]["input_started_at"],
                "activity_started_at": trial["window"]["input_started_at"],
                "observed_at": trial["window"]["cleanup_finished_at"],
                "progress": {
                    "total": 1000,
                    "completed": 1000,
                    "phase": "processing",
                    "checkpoint": "waiting",
                    "last_progress_at": trial["window"]["input_finished_at"],
                },
            }
        )
        with self.assertRaisesRegex(AcceptanceError, "remained active"):
            validate_evidence(evidence)

    def test_live_write_count_cannot_be_replaced_with_a_reported_count(self) -> None:
        evidence = valid_evidence()
        result = evidence["versions"]["candidate"]["trials"][0]["oracle"]["datasets"]["invocations"]
        result["actual_added_rows"] += 1
        result["live_rows"] += 1
        with self.assertRaisesRegex(AcceptanceError, "addition count is not bound"):
            validate_evidence(evidence)

    def test_missing_catchup_or_competitor_work_is_rejected(self) -> None:
        evidence = valid_evidence()
        rows = evidence["versions"]["baseline"]["trials"][0]["task_runs"]
        evidence["versions"]["baseline"]["trials"][0]["task_runs"] = [row for row in rows if row["task_key"] != "raw_orphan_sweep"]
        with self.assertRaisesRegex(AcceptanceError, "competitor evidence is missing"):
            validate_evidence(evidence)

    def test_catchup_runs_cannot_replace_a_scheduled_owner_run(self) -> None:
        evidence = valid_evidence()
        rows = evidence["versions"]["baseline"]["trials"][0]["task_runs"]
        for row in rows:
            if row["task_key"] == "retention_archive":
                row["trigger_kind"] = "catchup"
        with self.assertRaisesRegex(AcceptanceError, "both schedule and catch-up"):
            validate_evidence(evidence)

    def test_baseline_can_serialize_the_competitor_while_candidate_must_overlap(self) -> None:
        evidence = valid_evidence()
        for trial in evidence["versions"]["baseline"]["trials"]:
            competitor = next(row for row in trial["task_runs"] if row["task_key"] == "raw_orphan_sweep")
            started = datetime.fromisoformat(trial["window"]["setup_started_at"].replace("Z", "+00:00"))
            competitor["actual_started_at"] = timestamp(started + timedelta(seconds=20))
            competitor["actual_finished_at"] = timestamp(started + timedelta(seconds=25))
        self.assertEqual(validate_evidence(evidence)["status"], "passed")

    def test_candidate_must_overlap_the_competitor(self) -> None:
        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        competitor = next(row for row in trial["task_runs"] if row["task_key"] == "raw_orphan_sweep")
        started = datetime.fromisoformat(trial["window"]["setup_started_at"].replace("Z", "+00:00"))
        competitor["actual_started_at"] = timestamp(started + timedelta(seconds=20))
        competitor["actual_finished_at"] = timestamp(started + timedelta(seconds=25))
        with self.assertRaisesRegex(AcceptanceError, "candidate raw orphan-sweep did not overlap"):
            validate_evidence(evidence)

    def test_candidate_must_make_owner_progress_during_foreground_input(self) -> None:
        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        input_end = datetime.fromisoformat(trial["window"]["input_finished_at"].replace("Z", "+00:00"))
        owner_offsets = {
            "retention_archive": (0.5, 0.75, 1.0, 1.25),
            "pool_upstream_attempt_archive": (0.75, 1.0, 1.25, 1.5),
        }
        for row in trial["task_runs"]:
            offsets = owner_offsets.get(row["task_key"])
            if offsets is None:
                continue
            scheduled_start, scheduled_finish, catchup_start, catchup_finish = offsets
            if row["trigger_kind"] == "schedule":
                start_offset, finish_offset = scheduled_start, scheduled_finish
            else:
                start_offset, finish_offset = catchup_start, catchup_finish
            row["actual_started_at"] = timestamp(input_end + timedelta(seconds=start_offset))
            row["actual_finished_at"] = timestamp(input_end + timedelta(seconds=finish_offset))
        with self.assertRaisesRegex(AcceptanceError, "made no progress during foreground input"):
            validate_evidence(evidence)

    def test_latency_samples_must_measure_production_http_requests(self) -> None:
        evidence = valid_evidence()
        sample = evidence["versions"]["candidate"]["trials"][0]["latency_samples_ms"]["write"][0]
        sample["transport"] = "sqlite"
        with self.assertRaisesRegex(AcceptanceError, "production HTTP boundary"):
            validate_evidence(evidence)

    def test_write_timing_fields_are_required_and_bound_to_each_invocation(self) -> None:
        fields = (
            "sequence", "invoke_id", "scheduled_offset_ms", "dispatched_offset_ms",
            "completed_offset_ms", "dispatch_lag_ms", "request_elapsed_ms",
        )
        for field in fields:
            with self.subTest(field=field):
                evidence = valid_evidence()
                del evidence["versions"]["candidate"]["trials"][0]["latency_samples_ms"]["write"][0][field]
                with self.assertRaises(AcceptanceError):
                    validate_evidence(evidence)
        for field, value in (("sequence", True), ("sequence", 1), ("invoke_id", "unrelated-invocation")):
            with self.subTest(field=field, value=value):
                evidence = valid_evidence()
                evidence["versions"]["candidate"]["trials"][0]["latency_samples_ms"]["write"][0][field] = value
                with self.assertRaisesRegex(AcceptanceError, "timing sequence|timing identity"):
                    validate_evidence(evidence)

    def test_write_timing_values_must_be_finite_nonnegative_numbers(self) -> None:
        for field in ("scheduled_offset_ms", "dispatched_offset_ms", "completed_offset_ms", "dispatch_lag_ms", "request_elapsed_ms"):
            for value in (None, True, -1, math.nan, math.inf, "1"):
                with self.subTest(field=field, value=value):
                    evidence = valid_evidence()
                    evidence["versions"]["candidate"]["trials"][0]["latency_samples_ms"]["write"][0][field] = value
                    with self.assertRaisesRegex(AcceptanceError, f"invalid raw write {field}"):
                        validate_evidence(evidence)

    def test_write_timing_cannot_shift_the_arrival_plan_to_hide_queueing(self) -> None:
        evidence = valid_evidence()
        sample = evidence["versions"]["candidate"]["trials"][0]["latency_samples_ms"]["write"][0]
        for field in ("scheduled_offset_ms", "dispatched_offset_ms", "completed_offset_ms"):
            sample[field] += 100
        sample["observed_at"] = timestamp(datetime.fromisoformat(sample["observed_at"]) + timedelta(milliseconds=100))
        with self.assertRaisesRegex(AcceptanceError, "fixed arrival plan"):
            validate_evidence(evidence)

    def test_write_timing_rejects_omitted_or_reordered_requests(self) -> None:
        for reorder in (False, True):
            with self.subTest(reorder=reorder):
                evidence = valid_evidence()
                samples = evidence["versions"]["candidate"]["trials"][0]["latency_samples_ms"]["write"]
                if reorder:
                    samples[0], samples[1] = samples[1], samples[0]
                else:
                    samples.pop()
                with self.assertRaisesRegex(AcceptanceError, "every invocation|incomplete or reordered"):
                    validate_evidence(evidence)

    def test_write_timing_rejects_inconsistent_latency_components(self) -> None:
        for field, expected_error in (
            ("dispatch_lag_ms", "dispatch lag does not match"),
            ("request_elapsed_ms", "request latency does not match"),
            ("elapsed_ms", "end-to-end latency omitted"),
        ):
            with self.subTest(field=field):
                evidence = valid_evidence()
                trial = evidence["versions"]["candidate"]["trials"][0]
                start = datetime.fromisoformat(trial["window"]["input_started_at"])
                sample = write_sample_row(start, 0, trial["added_ids"]["invocations"][0], request_ms=1.0, lag_ms=20.0)
                sample[field] = 0.0 if field == "dispatch_lag_ms" else 1.0 if field == "elapsed_ms" else 2.0
                trial["latency_samples_ms"]["write"][0] = sample
                with self.assertRaisesRegex(AcceptanceError, expected_error):
                    validate_evidence(evidence)

    def test_write_completion_must_match_the_observed_wall_timestamp(self) -> None:
        evidence = valid_evidence()
        sample = evidence["versions"]["candidate"]["trials"][0]["latency_samples_ms"]["write"][0]
        sample["observed_at"] = timestamp(datetime.fromisoformat(sample["observed_at"]) + timedelta(seconds=1))
        with self.assertRaisesRegex(AcceptanceError, "completion time is not bound"):
            validate_evidence(evidence)

    def test_serial_writer_cannot_dispatch_before_arrival_or_previous_completion(self) -> None:
        for overlap in (False, True):
            with self.subTest(overlap=overlap):
                evidence = valid_evidence()
                trial = evidence["versions"]["candidate"]["trials"][0]
                samples = trial["latency_samples_ms"]["write"]
                if overlap:
                    start = datetime.fromisoformat(trial["window"]["input_started_at"])
                    samples[0] = write_sample_row(start, 0, trial["added_ids"]["invocations"][0], request_ms=2000.0)
                else:
                    samples[1]["dispatched_offset_ms"] -= 1.0
                with self.assertRaisesRegex(AcceptanceError, "dispatch/completion order"):
                    validate_evidence(evidence)

    def test_write_percentiles_include_queueing_even_when_http_requests_are_faster(self) -> None:
        for delayed_count, percentile_name in ((1, "p99_ms"), (5, "p95_ms")):
            with self.subTest(percentile=percentile_name):
                evidence = valid_evidence()
                for trial in evidence["versions"]["candidate"]["trials"]:
                    start = datetime.fromisoformat(trial["window"]["input_started_at"])
                    samples = trial["latency_samples_ms"]["write"]
                    for sequence in range(len(samples) - delayed_count, len(samples)):
                        samples[sequence] = write_sample_row(start, sequence, trial["added_ids"]["invocations"][sequence], request_ms=1.0, lag_ms=20.0)
                with self.assertRaisesRegex(AcceptanceError, f"write {percentile_name} median regressed"):
                    validate_evidence(evidence)

    def test_consistent_nonzero_dispatch_lag_is_included_in_both_percentiles(self) -> None:
        evidence = valid_evidence()
        for group in evidence["versions"].values():
            for trial in group["trials"]:
                start = datetime.fromisoformat(trial["window"]["input_started_at"])
                trial["latency_samples_ms"]["write"] = [
                    write_sample_row(start, sequence, invoke_id, request_ms=1.0, lag_ms=20.0)
                    for sequence, invoke_id in enumerate(trial["added_ids"]["invocations"])
                ]
        result = validate_evidence(evidence)
        for version in ("baseline", "candidate"):
            for percentile in ("p95_ms", "p99_ms"):
                self.assertAlmostEqual(result["latency_summary"][version]["write"][percentile], 21.0)

    def test_mock_retry_workload_is_bound_to_the_expected_attempt_count(self) -> None:
        evidence = valid_evidence()
        upstream = evidence["versions"]["candidate"]["trials"][0]["foreground_workload"]["mock_upstream"]
        upstream["retryable_failures"] -= 1
        with self.assertRaisesRegex(AcceptanceError, "retry workload drifted"):
            validate_evidence(evidence)

    def test_baseline_and_candidate_must_complete_the_same_competitor_work(self) -> None:
        evidence = valid_evidence()
        row = next(
            item
            for item in evidence["versions"]["candidate"]["trials"][0]["task_runs"]
            if item["task_key"] == "raw_orphan_sweep"
        )
        row["committed_work"] -= 1
        with self.assertRaisesRegex(AcceptanceError, "fixed 512-file workload"):
            validate_evidence(evidence)

    def test_missing_oracle_and_lost_rows_are_rejected(self) -> None:
        evidence = valid_evidence()
        evidence["versions"]["candidate"]["trials"][2]["oracle"]["verified"] = False
        with self.assertRaisesRegex(AcceptanceError, "source/artifact oracle evidence"):
            validate_evidence(evidence)

        evidence = valid_evidence()
        evidence["versions"]["candidate"]["trials"][0]["oracle"]["datasets"]["invocations"]["lost_rows"] = 1
        with self.assertRaisesRegex(AcceptanceError, "found lost"):
            validate_evidence(evidence)

    def test_write_hold_is_recomputed_from_raw_histogram_scrapes(self) -> None:
        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        trial["write_hold_buckets"]["0.25"] += 1
        with self.assertRaisesRegex(AcceptanceError, "not derived from raw scrape deltas"):
            validate_evidence(evidence)

    def test_candidate_write_hold_above_threshold_is_rejected(self) -> None:
        evidence = valid_evidence()
        trial = evidence["versions"]["candidate"]["trials"][0]
        for operation in trial["write_hold_samples"][-1]["bucket_counts"]:
            trial["write_hold_samples"][-1]["bucket_counts"][operation] = {"0.1": 0, "0.25": 0, "0.5": 20, "+Inf": 20}
        trial["write_hold_buckets"] = {"0.1": 0, "0.25": 0, "0.5": 40, "+Inf": 40}
        with self.assertRaisesRegex(AcceptanceError, "write-hold p95 exceeded"):
            validate_evidence(evidence)

    def test_candidate_convergence_includes_the_full_trial_window(self) -> None:
        evidence = valid_evidence()
        result = evidence["versions"]["candidate"]["trials"][0]["oracle"]["datasets"]["invocations"]
        additions = result["actual_added_rows"]
        result["converted_rows"] = additions * 10
        result["distinct_archived_rows"] = result["converted_rows"]
        result["live_rows"] = result["seed_rows"] + additions - result["converted_rows"]
        result["sustained_convergence_factor"] = 10.0
        with self.assertRaisesRegex(AcceptanceError, "sustained convergence factor"):
            validate_evidence(evidence)

    def test_test_phase_cannot_exceed_900_seconds(self) -> None:
        evidence = valid_evidence()
        end = datetime.fromisoformat(evidence["test_phase"]["started_at"].replace("Z", "+00:00")) + timedelta(seconds=901)
        evidence["test_phase"]["finished_at"] = timestamp(end)
        evidence["test_phase"]["elapsed_seconds"] = 901.0
        evidence["comparison_window"]["finished_at"] = timestamp(end)
        evidence["comparison_window"]["elapsed_seconds"] = 901.0
        with self.assertRaisesRegex(AcceptanceError, "exceeded 900 seconds"):
            validate_evidence(evidence)

    def test_trial_window_must_fit_inside_test_phase(self) -> None:
        evidence = valid_evidence()
        phase_start = datetime.fromisoformat(evidence["test_phase"]["started_at"].replace("Z", "+00:00"))
        trial = evidence["versions"]["baseline"]["trials"][0]
        trial_start = phase_start - timedelta(seconds=1)
        trial["window"]["setup_started_at"] = timestamp(trial_start)
        trial["window"]["full_duration_seconds"] = 124.0
        group = evidence["versions"]["baseline"]["trial_span"]
        group_start = phase_start - timedelta(seconds=1)
        group["started_at"] = timestamp(group_start)
        group["elapsed_seconds"] = (
            datetime.fromisoformat(group["finished_at"].replace("Z", "+00:00")) - group_start
        ).total_seconds()
        with self.assertRaisesRegex(AcceptanceError, "outside the test phase"):
            validate_evidence(evidence)


if __name__ == "__main__":
    unittest.main()
