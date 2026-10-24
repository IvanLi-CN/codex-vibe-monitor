"""Run the isolated, time-bounded baseline/candidate retention comparison."""

from __future__ import annotations

from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import uuid
from typing import Any

from contract import (
    BASELINE_SHA,
    DATASETS,
    MAX_TEST_PHASE_SECONDS,
    RUN_SECONDS,
    SCHEMA_VERSION,
    SEED,
    AcceptanceError,
    parse_time,
    sha256_bytes,
    validate_evidence,
)
from fixture import add_foreground_record, populate_template, prepare_competitor, shanghai_cutoff
from oracle import verify_trial


CPU_LIMIT = 2
MEMORY_BYTES = 4_294_967_296
CONTAINER_DATA = "/srv/app/data"
RAW_CONTAINER = f"{CONTAINER_DATA}/proxy_raw_payloads"
METRICS_NAME = "cvm_retention_phase_duration_seconds_bucket"
WRITE_HOLD_OPERATIONS = ("invocation_archive", "timestamped_archive")
WRITE_HOLD_PHASE = "write_hold"
RUN_ID = uuid.uuid4().hex[:12]


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def run_command(command: list[str], timeout: float, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, capture_output=True, text=True, timeout=timeout, check=False)
    if check and result.returncode != 0:
        message = result.stderr.strip()[-3000:] or result.stdout.strip()[-3000:]
        raise RuntimeError(f"command failed ({result.returncode}): {command[0]}: {message}")
    return result


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_bytes(json.dumps(payload, indent=2, sort_keys=True).encode() + b"\n")
    temporary.replace(path)


def remaining_budget(started: float) -> float:
    left = MAX_TEST_PHASE_SECONDS - (time.monotonic() - started)
    if left <= 0:
        raise TimeoutError("retention acceptance exceeded its 900-second test-phase budget")
    return left


def docker_image_identity(tag: str, expected_sha: str) -> dict[str, str]:
    image_id = run_command(["docker", "image", "inspect", "--format", "{{.Id}}", tag], 20).stdout.strip()
    labels = json.loads(run_command(["docker", "image", "inspect", "--format", "{{json .Config.Labels}}", tag], 20).stdout)
    if labels.get("org.opencontainers.image.revision") != expected_sha:
        raise AcceptanceError(f"loaded image {tag} does not identify source {expected_sha}")
    if not image_id.startswith("sha256:"):
        raise AcceptanceError(f"loaded image {tag} has no immutable Docker image ID")
    return {"source_sha": expected_sha, "image_id": image_id}


def host_identity() -> dict[str, Any]:
    cpu_model = "unknown"
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.lower().startswith("model name"):
                cpu_model = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    memory_kib = None
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal:"):
                memory_kib = int(line.split()[1])
                break
    except (OSError, ValueError):
        pass
    return {
        "runner_image": "ubuntu-24.04",
        "runner_os": platform.platform(),
        "architecture": platform.machine(),
        "cpu_model": cpu_model,
        "cpu_count": os.cpu_count(),
        "memory_kib": memory_kib,
        "python": platform.python_version(),
        "docker_server": run_command(["docker", "version", "--format", "{{.Server.Version}}"], 20).stdout.strip(),
    }


def app_environment() -> dict[str, str]:
    return {
        "DATABASE_PATH": f"{CONTAINER_DATA}/codex_vibe_monitor.db",
        "MAINTENANCE_DATABASE_PATH": f"{CONTAINER_DATA}/codex_vibe_monitor.maintenance.sqlite",
        "ARCHIVE_DIR": f"{CONTAINER_DATA}/archives",
        "PROXY_RAW_DIR": RAW_CONTAINER,
        "INVOCATION_MAX_DAYS": "1",
        "POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS": "1",
        "HTTP_BIND": "0.0.0.0:8080",
        "METRICS_BIND": "127.0.0.1:9091",
        "RUST_LOG": "warn",
    }


def start_container(tag: str, data_dir: Path, name: str) -> str:
    data_dir.mkdir(parents=True, exist_ok=True)
    command = [
        "docker", "run", "--detach", "--name", name,
        "--cpus", str(CPU_LIMIT),
        "--memory", str(MEMORY_BYTES),
        "--memory-swap", str(MEMORY_BYTES),
        "--network", "none",
        "--mount", f"type=bind,source={data_dir.resolve()},target={CONTAINER_DATA}",
    ]
    for key, value in app_environment().items():
        command.extend(["--env", f"{key}={value}"])
    command.append(tag)
    return run_command(command, 60).stdout.strip()


def wait_healthy(container: str, started: float) -> None:
    deadline = time.monotonic() + min(60, remaining_budget(started))
    while time.monotonic() < deadline:
        check = run_command(["docker", "exec", container, "curl", "-fsS", "-m", "1", "http://127.0.0.1:8080/health"], 5, check=False)
        if check.returncode == 0 and "ok" in check.stdout.lower():
            return
        time.sleep(0.3)
    raise TimeoutError(f"production image {container} failed its health check")


def stop_container(container: str, log_path: Path | None = None) -> None:
    logs = run_command(["docker", "logs", container], 15, check=False)
    if log_path is not None:
        log_path.parent.mkdir(parents=True, exist_ok=True)
        log_path.write_text(logs.stdout + logs.stderr)
    run_command(["docker", "stop", "--time", "5", container], 10, check=False)
    run_command(["docker", "rm", "--force", container], 10, check=False)


def checkpoint_and_reset_history(main_db: Path, maintenance_db: Path) -> None:
    main = sqlite3.connect(main_db, timeout=30)
    try:
        main.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    finally:
        main.close()
    maintenance = sqlite3.connect(maintenance_db, timeout=30)
    try:
        maintenance.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        names = {row[0] for row in maintenance.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        if "managed_task_work_runs" in names:
            maintenance.execute("DELETE FROM managed_task_work_runs")
        if "managed_task_runs" in names:
            maintenance.execute("DELETE FROM managed_task_runs")
        maintenance.commit()
        maintenance.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    finally:
        maintenance.close()


def prepare_template(tag: str, version: str, root: Path, cutoff: str, started: float) -> tuple[Path, str]:
    directory = root / f"template-{version}"
    directory.mkdir(parents=True)
    container = f"retention-{RUN_ID}-init-{version}"
    try:
        start_container(tag, directory, container)
        wait_healthy(container, started)
    finally:
        stop_container(container, root / "logs" / f"init-{version}.log")
    main_db = directory / "codex_vibe_monitor.db"
    maintenance_db = directory / "codex_vibe_monitor.maintenance.sqlite"
    if not main_db.is_file() or not maintenance_db.is_file():
        raise AcceptanceError(f"{version} production startup did not create both SQLite databases")
    fingerprint = populate_template(main_db, cutoff)
    checkpoint_and_reset_history(main_db, maintenance_db)
    return directory, fingerprint


def parse_metrics(text: str) -> dict[str, dict[str, int]]:
    result: dict[str, dict[str, int]] = {operation: {} for operation in WRITE_HOLD_OPERATIONS}
    for line in text.splitlines():
        if not line.startswith(METRICS_NAME + "{"):
            continue
        labels_text, raw_count = line.split("}", 1)
        labels: dict[str, str] = {}
        for key, value in re.findall(r'([a-zA-Z_][a-zA-Z0-9_]*)="((?:\\.|[^"\\])*)"', labels_text[1:]):
            labels[key] = bytes(value, "utf-8").decode("unicode_escape")
        operation = labels.get("operation")
        if operation not in result or labels.get("phase") != WRITE_HOLD_PHASE:
            continue
        upper = labels.get("le")
        if upper is None:
            raise AcceptanceError("write-hold metric bucket omitted its upper bound")
        try:
            count = int(raw_count.strip().split()[0])
        except (IndexError, ValueError) as error:
            raise AcceptanceError("write-hold metric bucket has an invalid count") from error
        result[operation][upper] = count
    return result


def scrape_metrics(container: str) -> tuple[dict[str, dict[str, int]], str, str]:
    result = run_command(["docker", "exec", container, "curl", "-fsS", "-m", "5", "http://127.0.0.1:9091/metrics"], 10)
    return parse_metrics(result.stdout), result.stdout, utc_now()


def histogram_delta(before: dict[str, dict[str, int]], after: dict[str, dict[str, int]]) -> dict[str, int]:
    combined: dict[str, int] = {}
    for operation in WRITE_HOLD_OPERATIONS:
        old = before.get(operation, {})
        new = after.get(operation, {})
        if not new or "+Inf" not in new or not set(old).issubset(new):
            raise AcceptanceError(f"production write-hold histogram is missing or changed for {operation}")
        for upper, count in new.items():
            previous = old.get(upper, 0)
            if count < previous:
                raise AcceptanceError("write-hold histogram counters reset or changed buckets during the trial")
            combined[upper] = combined.get(upper, 0) + count - previous
    return combined


def foreground_workload(database: Path) -> tuple[Any, ...]:
    stop = threading.Event()
    failures: list[BaseException] = []
    lock = threading.Lock()
    samples: dict[str, list[dict[str, Any]]] = {"read": [], "write": []}
    added: dict[str, list[str]] = {name: [] for name in DATASETS}

    def reader() -> None:
        connection = sqlite3.connect(database, timeout=1.0)
        connection.execute("PRAGMA busy_timeout=1000")
        tables = [DATASETS[name]["table"] for name in DATASETS]
        index = 0
        try:
            while not stop.is_set():
                table = tables[index % len(tables)]
                index += 1
                started = time.perf_counter()
                list(connection.execute(f"SELECT id,invoke_id,occurred_at FROM {table} ORDER BY occurred_at DESC LIMIT 64"))
                sample = {"observed_at": utc_now(), "elapsed_ms": (time.perf_counter() - started) * 1000, "dataset": table}
                with lock:
                    samples["read"].append(sample)
                stop.wait(0.25)
        except BaseException as error:
            failures.append(error)
        finally:
            connection.close()

    def writer(dataset_name: str) -> None:
        rate = DATASETS[dataset_name]["arrival_rate"]
        end_at = input_deadline[0]
        sequence = 0
        try:
            while True:
                scheduled = input_start_mono[0] + sequence / rate
                if scheduled >= end_at:
                    break
                if stop.wait(max(0, scheduled - time.monotonic())):
                    break
                started = time.perf_counter()
                identity = add_foreground_record(database, dataset_name, sequence + 1)
                sample = {"observed_at": utc_now(), "elapsed_ms": (time.perf_counter() - started) * 1000, "dataset": DATASETS[dataset_name]["table"]}
                with lock:
                    samples["write"].append(sample)
                    added[dataset_name].append(identity)
                sequence += 1
        except BaseException as error:
            failures.append(error)

    input_start_mono = [0.0]
    input_deadline = [0.0]
    threads = [threading.Thread(target=reader, name="retention-reader", daemon=True)]
    threads.extend(threading.Thread(target=writer, args=(name,), name=f"retention-writer-{name}", daemon=True) for name in DATASETS)
    return samples, added, stop, threads, input_start_mono, input_deadline, failures


def task_runs(maintenance_db: Path) -> list[dict[str, Any]]:
    connection = sqlite3.connect(f"file:{maintenance_db}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    try:
        rows = connection.execute(
            "SELECT task_key,trigger_kind,status,actual_started_at,actual_finished_at,processed_count,updated_count,details,summary FROM managed_task_runs WHERE task_key IN ('retention_archive','pool_upstream_attempt_archive','raw_orphan_sweep') AND actual_started_at IS NOT NULL AND actual_finished_at IS NOT NULL ORDER BY id"
        ).fetchall()
        result = []
        for row in rows:
            detail: Any = {}
            try:
                detail = json.loads(row["details"] or "{}")
            except (TypeError, json.JSONDecodeError):
                detail = {}
            released = detail.get("filesReleased", 0) if isinstance(detail, dict) else 0
            result.append({
                "task_key": row["task_key"],
                "trigger_kind": row["trigger_kind"],
                "status": row["status"],
                "actual_started_at": row["actual_started_at"],
                "actual_finished_at": row["actual_finished_at"],
                "processed_count": row["processed_count"],
                "updated_count": row["updated_count"],
                "committed_work": released if row["task_key"] == "raw_orphan_sweep" else (row["processed_count"] or row["updated_count"] or 0),
                "details": detail,
            })
        return result
    finally:
        connection.close()


def task_owners_idle(maintenance_db: Path) -> bool:
    connection = sqlite3.connect(f"file:{maintenance_db}?mode=ro", uri=True)
    try:
        keys = tuple(DATASETS[name]["task_key"] for name in DATASETS)
        placeholders = ",".join("?" for _ in keys)
        pending = connection.execute(
            f"SELECT COUNT(*) FROM managed_task_runs WHERE task_key IN ({placeholders}) AND status IN ('running','requested','queued')",
            keys,
        ).fetchone()[0]
        return pending == 0
    finally:
        connection.close()


def archive_proofs_ready(database: Path, cutoff: str, added_counts: dict[str, int]) -> bool:
    connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    try:
        for name, definition in DATASETS.items():
            dataset = definition["manifest_dataset"]
            table = definition["table"]
            manifests = connection.execute(
                "SELECT id,file_path,sha256 FROM archive_batches WHERE dataset=? AND status='completed' AND cleanup_state='active' AND superseded_by IS NULL",
                (dataset,),
            ).fetchall()
            if not manifests:
                return False
            if name == "invocations":
                targets = {"invocation_rollup_hourly", "upstream_account_stats_hourly", "upstream_account_usage_breakdown_hourly"}
                for archive_id, file_path, manifest_sha in manifests:
                    proof = connection.execute(
                        "SELECT page_count FROM summary_archive_snapshot_v2_proof WHERE archive_batch_id=? AND manifest_sha256=?",
                        (archive_id, manifest_sha),
                    ).fetchone()
                    if proof is None:
                        return False
                    pages = connection.execute(
                        "SELECT COUNT(*) FROM summary_archive_snapshot WHERE archive_batch_id=? AND manifest_sha256=? AND format_version=2",
                        (archive_id, manifest_sha),
                    ).fetchone()[0]
                    if pages != proof[0]:
                        return False
                    replayed = {
                        row[0]
                        for row in connection.execute(
                            "SELECT target FROM hourly_rollup_archive_replay WHERE dataset=? AND file_path=? AND archive_sha256=?",
                            (dataset, file_path, manifest_sha),
                        )
                    }
                    if not targets.issubset(replayed):
                        return False
            converted = definition["seed_rows"] - connection.execute(
                f"SELECT COUNT(*) FROM {table} WHERE invoke_id LIKE 'retention-seed-%' AND occurred_at < ?",
                (cutoff,),
            ).fetchone()[0]
            if converted < math.ceil(10 * added_counts[name]):
                return False
        return True
    finally:
        connection.close()


def wait_for_task_evidence(
    main_db: Path,
    maintenance_db: Path,
    cutoff: str,
    added_counts: dict[str, int],
    started: float,
    timeout_seconds: float = 30,
) -> list[dict[str, Any]]:
    deadline = time.monotonic() + min(timeout_seconds, remaining_budget(started))
    latest: list[dict[str, Any]] = []
    last_error: AcceptanceError | None = None
    while time.monotonic() < deadline:
        latest = task_runs(maintenance_db)
        try:
            validate_task_snapshot(latest)
            if task_owners_idle(maintenance_db) and archive_proofs_ready(main_db, cutoff, added_counts):
                return latest
            last_error = AcceptanceError("retention owners, archive proofs, or 10x source conversion are still draining")
        except AcceptanceError as error:
            last_error = error
        time.sleep(0.5)
    if last_error is not None:
        raise AcceptanceError(f"production task/evidence drain timed out: {last_error}") from last_error
    validate_task_snapshot(latest)
    raise AcceptanceError("production task/evidence drain timed out")


def validate_task_snapshot(rows: list[dict[str, Any]]) -> None:
    by_key: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        if row.get("actual_finished_at") is not None and row.get("status") == "success":
            by_key.setdefault(row["task_key"], []).append(row)
    for key in (DATASETS[name]["task_key"] for name in DATASETS):
        runs = by_key.get(key, [])
        scheduled = [row for row in runs if row.get("trigger_kind") == "schedule"]
        catchups = [row for row in runs if row.get("trigger_kind") == "catchup"]
        if not scheduled or not catchups:
            raise AcceptanceError(f"production dispatcher has not completed both schedule and catch-up runs for {key}")
        if not any(
            parse_time(schedule["actual_finished_at"]) <= parse_time(catchup["actual_started_at"])
            for schedule in scheduled
            for catchup in catchups
        ):
            raise AcceptanceError(f"production catch-up did not follow a scheduled run for {key}")
    competitors = by_key.get("raw_orphan_sweep", [])
    if not any(row.get("committed_work", 0) > 0 for row in competitors):
        raise AcceptanceError("production raw-orphan competitor has not released an old orphan file")
    target_runs = [row for key in (DATASETS[name]["task_key"] for name in DATASETS) for row in by_key.get(key, [])]
    if not any(
        max(parse_time(left["actual_started_at"]), parse_time(right["actual_started_at"]))
        < min(parse_time(left["actual_finished_at"]), parse_time(right["actual_finished_at"]))
        for left in target_runs
        for right in competitors
    ):
        raise AcceptanceError("production orphan sweep did not overlap a real retention owner run")


def run_trial(
    version: str,
    index: int,
    tag: str,
    image: dict[str, str],
    template: Path,
    fingerprint: str,
    root: Path,
    evidence_dir: Path,
    cutoff: str,
    previous_end: str | None,
    phase_started: float,
) -> tuple[dict[str, Any], str]:
    if shanghai_cutoff() != cutoff:
        raise AcceptanceError("Shanghai retention cutoff changed during the experiment")
    trial_start = utc_now()
    trial_dir = root / f"trial-{version}-{index}"
    trial_dir.mkdir()
    for name in ("codex_vibe_monitor.db", "codex_vibe_monitor.maintenance.sqlite"):
        shutil.copyfile(template / name, trial_dir / name)
    main_db = trial_dir / "codex_vibe_monitor.db"
    maintenance_db = trial_dir / "codex_vibe_monitor.maintenance.sqlite"
    raw_dir = trial_dir / "proxy_raw_payloads"
    prepare_competitor(main_db, maintenance_db, raw_dir, RAW_CONTAINER)
    container = f"retention-{RUN_ID}-{version}-{index}"
    container_id = ""
    trial_error: BaseException | None = None
    input_samples: dict[str, list[dict[str, Any]]] = {"read": [], "write": []}
    added: dict[str, list[str]] = {name: [] for name in DATASETS}
    task_evidence: list[dict[str, Any]] = []
    write_scrapes: list[dict[str, Any]] = []
    write_buckets: dict[str, int] | None = None
    oracle: dict[str, Any] | None = None
    input_start = input_end = cleanup_end = None
    container_stopped = False
    wall_start = time.monotonic()
    try:
        container_id = start_container(tag, trial_dir, container)
        wait_healthy(container, phase_started)
        pre_buckets, pre_text, pre_time = scrape_metrics(container)
        write_scrapes.append({"observed_at": pre_time, "scope": "pre-input", "bucket_counts": pre_buckets})
        input_samples, added, stop_event, threads, input_start_mono, input_deadline, failures = foreground_workload(main_db)
        input_start = utc_now()
        input_start_mono[0] = time.monotonic()
        input_deadline[0] = input_start_mono[0] + RUN_SECONDS
        for thread in threads:
            thread.start()
        while time.monotonic() < input_deadline[0]:
            remaining_budget(phase_started)
            time.sleep(min(0.25, max(0, input_deadline[0] - time.monotonic())))
        stop_event.set()
        for thread in threads:
            thread.join(timeout=5)
        if any(thread.is_alive() for thread in threads):
            raise TimeoutError("foreground workload did not stop at the end of its 120-second window")
        if failures:
            raise RuntimeError(f"foreground workload failed: {failures[0]}")
        expected_counts = {
            name: math.ceil(definition["arrival_rate"] * RUN_SECONDS)
            for name, definition in DATASETS.items()
        }
        for name in DATASETS:
            if len(added[name]) != expected_counts[name]:
                raise AcceptanceError(f"{name} arrival rate produced {len(added[name])}, expected {expected_counts[name]} rows")
        input_end = utc_now()
        task_evidence = wait_for_task_evidence(
            main_db,
            maintenance_db,
            cutoff,
            {name: len(added[name]) for name in DATASETS},
            phase_started,
        )
        post_buckets, post_text, post_time = scrape_metrics(container)
        write_scrapes.append({"observed_at": post_time, "scope": "after-drain", "bucket_counts": post_buckets})
        write_buckets = histogram_delta(pre_buckets, post_buckets)
        (evidence_dir / "metrics").mkdir(parents=True, exist_ok=True)
        (evidence_dir / "metrics" / f"{version}-{index}-pre.prom").write_text(pre_text)
        (evidence_dir / "metrics" / f"{version}-{index}-post.prom").write_text(post_text)
        stop_container(container, evidence_dir / "logs" / f"{version}-{index}.log")
        container_stopped = True
        oracle = verify_trial(main_db, trial_dir / "archives", added, root / f"oracle-{version}-{index}")
    except BaseException as error:
        trial_error = error
        write_json(evidence_dir / "partial-trial.json", {
            "version": version,
            "trial_index": index,
            "error": {"type": type(error).__name__, "message": str(error)},
            "input_counts": {name: len(added[name]) for name in DATASETS},
            "added_ids": added,
            "latency_samples_ms": input_samples,
            "task_runs": task_evidence,
            "write_hold_samples": write_scrapes,
            "write_hold_buckets": write_buckets,
        })
    finally:
        if container_id and not container_stopped:
            stop_container(container, evidence_dir / "logs" / f"{version}-{index}.log")
        if input_end is None:
            input_end = utc_now()
        if trial_dir.exists():
            shutil.rmtree(trial_dir)
        cleanup_end = utc_now()

    if trial_error is not None:
        raise trial_error
    if input_start is None or input_end is None or cleanup_end is None or write_buckets is None or oracle is None:
        raise AcceptanceError("trial ended without complete measured windows")
    full_started = parse_time(trial_start)
    input_started = parse_time(input_start)
    input_finished = parse_time(input_end)
    full_finished = parse_time(cleanup_end)
    oracle["datasets"]["invocations"]["added_ids_sha256"] = sha256_bytes("\n".join(added["invocations"]).encode())
    oracle["datasets"]["pool_upstream_attempts"]["added_ids_sha256"] = sha256_bytes("\n".join(added["pool_upstream_attempts"]).encode())
    return {
        "trial_index": index,
        "source_sha": image["source_sha"],
        "image_id": image["image_id"],
        "input_fingerprint": fingerprint,
        "cutoff": cutoff,
        "container_id": container_id,
        "window": {
            "setup_started_at": trial_start,
            "input_started_at": input_start,
            "input_finished_at": input_end,
            "cleanup_finished_at": cleanup_end,
            "input_duration_seconds": (input_finished - input_started).total_seconds(),
            "full_duration_seconds": (full_finished - full_started).total_seconds(),
            "gap_before_seconds": 0.0 if previous_end is None else (full_started - parse_time(previous_end)).total_seconds(),
            "wall_duration_seconds": time.monotonic() - wall_start,
        },
        "input_counts": {name: len(added[name]) for name in DATASETS},
        "added_ids": added,
        "latency_samples_ms": input_samples,
        "task_runs": task_evidence,
        "write_hold_samples": write_scrapes,
        "write_hold_buckets": write_buckets,
        "oracle": oracle,
    }, cleanup_end


def main() -> int:
    evidence_dir = Path(os.environ.get("RETENTION_ACCEPTANCE_EVIDENCE", "retention-acceptance-evidence")).resolve()
    evidence_dir.mkdir(parents=True, exist_ok=True)
    phase_start_iso = os.environ.get("RETENTION_ACCEPTANCE_PHASE_START_UTC", utc_now())
    phase_start = parse_time(phase_start_iso)
    preflight_elapsed = max(0.0, (datetime.now(timezone.utc) - phase_start).total_seconds())
    phase_started = time.monotonic() - preflight_elapsed
    evidence: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "test_phase": {"started_at": phase_start_iso},
        "comparison_window": {"started_at": phase_start_iso},
        "contract": {},
        "versions": {},
    }
    root: Path | None = None
    failure: BaseException | None = None
    try:
        candidate_sha = os.environ.get("CANDIDATE_SHA", "")
        if not re.fullmatch(r"[0-9a-f]{40}", candidate_sha):
            raise AcceptanceError("CANDIDATE_SHA must be the exact 40-character lowercase commit SHA")
        cutoff = shanghai_cutoff()
        tags = {"baseline": f"retention-acceptance:{BASELINE_SHA}", "candidate": f"retention-acceptance:{candidate_sha}"}
        images = {
            "baseline": docker_image_identity(tags["baseline"], BASELINE_SHA),
            "candidate": docker_image_identity(tags["candidate"], candidate_sha),
        }
        host = host_identity()
        evidence["contract"] = {
            "baseline_sha": BASELINE_SHA,
            "candidate_sha": candidate_sha,
            "seed": SEED,
            "retention_days": 1,
            "cutoff": cutoff,
            "seed_rows": {key: value["seed_rows"] for key, value in DATASETS.items()},
            "input_fingerprint": "",
            "input_rates_rows_per_second": {key: value["arrival_rate"] for key, value in DATASETS.items()},
        }
        root = Path(tempfile.mkdtemp(prefix=f"retention-{RUN_ID}-", dir=evidence_dir.parent))
        templates: dict[str, Path] = {}
        fingerprints: dict[str, str] = {}
        for version in ("baseline", "candidate"):
            remaining_budget(phase_started)
            templates[version], fingerprints[version] = prepare_template(tags[version], version, root, cutoff, phase_started)
        if fingerprints["baseline"] != fingerprints["candidate"]:
            raise AcceptanceError("baseline and candidate fixture fingerprints differ")
        fingerprint = fingerprints["baseline"]
        evidence["contract"]["input_fingerprint"] = fingerprint
        previous_end: str | None = None
        for version in ("baseline", "candidate"):
            group_start = utc_now()
            group: dict[str, Any] = {
                "image": images[version],
                "host": host,
                "resource_limits": {"cpus": CPU_LIMIT, "memory_bytes": MEMORY_BYTES},
                "comparison_window": {"started_at": group_start},
                "trials": [],
            }
            evidence["versions"][version] = group
            for index in range(1, 4):
                remaining_budget(phase_started)
                trial, end_at = run_trial(
                    version,
                    index,
                    tags[version],
                    images[version],
                    templates[version],
                    fingerprint,
                    root,
                    evidence_dir,
                    cutoff,
                    previous_end,
                    phase_started,
                )
                previous_end = end_at
                group["trials"].append(trial)
                write_json(evidence_dir / "evidence.partial.json", evidence)
            group_end = utc_now()
            group["comparison_window"]["finished_at"] = group_end
            group["comparison_window"]["elapsed_seconds"] = (parse_time(group_end) - parse_time(group_start)).total_seconds()
        if shanghai_cutoff() != cutoff:
            raise AcceptanceError("Shanghai retention cutoff changed before test-phase cleanup completed")
        if root is not None:
            shutil.rmtree(root)
            root = None
    except BaseException as error:
        failure = error
        evidence["failure"] = {"message": str(error), "type": type(error).__name__}
        (evidence_dir / "failure.traceback.txt").write_text(traceback.format_exc())
    finally:
        if root is not None and root.exists():
            try:
                shutil.rmtree(root)
            except OSError as error:
                if failure is None:
                    failure = error
                    evidence["failure"] = {"message": str(error), "type": type(error).__name__}
        phase_end_iso = utc_now()
        elapsed = time.monotonic() - phase_started
        evidence["test_phase"].update({"finished_at": phase_end_iso, "elapsed_seconds": elapsed})
        evidence["comparison_window"].update({"finished_at": phase_end_iso, "elapsed_seconds": elapsed})
        evidence["status"] = "failed" if failure else "completed"
        write_json(evidence_dir / "evidence.json", evidence)
    if failure is not None:
        print(f"retention acceptance failed: {failure}", file=sys.stderr)
        return 1
    try:
        result = validate_evidence(evidence)
        write_json(evidence_dir / "summary.json", result)
        write_json(evidence_dir / "evidence.json", evidence)
        print(json.dumps(result, indent=2, sort_keys=True))
        return 0
    except BaseException as error:
        evidence["status"] = "failed"
        evidence["validation_failure"] = {"message": str(error), "type": type(error).__name__}
        write_json(evidence_dir / "evidence.json", evidence)
        print(f"retention acceptance failed validation: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BaseException as error:
        print(f"retention acceptance setup failed: {error}", file=sys.stderr)
        raise SystemExit(1)
