"""Shared, fail-closed contract for the isolated retention acceptance run."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
import hashlib
import json
import math
import re
from statistics import median
from typing import Any


BASELINE_SHA = "dff0b88dc00681720acfaa9103c8ddbba84d3325"
DATASETS = {
    "invocations": {
        "table": "codex_invocations",
        "identity_column": "invoke_id",
        "manifest_dataset": "codex_invocations",
        "seed_rows": 100_000,
        "arrival_rate": 0.673,
        "task_key": "retention_archive",
    },
    "pool_upstream_attempts": {
        "table": "pool_upstream_request_attempts",
        "identity_column": "attempt_public_id",
        "manifest_dataset": "pool_upstream_request_attempts",
        "seed_rows": 120_000,
        "arrival_rate": 0.673 * 1.2,
        "task_key": "pool_upstream_attempt_archive",
    },
}
COMPETITOR_TASK = "raw_orphan_sweep"
COMPETITOR_FILE_COUNT = 512
COMPETITOR_FILE_BYTES = 256
COMPETITOR_START_DELAY_SECONDS = 10
RUN_SECONDS = 120
TRIALS_PER_VERSION = 3
MAX_TEST_PHASE_SECONDS = 900
CLEANUP_RESERVE_SECONDS = 45
MIN_CONVERGENCE_FACTOR = 10.0
MAX_WRITE_HOLD_P95_MS = 250.0
WRITE_TIMING_TOLERANCE_MS = 0.01
SEED = 20261024
RETENTION_DAYS = 1
SCHEMA_VERSION = 6
CONTAINER_LABEL_PREFIX = "io.github.ivanli.codex-vibe-monitor.retention."
CONTAINER_RUN_ID_LABEL = CONTAINER_LABEL_PREFIX + "run-id"
CONTAINER_KIND_LABEL = CONTAINER_LABEL_PREFIX + "kind"
CONTAINER_VERSION_LABEL = CONTAINER_LABEL_PREFIX + "version"
CONTAINER_TRIAL_LABEL = CONTAINER_LABEL_PREFIX + "trial"
CONTAINER_OWNER_LABEL = CONTAINER_LABEL_PREFIX + "owner"
DRIVER_IMAGE_REF = "python:3.12-alpine@sha256:b658b05ce9c4eb97ec6fc19424818a1f31993659c2b505a532f470e83807815d"
DRIVER_IMAGE_DIGEST = "sha256:b658b05ce9c4eb97ec6fc19424818a1f31993659c2b505a532f470e83807815d"
TRIAL_EXECUTION_ORDER = [
    {"version": "baseline", "trial_index": 1},
    {"version": "candidate", "trial_index": 1},
    {"version": "candidate", "trial_index": 2},
    {"version": "baseline", "trial_index": 2},
    {"version": "baseline", "trial_index": 3},
    {"version": "candidate", "trial_index": 3},
]


class AcceptanceError(ValueError):
    """The evidence is incomplete, inconsistent, or failed an acceptance gate."""


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_json(value: Any) -> str:
    return sha256_bytes(canonical_json(value))


def parse_time(value: str) -> datetime:
    if not isinstance(value, str) or not value:
        raise AcceptanceError("missing timestamp")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise AcceptanceError("invalid timestamp") from error
    if parsed.tzinfo is None:
        # SQLite invocation timestamps are naive Asia/Shanghai local time; evidence timestamps
        # produced by the runner normally carry an explicit UTC offset.
        parsed = parsed.replace(tzinfo=timezone(timedelta(hours=8)))
    return parsed.astimezone(timezone.utc)


def task_run_is_accepted_terminal(row: dict[str, Any]) -> bool:
    """Accept a legacy partial status only when its selected batch was fully committed."""
    if row.get("status") == "success":
        return True
    if row.get("status") != "skipped" or row.get("completion") != "partial":
        return False
    if row.get("error_detail") is not None:
        return False
    details = row.get("details")
    if not isinstance(details, dict):
        return False
    if details.get("result") not in (None, "partial"):
        return False
    if details.get("fatalError") is not None or details.get("errorChain") is not None:
        return False
    if details.get("timeoutCount", 0) != 0:
        return False
    scope = details.get("selectedScope")
    if not isinstance(scope, dict):
        return False
    selected = scope.get("selected")
    committed = scope.get("committed")
    if (
        not isinstance(selected, int)
        or isinstance(selected, bool)
        or selected <= 0
        or not isinstance(committed, int)
        or isinstance(committed, bool)
        or committed != selected
    ):
        return False
    batches = details.get("archiveBatches")
    if not isinstance(batches, list) or not batches:
        return False
    return all(
        isinstance(batch, dict)
        and batch.get("batchRows") == batch.get("committedRows")
        and batch.get("timeoutCount", 0) == 0
        for batch in batches
    )


def task_run_is_bounded_baseline_observation(row: dict[str, Any]) -> bool:
    """Accept a productive, pressure-bounded legacy schedule observation.

    The baseline is a reference measurement and may finish its first selected batch
    with a recoverable partial commit before catch-up takes over. This exception is
    intentionally narrower than a terminal run and is never used for the candidate.
    """
    if (
        row.get("trigger_kind") != "schedule"
        or row.get("status") != "skipped"
        or row.get("completion") != "partial"
    ):
        return False
    if row.get("actual_finished_at") is None or row.get("error_detail") is not None:
        return False
    details = row.get("details")
    if not isinstance(details, dict):
        return False
    if (
        details.get("result") != "partial"
        or details.get("fatalError") is not None
        or details.get("errorChain") is not None
        or details.get("timeoutCount", 0) != 0
    ):
        return False
    scope = details.get("selectedScope")
    batches = details.get("archiveBatches")
    if not isinstance(scope, dict) or not isinstance(batches, list) or not batches:
        return False
    selected = scope.get("selected")
    committed = scope.get("committed")
    if (
        not isinstance(selected, int)
        or isinstance(selected, bool)
        or selected <= 0
        or not isinstance(committed, int)
        or isinstance(committed, bool)
        or not 0 < committed < selected
        or scope.get("hasMore") is not True
    ):
        return False
    backlog = details.get("backlogRemaining")
    if not isinstance(backlog, int) or isinstance(backlog, bool) or backlog <= 0:
        return False
    total_rows = 0
    committed_rows = 0
    for batch in batches:
        if not isinstance(batch, dict) or batch.get("timeoutCount", 0) != 0:
            return False
        batch_rows = batch.get("batchRows")
        batch_committed = batch.get("committedRows")
        if (
            not isinstance(batch_rows, int)
            or isinstance(batch_rows, bool)
            or batch_rows <= 0
            or not isinstance(batch_committed, int)
            or isinstance(batch_committed, bool)
            or not 0 < batch_committed <= batch_rows
        ):
            return False
        total_rows += batch_rows
        committed_rows += batch_committed
    return total_rows == selected and committed_rows == committed


def task_run_is_accepted_competitor_chunk(row: dict[str, Any]) -> bool:
    """Accept productive raw-sweep chunks while retaining the file oracle."""
    if row.get("status") == "success":
        details = row.get("details")
        if details is None:
            return True
        if not isinstance(details, dict):
            return False
        failures = details.get("failures", 0)
        return isinstance(failures, int) and not isinstance(failures, bool) and failures == 0
    if row.get("status") != "skipped" or row.get("completion") not in ("partial", "deferred"):
        return False
    if row.get("error_detail") is not None:
        return False
    details = row.get("details")
    if not isinstance(details, dict):
        return False
    failures = details.get("failures", 0)
    if not isinstance(failures, int) or isinstance(failures, bool) or failures != 0:
        return False
    released = details.get("filesReleased")
    checked = details.get("filesChecked")
    return (
        isinstance(released, int)
        and not isinstance(released, bool)
        and released > 0
        and isinstance(checked, int)
        and not isinstance(checked, bool)
        and checked >= released
    )


def task_run_is_accepted_zero_contribution_competitor_defer(row: dict[str, Any]) -> bool:
    """Accept a file-phase defer without counting work that did not run."""
    if row.get("status") != "skipped" or row.get("completion") not in ("partial", "deferred"):
        return False
    if row.get("error_detail") is not None:
        return False
    if (
        "committed_work" not in row
        or row["committed_work"] is not None
        or "committed_bytes" not in row
        or row["committed_bytes"] is not None
    ):
        return False
    details = row.get("details")
    if not isinstance(details, dict):
        return False
    if (details.get("admissionStage"), details.get("waitReason")) not in {
        ("file_phase", "file_phase_busy"),
        ("archive_file_phase", "archive_directory_lock_busy"),
    }:
        return False
    return all(
        field in details and details[field] is None
        for field in (
            "filesChecked",
            "directoryEntriesChecked",
            "reconciliationRowsChecked",
            "filesReleased",
            "bytesReleased",
            "referencedSkipped",
            "quarantined",
            "failures",
        )
    )


def task_run_is_recoverable_competitor_chunk(row: dict[str, Any]) -> bool:
    """Recognize a progressed failed chunk that must be followed by a successful retry."""
    if row.get("status") != "skipped" or row.get("completion") not in ("partial", "deferred"):
        return False
    if row.get("error_detail") is not None:
        return False
    details = row.get("details")
    if not isinstance(details, dict) or details.get("hasMore") is not True:
        return False
    released = details.get("filesReleased")
    checked = details.get("filesChecked")
    failures = details.get("failures", 0)
    return (
        isinstance(released, int)
        and not isinstance(released, bool)
        and released > 0
        and isinstance(checked, int)
        and not isinstance(checked, bool)
        and checked >= released
        and isinstance(failures, int)
        and not isinstance(failures, bool)
        and failures > 0
    )


def task_run_is_bounded_active(row: dict[str, Any]) -> bool:
    """Accept only a measured baseline owner that is still running at the window bound."""
    progress = row.get("progress")
    return (
        row.get("evidence_kind") == "bounded_active"
        and row.get("status") == "running"
        and row.get("actual_finished_at") is None
        and isinstance(row.get("started_at"), str)
        and isinstance(row.get("activity_started_at"), str)
        and isinstance(row.get("observed_at"), str)
        and isinstance(progress, dict)
        and progress.get("phase") in {"running", "processing"}
        and progress.get("checkpoint") in {"admitted", "processing"}
        and isinstance(progress.get("last_progress_at"), str)
    )


def _number(value: Any, message: str) -> float:
    _require(
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value),
        message,
    )
    return float(value)


def percentile(values: list[float], fraction: float) -> float:
    if not values or not 0 < fraction <= 1:
        raise AcceptanceError("latency sample set is empty or percentile is invalid")
    ordered = sorted(values)
    rank = max(1, math.ceil(fraction * len(ordered)))
    return ordered[rank - 1]


def latency_summary(samples: list[float]) -> dict[str, float]:
    return {
        "p95_ms": percentile(samples, 0.95),
        "p99_ms": percentile(samples, 0.99),
    }


def histogram_quantile_ms(buckets: dict[str, int], fraction: float) -> float:
    if not buckets or not 0 < fraction <= 1:
        raise AcceptanceError("write-hold histogram is missing")
    parsed: list[tuple[float, int]] = []
    for upper, count in buckets.items():
        try:
            bound = math.inf if upper == "+Inf" else float(upper)
        except (TypeError, ValueError) as error:
            raise AcceptanceError("write-hold histogram has an invalid bucket") from error
        if isinstance(count, bool) or not isinstance(count, int) or count < 0:
            raise AcceptanceError("write-hold histogram has an invalid count")
        parsed.append((bound, count))
    parsed.sort(key=lambda item: item[0])
    if not parsed or parsed[-1][0] != math.inf:
        raise AcceptanceError("write-hold histogram has no +Inf bucket")
    previous = 0
    for _, count in parsed:
        if count < previous:
            raise AcceptanceError("write-hold histogram buckets are not cumulative")
        previous = count
    total = parsed[-1][1]
    if total == 0:
        raise AcceptanceError("write-hold histogram contains no observations")
    rank = math.ceil(fraction * total)
    for upper, count in parsed:
        if count >= rank:
            return upper * 1000 if math.isfinite(upper) else math.inf
    raise AcceptanceError("write-hold histogram percentile is unavailable")


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise AcceptanceError(message)


def _validate_container_identity_labels(
    cleanup: dict[str, Any],
    run_id: str,
    kind: str,
    version: str,
    trial_index: int,
    owner_tokens: set[str],
) -> None:
    labels = cleanup.get("identity_labels")
    _require(isinstance(labels, dict), f"{kind} container identity labels are missing")
    expected = {
        CONTAINER_RUN_ID_LABEL: run_id,
        CONTAINER_KIND_LABEL: kind,
        CONTAINER_VERSION_LABEL: version,
        CONTAINER_TRIAL_LABEL: str(trial_index),
    }
    _require(
        all(labels.get(key) == value for key, value in expected.items()),
        f"{kind} container identity labels do not match its trial",
    )
    owner_token = labels.get(CONTAINER_OWNER_LABEL)
    _require(
        isinstance(owner_token, str) and re.fullmatch(r"[0-9a-f]{32}", owner_token),
        f"{kind} container ownership token is invalid",
    )
    _require(owner_token not in owner_tokens, "container ownership token was reused")
    owner_tokens.add(owner_token)


def _validate_samples(trial: dict[str, Any], input_start: datetime, input_end: datetime) -> None:
    samples = trial.get("latency_samples_ms")
    _require(isinstance(samples, dict), "raw foreground latency evidence is missing")
    for kind in ("read", "write"):
        values = samples.get(kind)
        _require(isinstance(values, list) and len(values) >= 20, f"insufficient raw {kind} latency samples")
        if kind == "write":
            _require(len(values) == len(trial["added_ids"]["invocations"]), "foreground write timing evidence does not cover every invocation")
        previous_completion = 0.0
        for sequence, sample in enumerate(values):
            _require(isinstance(sample, dict), f"malformed raw {kind} latency sample")
            observed = sample.get("observed_at")
            latency = sample.get("elapsed_ms")
            observed_at = parse_time(observed)
            _require(input_start <= observed_at <= input_end, f"{kind} latency sample fell outside its window")
            _require(
                isinstance(latency, (int, float))
                and not isinstance(latency, bool)
                and math.isfinite(latency)
                and latency >= 0,
                f"invalid raw {kind} latency",
            )
            expected_path = "/api/pool/upstream-accounts" if kind == "read" else "/v1/responses"
            _require(sample.get("transport") == "http" and sample.get("path") == expected_path, f"{kind} latency was not measured at the production HTTP boundary")
            if kind == "write":
                _require(
                    isinstance(sample.get("sequence"), int)
                    and not isinstance(sample["sequence"], bool)
                    and sample["sequence"] == sequence,
                    "foreground write timing sequence is incomplete or reordered",
                )
                _require(sample.get("invoke_id") == trial["added_ids"]["invocations"][sequence], "foreground write timing identity does not match its measured invocation")
                timing = {}
                for field in ("scheduled_offset_ms", "dispatched_offset_ms", "completed_offset_ms", "dispatch_lag_ms", "request_elapsed_ms"):
                    value = _number(sample.get(field), f"invalid raw write {field}")
                    _require(value >= 0, f"invalid raw write {field}")
                    timing[field] = value
                scheduled = timing["scheduled_offset_ms"]
                dispatched = timing["dispatched_offset_ms"]
                completed = timing["completed_offset_ms"]
                expected_arrival = sequence / DATASETS["invocations"]["arrival_rate"] * 1000
                _require(abs(scheduled - expected_arrival) <= WRITE_TIMING_TOLERANCE_MS, "foreground write schedule drifted from its fixed arrival plan")
                _require(
                    scheduled <= dispatched <= completed
                    and dispatched + WRITE_TIMING_TOLERANCE_MS >= previous_completion,
                    "foreground write dispatch/completion order is invalid",
                )
                _require(
                    abs(timing["dispatch_lag_ms"] - (dispatched - scheduled)) <= WRITE_TIMING_TOLERANCE_MS,
                    "foreground write dispatch lag does not match its arrival and dispatch times",
                )
                _require(
                    abs(timing["request_elapsed_ms"] - (completed - dispatched)) <= WRITE_TIMING_TOLERANCE_MS,
                    "foreground write request latency does not match its dispatch and completion times",
                )
                _require(
                    abs(latency - (completed - scheduled)) <= WRITE_TIMING_TOLERANCE_MS,
                    "foreground write end-to-end latency omitted dispatch lag or completion time",
                )
                # Wall timestamps have millisecond precision; durations use the monotonic clock.
                observed_offset_ms = (observed_at - input_start).total_seconds() * 1000
                _require(abs(observed_offset_ms - completed) <= 2.0, "foreground write completion time is not bound to its observed timestamp")
                previous_completion = completed


def _validate_task_runs(
    trial: dict[str, Any],
    setup_start: datetime,
    input_end: datetime,
    cleanup_end: datetime,
    require_competitor_overlap: bool,
    version: str,
    competitor_oracle: dict[str, Any] | None = None,
) -> int:
    rows = trial.get("task_runs")
    _require(isinstance(rows, list) and rows, "production dispatcher task-run evidence is missing")
    by_key: dict[str, list[dict[str, Any]]] = {}
    recoverable_competitor_rows: list[dict[str, Any]] = []
    for row in rows:
        _require(isinstance(row, dict), "malformed production task-run evidence")
        key = row.get("task_key")
        _require(isinstance(key, str) and key, "task run has no owner key")
        started = parse_time(row.get("activity_started_at") or row.get("actual_started_at"))
        active = row.get("actual_finished_at") is None
        if active:
            _require(
                version == "baseline" and key in {DATASETS[name]["task_key"] for name in DATASETS},
                f"background task {key} remained active outside the baseline observation contract",
            )
            _require(task_run_is_bounded_active(row), f"active background task {key} lacks a bounded progress snapshot")
            requested = parse_time(row.get("started_at"))
            observed = parse_time(row.get("observed_at"))
            progress_at = parse_time(row["progress"]["last_progress_at"])
            _require(requested <= started, f"active task {key} activity predates its requested run")
            _require(setup_start <= requested <= input_end, f"active task {key} was not requested during the foreground window")
            _require(started <= progress_at <= observed <= cleanup_end, f"active task {key} progress fell outside the measured trial")
        else:
            finished = parse_time(row.get("actual_finished_at"))
            _require(finished >= started, "task run has an inverted execution interval")
            _require(setup_start <= started <= finished <= cleanup_end, "task execution fell outside the measured trial")
            zero_contribution_defer = (
                key == COMPETITOR_TASK
                and task_run_is_accepted_zero_contribution_competitor_defer(row)
            )
            accepted = (
                zero_contribution_defer
                or task_run_is_accepted_competitor_chunk(row)
                if key == COMPETITOR_TASK
                else task_run_is_accepted_terminal(row)
                or (version == "baseline" and task_run_is_bounded_baseline_observation(row))
            )
            if (
                not accepted
                and key == COMPETITOR_TASK
                and isinstance(competitor_oracle, dict)
                and competitor_oracle.get("verified_empty") is True
                and competitor_oracle.get("remaining_count") == 0
                and competitor_oracle.get("remaining_bytes") == 0
                and task_run_is_recoverable_competitor_chunk(row)
            ):
                accepted = True
                recoverable_competitor_rows.append(row)
            _require(accepted, f"background task {key} did not finish successfully or commit valid work")
        work = row.get("committed_work")
        if key == COMPETITOR_TASK and task_run_is_accepted_zero_contribution_competitor_defer(row):
            _require(work is None, f"task {key} zero-contribution defer reported committed work")
        else:
            _require(isinstance(work, int) and not isinstance(work, bool) and work >= 0, f"task {key} committed-work count is invalid")
        by_key.setdefault(key, []).append(row)

    for row in recoverable_competitor_rows:
        finished = parse_time(row["actual_finished_at"])
        _require(
            any(
                candidate is not row
                and candidate.get("task_key") == COMPETITOR_TASK
                and parse_time(candidate.get("actual_started_at")) >= finished
                and task_run_is_accepted_competitor_chunk(candidate)
                for candidate in rows
            ),
            "recoverable competitor chunk has no later successful retry",
        )

    targets = [DATASETS[name]["task_key"] for name in DATASETS]
    for key in targets:
        runs = by_key.get(key, [])
        scheduled = [row for row in runs if row.get("trigger_kind") == "schedule" and row.get("actual_finished_at") is not None]
        catchups = [row for row in runs if row.get("trigger_kind") == "catchup" and row.get("actual_finished_at") is not None]
        active_scheduled = [row for row in runs if row.get("trigger_kind") == "schedule" and task_run_is_bounded_active(row)]
        active_catchups = [row for row in runs if row.get("trigger_kind") == "catchup" and task_run_is_bounded_active(row)]
        completed_chain = bool(scheduled and catchups and any(
            parse_time(schedule["actual_finished_at"]) <= parse_time(catchup["actual_started_at"])
            for schedule in scheduled
            for catchup in catchups
        ))
        bounded_chain = bool(active_scheduled or any(
            parse_time(scheduled_row["actual_finished_at"]) <= parse_time(active_row["observed_at"])
            for scheduled_row in scheduled
            for active_row in active_catchups
        ))
        if version == "baseline":
            _require(
                completed_chain or bounded_chain,
                f"production dispatcher did not complete both schedule and catch-up runs or provide a bounded baseline observation for {key}",
            )
        else:
            _require(completed_chain, f"production dispatcher did not complete both schedule and catch-up runs for {key}")
        _require(
            any(
                (row["committed_work"] > 0 and parse_time(row["actual_finished_at"]) <= input_end)
                if row.get("actual_finished_at") is not None
                else task_run_is_bounded_active(row)
                for row in runs
            ),
            f"production owner {key} made no progress during foreground input",
        )

    competitor = by_key.get(COMPETITOR_TASK, [])
    _require(competitor, "real background competitor evidence is missing")
    counted_competitor = [
        row for row in competitor
        if not task_run_is_accepted_zero_contribution_competitor_defer(row)
    ]
    competitor_work = sum(row["committed_work"] for row in counted_competitor)
    _require(competitor_work == COMPETITOR_FILE_COUNT, "background competitor did not complete the fixed 512-file workload")
    competitor_bytes = [row.get("committed_bytes") for row in counted_competitor]
    _require(
        len(competitor_bytes) == len(counted_competitor)
        and all(isinstance(value, int) and not isinstance(value, bool) and value >= 0 for value in competitor_bytes)
        and sum(competitor_bytes) == COMPETITOR_FILE_COUNT * COMPETITOR_FILE_BYTES,
        "background competitor released an unexpected byte count",
    )
    target_rows = [row for key in targets for row in by_key.get(key, [])]
    def interval_end(row: dict[str, Any]) -> datetime:
        return parse_time(row["actual_finished_at"] or row["observed_at"])

    def interval_start(row: dict[str, Any]) -> datetime:
        return parse_time(row.get("activity_started_at") or row["actual_started_at"])

    overlap = any(
        max(interval_start(left), interval_start(right))
        < min(interval_end(left), interval_end(right))
        for left in target_rows
        for right in counted_competitor
    )
    if require_competitor_overlap:
        _require(overlap, "candidate raw orphan-sweep did not overlap a retention owner run")
    return competitor_work


def validate_evidence(evidence: dict[str, Any]) -> dict[str, Any]:
    """Validate identity, complete windows, source-oracle results, and performance gates."""
    _require(evidence.get("schema_version") == SCHEMA_VERSION, "unsupported evidence schema")
    _require(evidence.get("status") == "completed", "measurement evidence is not complete")
    run_id = evidence.get("run_id")
    _require(isinstance(run_id, str) and re.fullmatch(r"[0-9a-f]{12}", run_id), "measurement run ID is invalid")
    phase = evidence.get("test_phase")
    _require(isinstance(phase, dict), "test-phase clock evidence is missing")
    phase_start = parse_time(phase.get("started_at"))
    phase_end = parse_time(phase.get("finished_at"))
    phase_elapsed = (phase_end - phase_start).total_seconds()
    _require(phase_end >= phase_start, "test-phase clock is inverted")
    _require(phase_elapsed <= MAX_TEST_PHASE_SECONDS, "test phase exceeded 900 seconds")
    _require(abs(_number(phase.get("elapsed_seconds"), "test-phase elapsed time is invalid") - phase_elapsed) <= 1.0, "test-phase elapsed time is not bound to its timestamps")

    contract = evidence.get("contract")
    _require(isinstance(contract, dict), "acceptance contract identity is missing")
    _require(contract.get("baseline_sha") == BASELINE_SHA, "baseline SHA drift")
    candidate_sha = contract.get("candidate_sha")
    _require(isinstance(candidate_sha, str) and len(candidate_sha) == 40 and all(c in "0123456789abcdef" for c in candidate_sha), "candidate SHA is invalid")
    _require(candidate_sha != BASELINE_SHA, "candidate SHA must differ from the baseline SHA")
    _require(contract.get("seed") == SEED, "synthetic seed drift")
    _require(contract.get("retention_days") == RETENTION_DAYS, "retention cutoff policy drift")
    _require(
        isinstance(contract.get("driver_image_id"), str)
        and re.fullmatch(r"sha256:[0-9a-f]{64}", contract["driver_image_id"]),
        "foreground driver image identity is invalid",
    )
    _require(contract.get("driver_image_ref") == DRIVER_IMAGE_REF, "foreground driver image reference drift")
    _require(contract.get("driver_image_digest") == DRIVER_IMAGE_DIGEST, "foreground driver image digest drift")
    _require(
        contract.get("input_rates_rows_per_second") == {key: value["arrival_rate"] for key, value in DATASETS.items()},
        "foreground arrival-rate contract drift",
    )
    input_fingerprint = contract.get("input_fingerprint")
    _require(
        isinstance(input_fingerprint, str)
        and len(input_fingerprint) == 64
        and all(c in "0123456789abcdef" for c in input_fingerprint),
        "input fingerprint is missing or malformed",
    )
    _require(isinstance(contract.get("cutoff"), str) and contract["cutoff"], "fixed retention cutoff is missing")
    _require(contract.get("seed_rows") == {key: value["seed_rows"] for key, value in DATASETS.items()}, "synthetic snapshot size drift")

    versions = evidence.get("versions")
    _require(isinstance(versions, dict) and set(versions) == {"baseline", "candidate"}, "baseline/candidate evidence is incomplete")
    _require(evidence.get("trial_execution_order") == TRIAL_EXECUTION_ORDER, "baseline/candidate trial execution order drift")
    comparison = evidence.get("comparison_window")
    _require(isinstance(comparison, dict), "full baseline/candidate comparison window is missing")
    comparison_start = parse_time(comparison.get("started_at"))
    comparison_end = parse_time(comparison.get("finished_at"))
    _require(comparison_start <= phase_start and comparison_end >= phase_end, "comparison window omitted test-phase preparation or cleanup")
    _require(
        abs(_number(comparison.get("elapsed_seconds"), "comparison duration is invalid") - (comparison_end - comparison_start).total_seconds()) <= 1.0,
        "full comparison duration is inconsistent",
    )
    host_identity = None
    resource_limits = None
    owner_tokens: set[str] = set()
    trial_windows: dict[tuple[str, int], tuple[datetime, datetime, dict[str, Any]]] = {}
    competitor_workloads: dict[str, list[int]] = {"baseline": [], "candidate": []}
    for version, expected_sha in (("baseline", BASELINE_SHA), ("candidate", candidate_sha)):
        group = versions[version]
        _require(isinstance(group, dict), f"{version} evidence is malformed")
        image = group.get("image")
        _require(isinstance(image, dict), f"{version} image identity is missing")
        _require(image.get("source_sha") == expected_sha, f"{version} image source SHA drift")
        image_id = image.get("image_id")
        _require(
            isinstance(image_id, str)
            and len(image_id) == 71
            and image_id.startswith("sha256:")
            and all(character in "0123456789abcdef" for character in image_id[7:]),
            f"{version} immutable image ID is missing or malformed",
        )
        host = group.get("host")
        _require(isinstance(host, dict) and host.get("runner_image"), f"{version} host identity is missing")
        limits = group.get("resource_limits")
        _require(
            isinstance(limits, dict)
            and limits.get("cpus") == 2
            and limits.get("memory_bytes") == 4_294_967_296,
            f"{version} resource limits drifted",
        )
        if host_identity is None:
            host_identity = host
            resource_limits = limits
        else:
            _require(host == host_identity, "baseline and candidate used different runner identities")
            _require(limits == resource_limits, "baseline and candidate resource limits differ")
        trials = group.get("trials")
        _require(isinstance(trials, list) and len(trials) == TRIALS_PER_VERSION, f"{version} must contain exactly three trials")
        trial_read_metrics: list[dict[str, float]] = []
        trial_write_metrics: list[dict[str, float]] = []
        for index, trial in enumerate(trials, start=1):
            _require(isinstance(trial, dict), f"{version} trial is malformed")
            _require(trial.get("trial_index") == index, f"{version} trial order is invalid")
            _require(trial.get("source_sha") == expected_sha, f"{version} trial SHA drift")
            _require(trial.get("image_id") == image["image_id"], f"{version} trial image drift")
            _require(trial.get("input_fingerprint") == input_fingerprint, f"{version} input fingerprint drift")
            _require(trial.get("cutoff") == contract["cutoff"], f"{version} cutoff drift")
            application_cleanup = trial.get("application_container_cleanup")
            _require(
                isinstance(application_cleanup, dict)
                and application_cleanup.get("container_name") == f"retention-{run_id}-{version}-{index}"
                and isinstance(application_cleanup.get("container_id"), str)
                and re.fullmatch(r"[0-9a-f]{64}", application_cleanup["container_id"])
                and application_cleanup.get("container_id") == trial.get("container_id")
                and application_cleanup.get("status") in {"already_absent", "removed"},
                "named application container cleanup is incomplete",
            )
            _validate_container_identity_labels(
                application_cleanup, run_id, "application", version, index, owner_tokens
            )
            driver_cleanup = trial.get("foreground_driver_cleanup")
            _require(
                isinstance(driver_cleanup, dict)
                and driver_cleanup.get("container_name") == f"retention-{run_id}-{version}-{index}-driver"
                and isinstance(driver_cleanup.get("container_id"), str)
                and re.fullmatch(r"[0-9a-f]{64}", driver_cleanup["container_id"])
                and driver_cleanup.get("status") in {"already_absent", "removed"},
                "named foreground driver cleanup is incomplete or belongs to another trial",
            )
            _validate_container_identity_labels(
                driver_cleanup, run_id, "foreground", version, index, owner_tokens
            )

            window = trial.get("window")
            _require(isinstance(window, dict), "trial window evidence is missing")
            setup_start = parse_time(window.get("setup_started_at"))
            input_start = parse_time(window.get("input_started_at"))
            input_end = parse_time(window.get("input_finished_at"))
            cleanup_end = parse_time(window.get("cleanup_finished_at"))
            _require(setup_start <= input_start <= input_end <= cleanup_end, "trial window boundaries are inconsistent")
            _require(
                phase_start <= setup_start and cleanup_end <= phase_end,
                "trial window fell outside the test phase",
            )
            input_elapsed = (input_end - input_start).total_seconds()
            _require(abs(input_elapsed - RUN_SECONDS) <= 1.0, "foreground input window is not 120 seconds")
            _require(abs(_number(window.get("input_duration_seconds"), "input duration is invalid") - input_elapsed) <= 1.0, "recorded input duration is inconsistent")
            full_elapsed = (cleanup_end - setup_start).total_seconds()
            _require(abs(_number(window.get("full_duration_seconds"), "trial duration is invalid") - full_elapsed) <= 1.0, "trial full window omitted setup or cleanup")
            trial_windows[(version, index)] = (setup_start, cleanup_end, window)

            input_counts = trial.get("input_counts")
            added_ids = trial.get("added_ids")
            _require(isinstance(input_counts, dict) and isinstance(added_ids, dict), "foreground input count or identity evidence is missing")
            for name, definition in DATASETS.items():
                expected_count = math.ceil(definition["arrival_rate"] * RUN_SECONDS)
                identities = added_ids.get(name)
                _require(input_counts.get(name) == expected_count, f"{name} foreground arrival rate drifted")
                _require(
                    isinstance(identities, list)
                    and len(identities) == expected_count
                    and all(isinstance(identity, str) and identity for identity in identities)
                    and len(set(identities)) == expected_count,
                    f"{name} foreground source identities are incomplete",
                )
            _validate_samples(trial, input_start, input_end)
            attempt_ids = added_ids["pool_upstream_attempts"]
            attempt_states = trial.get("attempt_states")
            _require(
                isinstance(attempt_states, list) and len(attempt_states) == len(attempt_ids),
                "terminal upstream attempt-state evidence is incomplete",
            )
            expected_invocations = added_ids["invocations"]
            retry_count = len(attempt_ids) - len(expected_invocations)
            retry_sequences = {
                math.floor(index * len(expected_invocations) / retry_count)
                for index in range(retry_count)
            } if retry_count else set()
            expected_statuses = {
                invoke_id: (["http_failure", "success"] if sequence in retry_sequences else ["success"])
                for sequence, invoke_id in enumerate(expected_invocations)
            }
            observed_by_invocation: dict[str, list[dict[str, Any]]] = {}
            observed_attempt_ids: set[str] = set()
            for attempt in attempt_states:
                _require(isinstance(attempt, dict), "upstream attempt-state row is malformed")
                attempt_id = attempt.get("attempt_public_id")
                invoke_id = attempt.get("invoke_id")
                status = attempt.get("status")
                finished_at = parse_time(attempt.get("finished_at"))
                attempt_index = attempt.get("attempt_index")
                _require(isinstance(attempt_id, str) and attempt_id and attempt_id in attempt_ids, "upstream attempt-state identity is unexpected")
                _require(attempt_id not in observed_attempt_ids, "upstream attempt-state identity is duplicated")
                observed_attempt_ids.add(attempt_id)
                _require(isinstance(invoke_id, str) and invoke_id in expected_statuses, "upstream attempt-state invocation is unexpected")
                _require(isinstance(status, str) and status not in {"", "pending", "running", "budget_exhausted_final", "interrupted"}, "upstream attempt did not reach a terminal state")
                _require(isinstance(attempt_index, int) and not isinstance(attempt_index, bool) and attempt_index > 0, "upstream attempt index is invalid")
                _require(setup_start <= finished_at <= cleanup_end, "upstream attempt finished outside the trial window")
                observed_by_invocation.setdefault(invoke_id, []).append(attempt)
            _require(observed_attempt_ids == set(attempt_ids), "upstream attempt-state IDs do not match measured source identities")
            for invoke_id, expected in expected_statuses.items():
                actual = sorted(observed_by_invocation.get(invoke_id, []), key=lambda row: row["attempt_index"])
                _require(
                    [row.get("status") for row in actual] == expected
                    and [row.get("attempt_index") for row in actual] == list(range(1, len(expected) + 1)),
                    f"upstream attempt states for {invoke_id} are incomplete or non-terminal",
                )
            invocation_count = math.ceil(DATASETS["invocations"]["arrival_rate"] * RUN_SECONDS)
            attempt_count = math.ceil(DATASETS["pool_upstream_attempts"]["arrival_rate"] * RUN_SECONDS)
            expected_retries = attempt_count - invocation_count
            workload = trial.get("foreground_workload")
            _require(isinstance(workload, dict), "foreground HTTP workload evidence is missing")
            upstream = workload.get("mock_upstream")
            _require(isinstance(upstream, dict), "mock upstream request evidence is missing")
            _require(workload.get("invocation_requests") == invocation_count, "foreground proxy request count drifted")
            _require(workload.get("pool_read_requests", 0) >= 20, "foreground pool read workload is incomplete")
            _require(upstream.get("request_count") == attempt_count, "mock upstream attempt count does not match the measured source rows")
            _require(upstream.get("retryable_failures") == expected_retries, "mock upstream retry workload drifted")
            _require(upstream.get("successful_responses") == invocation_count, "mock upstream success count does not match proxy requests")
            raw_samples = trial["latency_samples_ms"]
            _require(len(raw_samples["write"]) == invocation_count, "foreground response latency sample count does not match HTTP writes")
            _require(len(raw_samples["read"]) == workload["pool_read_requests"], "foreground read latency sample count is inconsistent")
            competitor_oracle = trial.get("competitor_file_oracle")
            _require(
                isinstance(competitor_oracle, dict)
                and competitor_oracle.get("expected_count") == COMPETITOR_FILE_COUNT
                and competitor_oracle.get("expected_bytes") == COMPETITOR_FILE_COUNT * COMPETITOR_FILE_BYTES
                and competitor_oracle.get("remaining_count") == 0
                and competitor_oracle.get("remaining_bytes") == 0
                and competitor_oracle.get("verified_empty") is True,
                "fixed competitor file oracle is missing or not empty",
            )
            competitor_workloads[version].append(
                _validate_task_runs(
                    trial,
                    setup_start,
                    input_end,
                    cleanup_end,
                    require_competitor_overlap=version == "candidate",
                    version=version,
                    competitor_oracle=competitor_oracle,
                )
            )
            scrape_rows = trial.get("write_hold_samples")
            _require(isinstance(scrape_rows, list) and len(scrape_rows) >= 2, "raw production write-hold histogram evidence is missing")
            for row in scrape_rows:
                _require(setup_start <= parse_time(row.get("observed_at")) <= cleanup_end, "write-hold sample fell outside the full trial window")
                _require(isinstance(row.get("bucket_counts"), dict), "raw write-hold scrape omitted bucket counts")
            _require(scrape_rows[0].get("scope") == "pre-input" and scrape_rows[-1].get("scope") == "after-drain", "write-hold scrape boundaries are incomplete")
            before = scrape_rows[0]["bucket_counts"]
            after = scrape_rows[-1]["bucket_counts"]
            derived_buckets: dict[str, int] = {}
            for operation in ("invocation_archive", "timestamped_archive"):
                old_buckets = before.get(operation)
                new_buckets = after.get(operation)
                _require(isinstance(old_buckets, dict) and isinstance(new_buckets, dict) and new_buckets, f"raw {operation} histogram scrape is missing")
                _require("+Inf" in new_buckets and set(old_buckets).issubset(new_buckets), "write-hold histogram bucket schema changed during a trial")
                for upper, count in new_buckets.items():
                    previous = old_buckets.get(upper, 0)
                    _require(
                        isinstance(count, int)
                        and not isinstance(count, bool)
                        and isinstance(previous, int)
                        and not isinstance(previous, bool)
                        and count >= previous,
                        "write-hold histogram counter reset or decreased",
                    )
                    derived_buckets[upper] = derived_buckets.get(upper, 0) + count - previous
            hold_buckets = trial.get("write_hold_buckets")
            _require(isinstance(hold_buckets, dict), "production write-hold histogram is missing")
            _require(hold_buckets == derived_buckets, "reported write-hold histogram is not derived from raw scrape deltas")
            hold_p95 = histogram_quantile_ms(hold_buckets, 0.95)
            trial["write_hold_p95_ms"] = hold_p95
            if version == "candidate":
                _require(hold_p95 <= MAX_WRITE_HOLD_P95_MS, "steady write-hold p95 exceeded 250ms")

            proof_status = trial.get("archive_proof_status")
            _require(isinstance(proof_status, dict), "archive proof status evidence is missing")
            proof_datasets = proof_status.get("datasets")
            _require(
                isinstance(proof_datasets, dict) and set(proof_datasets) == set(DATASETS),
                "archive proof status dataset evidence is incomplete",
            )
            _require(proof_status.get("reference_ready") is True, "archive reference evidence is incomplete")
            for name in DATASETS:
                dataset_status = proof_datasets[name]
                _require(isinstance(dataset_status, dict), f"archive proof status for {name} is malformed")
                _require(
                    dataset_status.get("dataset") == DATASETS[name]["manifest_dataset"],
                    f"archive proof status dataset mismatch for {name}",
                )
                _require(
                    dataset_status.get("reference_ready") is True,
                    f"archive reference evidence for {name} is incomplete",
                )
                _require(
                    isinstance(dataset_status.get("summary_proof_complete"), bool),
                    f"archive summary proof status for {name} is malformed",
                )
                if version == "candidate":
                    _require(
                        dataset_status.get("ready") is True,
                        f"candidate archive proof status for {name} is incomplete",
                    )
                    _require(
                        dataset_status.get("summary_proof_complete") is True,
                        f"candidate summary proof for {name} is incomplete",
                    )
                    _require(
                        dataset_status.get("summary_proof_missing") == [],
                        f"candidate summary proof for {name} has missing obligations",
                    )

            oracle = trial.get("oracle")
            _require(isinstance(oracle, dict) and oracle.get("verified") is True, "source/artifact oracle evidence is missing")
            datasets = oracle.get("datasets")
            _require(isinstance(datasets, dict) and set(datasets) == set(DATASETS), "oracle dataset evidence is incomplete")
            for name in DATASETS:
                result = datasets[name]
                _require(isinstance(result, dict), f"oracle result for {name} is malformed")
                _require(result.get("dataset") == DATASETS[name]["manifest_dataset"], f"oracle dataset mismatch for {name}")
                _require(
                    result.get("summary_proof_required") is (version == "candidate"),
                    f"oracle summary proof requirement for {name} is not bound to the version",
                )
                _require(
                    isinstance(result.get("summary_proof_verified"), bool),
                    f"oracle summary proof result for {name} is malformed",
                )
                if version == "candidate":
                    _require(
                        result.get("summary_proof_verified") is True,
                        f"candidate summary proof for {name} was not verified",
                    )
                _require(result.get("seed_rows") == DATASETS[name]["seed_rows"], f"oracle seed population mismatch for {name}")
                _require(result.get("actual_added_rows") == trial["input_counts"][name], f"oracle {name} addition count is not bound to measured writes")
                for field in ("actual_added_rows", "converted_rows", "distinct_archived_rows", "live_rows", "lost_rows", "duplicate_rows"):
                    value = result.get(field)
                    _require(isinstance(value, int) and not isinstance(value, bool) and value >= 0, f"oracle {field} is invalid")
                _require(result["lost_rows"] == 0, f"oracle found lost {name} source rows")
                _require(result["duplicate_rows"] == 0, f"oracle found duplicate {name} conversions")
                _require(result["converted_rows"] == result["distinct_archived_rows"], f"reported {name} conversion count is not proof-backed")
                _require(
                    result.get("row_content_rows_checked") == result["seed_rows"] + result["actual_added_rows"],
                    f"oracle {name} did not compare full source row content",
                )
                _require(
                    isinstance(result.get("row_content_columns"), int)
                    and not isinstance(result["row_content_columns"], bool)
                    and result["row_content_columns"] >= 3,
                    f"oracle {name} did not cover all source columns",
                )
                _require(
                    isinstance(result.get("row_content_sha256"), str)
                    and re.fullmatch(r"[0-9a-f]{64}", result["row_content_sha256"]),
                    f"oracle {name} full-row content digest is invalid",
                )
                identities = trial["added_ids"][name]
                _require(
                    result.get("added_ids_sha256") == sha256_bytes("\n".join(identities).encode()),
                    f"oracle {name} input identity digest does not match observed writes",
                )
                if name == "pool_upstream_attempts":
                    mapping_digest = sha256_json(sorted(
                        [row["attempt_public_id"], row["invoke_id"]]
                        for row in attempt_states
                    ))
                    _require(
                        result.get("attempt_invocation_map_sha256") == mapping_digest,
                        "oracle upstream attempt-to-invocation association does not match observed attempts",
                    )
                _require(result["seed_rows"] + result["actual_added_rows"] == result["live_rows"] + result["converted_rows"], f"{name} source conservation does not balance")
                if version == "candidate":
                    additions = result["actual_added_rows"]
                    _require(additions > 0, f"candidate added no {name} rows")
                    input_duration = _number(
                        trial["window"].get("input_duration_seconds"),
                        f"candidate {name} input window is invalid",
                    )
                    full_duration = _number(
                        trial["window"].get("full_duration_seconds"),
                        f"candidate {name} sustained window is invalid",
                    )
                    _require(
                        input_duration > 0 and full_duration >= input_duration,
                        f"candidate {name} sustained window is invalid",
                    )
                    factor = (result["converted_rows"] / full_duration) / (additions / input_duration)
                    reported_factor = _number(
                        result.get("sustained_convergence_factor"),
                        f"candidate {name} sustained convergence factor is missing",
                    )
                    _require(
                        abs(reported_factor - factor) <= 0.01,
                        f"reported candidate {name} sustained convergence factor is not derived from the trial window",
                    )
                    _require(factor >= MIN_CONVERGENCE_FACTOR, f"candidate {name} convergence factor is below 10x")
                    result["convergence_factor"] = factor

            trial_read_metrics.append(latency_summary([float(item["elapsed_ms"]) for item in raw_samples["read"]]))
            trial_write_metrics.append(latency_summary([float(item["elapsed_ms"]) for item in raw_samples["write"]]))

        trial_span = group.get("trial_span")
        _require(isinstance(trial_span, dict), "version trial span is missing")
        group_start = parse_time(trial_span.get("started_at"))
        group_end = parse_time(trial_span.get("finished_at"))
        _require(
            phase_start <= group_start <= group_end <= phase_end,
            f"{version} trial span fell outside the test phase",
        )
        _require(group_start <= parse_time(trials[0]["window"]["setup_started_at"]), "version span omitted initial preparation")
        _require(group_end >= parse_time(trials[-1]["window"]["cleanup_finished_at"]), "version span omitted final cleanup")
        _require(abs(_number(trial_span.get("elapsed_seconds"), "version span duration is invalid") - (group_end - group_start).total_seconds()) <= 1.0, "version span duration is inconsistent")
        group["latency_summary"] = {
            "read": {metric: median(row[metric] for row in trial_read_metrics) for metric in ("p95_ms", "p99_ms")},
            "write": {metric: median(row[metric] for row in trial_write_metrics) for metric in ("p95_ms", "p99_ms")},
        }

    previous_end: datetime | None = None
    for step in TRIAL_EXECUTION_ORDER:
        setup_start, cleanup_end, window = trial_windows[(step["version"], step["trial_index"])]
        gap_before = 0.0 if previous_end is None else (setup_start - previous_end).total_seconds()
        _require(gap_before >= 0, "trial windows overlap or run order changed")
        _require(
            abs(_number(window.get("gap_before_seconds"), "inter-run gap is invalid") - gap_before) <= 1.0,
            "inter-run gap was omitted or altered",
        )
        previous_end = cleanup_end

    _require(
        competitor_workloads["baseline"] == competitor_workloads["candidate"]
        == [COMPETITOR_FILE_COUNT] * TRIALS_PER_VERSION,
        "baseline and candidate did not receive the same fixed competitor workload",
    )

    baseline = versions["baseline"]["latency_summary"]
    candidate = versions["candidate"]["latency_summary"]
    for operation in ("read", "write"):
        for metric in ("p95_ms", "p99_ms"):
            _require(
                candidate[operation][metric] <= baseline[operation][metric],
                f"candidate foreground {operation} {metric} median regressed against baseline",
            )
    return {
        "status": "passed",
        "baseline_sha": BASELINE_SHA,
        "candidate_sha": candidate_sha,
        "test_phase_seconds": phase_elapsed,
        "latency_summary": {
            "baseline": baseline,
            "candidate": candidate,
        },
    }
