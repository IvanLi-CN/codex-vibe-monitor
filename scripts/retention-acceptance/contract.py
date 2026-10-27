"""Shared, fail-closed contract for the isolated retention acceptance run."""

from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import json
import math
import re
from statistics import median
from typing import Any


BASELINE_SHA = "4d46276dc5f8a6fca7db8cfb9cff98a03bd0ba9f"
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
# The fixed baseline is known to duplicate-publish these two source datasets. Keep this
# allowlist explicit so adding a future dataset cannot silently become a tolerated defect.
BASELINE_NEGATIVE_CONTROL_DATASETS = frozenset({
    "codex_invocations",
    "pool_upstream_request_attempts",
})
COMPETITOR_TASK = "raw_orphan_sweep"
SUMMARY_RECOVERY_TASK = "summary_coverage_recovery"
COMPETITOR_FILE_COUNT = 512
COMPETITOR_FILE_BYTES = 256
RUN_SECONDS = 120
TRIALS_PER_VERSION = 3
MAX_TEST_PHASE_SECONDS = 900
CLEANUP_RESERVE_SECONDS = 45
MIN_CONVERGENCE_FACTOR = 10.0
MAX_WRITE_HOLD_P95_MS = 250.0
SEED = 20261024
RETENTION_DAYS = 1
SCHEMA_VERSION = 9
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


BASELINE_NEGATIVE_CONTROL_FAILURE = "source_artifact_conservation"


def baseline_negative_control_oracle(error: BaseException) -> dict[str, Any] | None:
    """Classify one known baseline duplicate-publication defect without weakening candidate gates."""
    message = str(error)
    match = re.fullmatch(
        r"(?P<dataset>[a-z_]+) source/artifact conservation failed: "
        r"lost=(?P<lost>[0-9]+), duplicates=(?P<duplicates>[0-9]+)",
        message,
    )
    if match is None:
        return None
    dataset = match.group("dataset")
    lost_rows = int(match.group("lost"))
    duplicate_rows = int(match.group("duplicates"))
    if dataset not in BASELINE_NEGATIVE_CONTROL_DATASETS or lost_rows != 0 or duplicate_rows <= 0:
        return None
    return {
        "verified": False,
        "failure_class": BASELINE_NEGATIVE_CONTROL_FAILURE,
        "negative_controls": [{
            "dataset": dataset,
            "lost_rows": lost_rows,
            "duplicate_rows": duplicate_rows,
            "error": message,
        }],
    }


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
        raise AcceptanceError("timestamp must include a UTC offset")
    return parsed.astimezone(timezone.utc)


def task_run_is_accepted_terminal(row: dict[str, Any]) -> bool:
    """Accept legacy partial status only when the selected batch was fully committed."""
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
        or committed <= 0
        or committed > selected
    ):
        return False
    if committed != selected and scope.get("hasMore") is not True:
        return False
    batches = details.get("archiveBatches")
    if not isinstance(batches, list) or not batches:
        return False
    return all(
        isinstance(batch, dict)
        and isinstance(batch.get("batchRows"), int)
        and isinstance(batch.get("committedRows"), int)
        and batch.get("committedRows") > 0
        and batch.get("committedRows") <= batch.get("batchRows")
        and batch.get("timeoutCount", 0) == 0
        for batch in batches
    )


def task_run_is_accepted_competitor_chunk(row: dict[str, Any]) -> bool:
    """Accept bounded chunks while keeping fixed-file completion as the oracle."""
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
    """Accept a legal file-admission defer without counting work that did not run."""
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
    admission_pair = (details.get("admissionStage"), details.get("waitReason"))
    if admission_pair not in {
        ("file_phase", "file_phase_busy"),
        ("archive_file_phase", "archive_directory_lock_busy"),
    }:
        return False
    required_null_fields = (
        "filesChecked",
        "directoryEntriesChecked",
        "reconciliationRowsChecked",
        "filesReleased",
        "bytesReleased",
        "referencedSkipped",
        "quarantined",
        "failures",
    )
    return all(
        field in details and details[field] is None
        for field in required_null_fields
    )


def task_run_is_recoverable_competitor_chunk(row: dict[str, Any]) -> bool:
    """Recognize a progressed chunk whose transient failure was retried later."""
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
        for sample in values:
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


def _validate_task_runs(
    trial: dict[str, Any],
    setup_start: datetime,
    cleanup_end: datetime,
    require_competitor_overlap: bool,
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
        started = parse_time(row.get("actual_started_at"))
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
        if zero_contribution_defer:
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
                and parse_time(candidate["actual_started_at"]) >= finished
                and task_run_is_accepted_competitor_chunk(candidate)
                for candidate in rows
            ),
            "recoverable competitor chunk has no later successful retry",
        )

    targets = [DATASETS[name]["task_key"] for name in DATASETS]
    for key in targets:
        runs = by_key.get(key, [])
        scheduled = [row for row in runs if row.get("trigger_kind") == "schedule"]
        catchups = [row for row in runs if row.get("trigger_kind") == "catchup"]
        _require(scheduled and catchups, f"production dispatcher did not complete both schedule and catch-up runs for {key}")
        _require(
            any(parse_time(schedule["actual_finished_at"]) <= parse_time(catchup["actual_started_at"]) for schedule in scheduled for catchup in catchups),
            f"production catch-up did not follow a scheduled run for {key}",
        )

    competitor = by_key.get(COMPETITOR_TASK, [])
    _require(competitor, "real background competitor evidence is missing")
    counted_competitor = [
        row
        for row in competitor
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
    productive_competitor = [row for row in counted_competitor if row["committed_work"] > 0]
    overlap = any(
        max(parse_time(left["actual_started_at"]), parse_time(right["actual_started_at"]))
        < min(parse_time(left["actual_finished_at"]), parse_time(right["actual_finished_at"]))
        for left in target_rows
        for right in productive_competitor
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
            input_elapsed = (input_end - input_start).total_seconds()
            _require(abs(input_elapsed - RUN_SECONDS) <= 1.0, "foreground input window is not 120 seconds")
            _require(abs(_number(window.get("input_duration_seconds"), "input duration is invalid") - input_elapsed) <= 1.0, "recorded input duration is inconsistent")
            full_elapsed = (cleanup_end - setup_start).total_seconds()
            _require(abs(_number(window.get("full_duration_seconds"), "trial duration is invalid") - full_elapsed) <= 1.0, "trial full window omitted setup or cleanup")
            trial_windows[(version, index)] = (setup_start, cleanup_end, window)

            _validate_samples(trial, input_start, input_end)
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
                    cleanup_end,
                    require_competitor_overlap=version == "candidate",
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
            for name in DATASETS:
                dataset_status = proof_datasets[name]
                _require(isinstance(dataset_status, dict), f"archive proof status for {name} is malformed")
                _require(dataset_status.get("dataset") == DATASETS[name]["manifest_dataset"], f"archive proof status dataset mismatch for {name}")
                _require(dataset_status.get("reference_ready") is True, f"archive reference evidence for {name} is incomplete")
                _require(isinstance(dataset_status.get("summary_proof_complete"), bool), f"archive summary proof status for {name} is malformed")
                if version == "candidate":
                    _require(dataset_status.get("ready") is True, f"candidate archive proof status for {name} is incomplete")
                    _require(dataset_status.get("summary_proof_complete") is True, f"candidate summary proof for {name} is incomplete")
                    _require(dataset_status.get("summary_proof_missing") == [], f"candidate summary proof for {name} has missing obligations")

            oracle = trial.get("oracle")
            _require(isinstance(oracle, dict), "source/artifact oracle evidence is missing")
            negative_dataset_names: set[str] = set()
            if version == "baseline" and oracle.get("verified") is False:
                _require(
                    oracle.get("failure_class") == BASELINE_NEGATIVE_CONTROL_FAILURE,
                    "baseline negative-control failure class is invalid",
                )
                controls = oracle.get("negative_controls")
                _require(
                    isinstance(controls, list) and controls,
                    "baseline negative-control evidence is missing",
                )
                for control in controls:
                    _require(isinstance(control, dict), "baseline negative-control entry is malformed")
                    dataset = control.get("dataset")
                    _require(
                        dataset in BASELINE_NEGATIVE_CONTROL_DATASETS,
                        "baseline negative-control dataset is invalid",
                    )
                    lost_rows = control.get("lost_rows")
                    duplicate_rows = control.get("duplicate_rows")
                    _require(
                        lost_rows == 0
                        and isinstance(duplicate_rows, int)
                        and not isinstance(duplicate_rows, bool)
                        and duplicate_rows > 0,
                        "baseline negative-control counts are invalid",
                    )
                    _require(
                        control.get("error")
                        == f"{dataset} source/artifact conservation failed: "
                        f"lost=0, duplicates={duplicate_rows}",
                        "baseline negative-control error is not bound to its counts",
                    )
                    negative_dataset_name = next(
                        name
                        for name, definition in DATASETS.items()
                        if definition["manifest_dataset"] == dataset
                    )
                    _require(
                        negative_dataset_name not in negative_dataset_names,
                        "baseline negative-control dataset is repeated",
                    )
                    negative_dataset_names.add(negative_dataset_name)
            else:
                _require(oracle.get("verified") is True, "source/artifact oracle evidence is missing")
            datasets = oracle.get("datasets")
            expected_dataset_names = set(DATASETS) - negative_dataset_names
            _require(
                isinstance(datasets, dict) and set(datasets) == expected_dataset_names,
                "oracle dataset evidence is incomplete",
            )
            for name in DATASETS:
                if name in negative_dataset_names:
                    continue
                result = datasets[name]
                _require(isinstance(result, dict), f"oracle result for {name} is malformed")
                _require(result.get("dataset") == DATASETS[name]["manifest_dataset"], f"oracle dataset mismatch for {name}")
                _require(result.get("summary_proof_required") is (version == "candidate"), f"oracle summary proof requirement for {name} is not bound to the version")
                _require(isinstance(result.get("summary_proof_verified"), bool), f"oracle summary proof result for {name} is malformed")
                if version == "candidate":
                    _require(result.get("summary_proof_verified") is True, f"candidate summary proof for {name} was not verified")
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
                    factor = result["converted_rows"] / additions
                    _require(factor >= MIN_CONVERGENCE_FACTOR, f"candidate {name} convergence factor is below 10x")
                    result["convergence_factor"] = factor

            trial_read_metrics.append(latency_summary([float(item["elapsed_ms"]) for item in raw_samples["read"]]))
            trial_write_metrics.append(latency_summary([float(item["elapsed_ms"]) for item in raw_samples["write"]]))

        trial_span = group.get("trial_span")
        _require(isinstance(trial_span, dict), "version trial span is missing")
        group_start = parse_time(trial_span.get("started_at"))
        group_end = parse_time(trial_span.get("finished_at"))
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
