"""Run the isolated, time-bounded baseline/candidate retention comparison."""

from __future__ import annotations

from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import platform
import re
import sqlite3
import subprocess
import sys
import tempfile
import time
import traceback
import uuid
from typing import Any

from contract import (
    BASELINE_SHA,
    CLEANUP_RESERVE_SECONDS,
    COMPETITOR_FILE_COUNT,
    COMPETITOR_TASK,
    CONTAINER_KIND_LABEL,
    CONTAINER_OWNER_LABEL,
    CONTAINER_RUN_ID_LABEL,
    CONTAINER_TRIAL_LABEL,
    CONTAINER_VERSION_LABEL,
    DATASETS,
    DRIVER_IMAGE_DIGEST,
    DRIVER_IMAGE_REF,
    MAX_TEST_PHASE_SECONDS,
    RUN_SECONDS,
    SCHEMA_VERSION,
    SEED,
    TRIAL_EXECUTION_ORDER,
    TRIALS_PER_VERSION,
    AcceptanceError,
    parse_time,
    sha256_bytes,
    task_run_is_bounded_active,
    task_run_is_accepted_terminal,
    validate_evidence,
)
from fixture import populate_template, prepare_competitor, shanghai_cutoff
from oracle import verify_trial


CPU_LIMIT = 2
MEMORY_BYTES = 4_294_967_296
CONTAINER_DATA = "/srv/app/data"
RAW_CONTAINER = f"{CONTAINER_DATA}/proxy_raw_payloads"
METRICS_NAME = "cvm_retention_phase_duration_seconds_bucket"
WRITE_HOLD_OPERATIONS = ("invocation_archive", "timestamped_archive")
WRITE_HOLD_PHASE = "write_hold"
RUN_ID = uuid.uuid4().hex[:12]
WORKER_IMAGE = DRIVER_IMAGE_REF


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def run_command(
    command: list[str],
    timeout: float,
    check: bool = True,
    *,
    phase_started: float | None = None,
    cleanup: bool = False,
    cleanup_reserve_seconds: float = 0,
) -> subprocess.CompletedProcess[str]:
    if phase_started is not None:
        available = cleanup_budget(phase_started, cleanup_reserve_seconds) if cleanup else remaining_budget(phase_started)
        timeout = min(timeout, available)
    if timeout <= 0:
        raise TimeoutError(f"no time remains to run {command[0]}")
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
    left = MAX_TEST_PHASE_SECONDS - (time.monotonic() - started) - CLEANUP_RESERVE_SECONDS
    if left <= 0:
        raise TimeoutError("retention acceptance reached its work cutoff before the 900-second cleanup deadline")
    return left


def cleanup_budget(started: float, reserve_seconds: float = 0) -> float:
    left = MAX_TEST_PHASE_SECONDS - (time.monotonic() - started) - reserve_seconds
    if left <= 0:
        raise TimeoutError("retention acceptance exceeded its 900-second test-phase cleanup budget")
    return left


class OwnedContainerLedger:
    def __init__(self) -> None:
        self._records: dict[str, dict[str, Any]] = {}

    def register(
        self,
        name: str,
        kind: str,
        root: Path,
        recovery_path: Path,
        mount_path: Path | None = None,
        *,
        version: str,
        trial_index: int | None,
    ) -> None:
        if name in self._records:
            raise AcceptanceError(f"owned container name was registered more than once: {name}")
        if kind not in {"init", "application", "foreground"}:
            raise AcceptanceError(f"unsupported owned container kind: {kind}")
        if version not in {"baseline", "candidate"}:
            raise AcceptanceError(f"unsupported owned container version: {version}")
        if (kind == "init" and trial_index is not None) or (
            kind != "init" and (not isinstance(trial_index, int) or not 1 <= trial_index <= TRIALS_PER_VERSION)
        ):
            raise AcceptanceError(f"owned container trial identity is invalid for {kind}")
        if not re.fullmatch(r"[0-9a-f]{12}", RUN_ID):
            raise AcceptanceError("retention run ID is malformed")
        identity_labels = {
            CONTAINER_RUN_ID_LABEL: RUN_ID,
            CONTAINER_KIND_LABEL: kind,
            CONTAINER_VERSION_LABEL: version,
            CONTAINER_TRIAL_LABEL: "init" if trial_index is None else str(trial_index),
            CONTAINER_OWNER_LABEL: uuid.uuid4().hex,
        }
        self._records[name] = {
            "container_name": name,
            "container_id": None,
            "kind": kind,
            "identity_labels": identity_labels,
            "verified_identity_labels": None,
            "status": "creation_unconfirmed",
            "root_path": str(root),
            "recovery_path": str(recovery_path),
            "mount_path": str(mount_path) if mount_path is not None else None,
            "cleanup_attempts": [],
        }

    def mark_started(self, name: str, container_id: str) -> None:
        if not re.fullmatch(r"[0-9a-f]{64}", container_id):
            raise AcceptanceError(f"owned container ID is malformed for {name}")
        record = self._records[name]
        record["container_id"] = container_id
        record["status"] = "started"

    def mark_failure(self, name: str, error: BaseException) -> None:
        record = self._records[name]
        detail = {"type": type(error).__name__, "message": str(error)}
        record["status"] = "unconfirmed"
        record["last_error"] = detail
        record["cleanup_attempts"].append({"status": "unconfirmed", "error": detail})

    def mark_cleanup(self, name: str, result: dict[str, str]) -> None:
        record = self._records[name]
        container_id = result.get("container_id")
        if container_id:
            if record["container_id"] and record["container_id"] != container_id:
                raise AcceptanceError(f"owned container identity changed for {name}")
            record["container_id"] = container_id
        record["status"] = result["status"]
        labels = result.get("identity_labels")
        if labels:
            record["verified_identity_labels"] = dict(labels)
        record.pop("last_error", None)
        record["cleanup_attempts"].append({
            "status": result["status"],
            **({"container_id": container_id} if container_id else {}),
            **({"identity_labels": dict(labels)} if labels else {}),
        })

    def names(self) -> set[str]:
        return set(self._records)

    def record(self, name: str) -> dict[str, Any]:
        record = self._records[name]
        return {
            **record,
            "identity_labels": dict(record["identity_labels"]),
            "verified_identity_labels": (
                dict(record["verified_identity_labels"])
                if record["verified_identity_labels"] is not None
                else None
            ),
            "cleanup_attempts": list(record["cleanup_attempts"]),
        }

    def docker_label_arguments(self, name: str) -> list[str]:
        labels = self._records[name]["identity_labels"]
        return [argument for key, value in sorted(labels.items()) for argument in ("--label", f"{key}={value}")]

    def records(self, names: set[str] | None = None) -> list[dict[str, Any]]:
        selected = self._records if names is None else {name: self._records[name] for name in names if name in self._records}
        return [self.record(name) for name in sorted(selected)]

    def unresolved(self, names: set[str] | None = None) -> list[dict[str, Any]]:
        return [record for record in self.records(names) if record["status"] not in {"removed", "already_absent"}]

    def cleanup(
        self,
        name: str,
        phase_started: float,
        cleanup_reserve_seconds: float,
        log_path: Path | None = None,
    ) -> dict[str, str]:
        record = self._records[name]
        try:
            if log_path is None:
                result = cleanup_named_container(
                    name,
                    phase_started,
                    cleanup_reserve_seconds,
                    expected_container_id=record["container_id"],
                    expected_identity_labels=record["identity_labels"],
                )
            else:
                result = cleanup_named_container(
                    name,
                    phase_started,
                    cleanup_reserve_seconds,
                    expected_container_id=record["container_id"],
                    expected_identity_labels=record["identity_labels"],
                    log_path=log_path,
                )
        except BaseException as error:
            self.mark_failure(name, error)
            raise
        self.mark_cleanup(name, result)
        return result

    def retry_unresolved(
        self,
        phase_started: float,
        cleanup_reserve_seconds: float,
        names: set[str] | None = None,
    ) -> list[BaseException]:
        errors: list[BaseException] = []
        for record in self.unresolved(names):
            try:
                self.cleanup(record["container_name"], phase_started, cleanup_reserve_seconds)
            except BaseException as error:
                errors.append(error)
        return errors

    def recovery_payload(
        self,
        root: Path,
        path: Path | None = None,
        names: set[str] | None = None,
    ) -> dict[str, Any]:
        unresolved = self.unresolved(names)
        preserved_paths = {str(root)}
        if path is not None:
            preserved_paths.add(str(path))
        for record in unresolved:
            preserved_paths.add(record["recovery_path"])
            preserved_paths.add(record["root_path"])
        return {
            "root_path": str(root),
            "preserved_paths": sorted(preserved_paths),
            "unresolved_containers": unresolved,
        }


def docker_image_identity(tag: str, expected_sha: str, phase_started: float) -> dict[str, str]:
    image_id = run_command(["docker", "image", "inspect", "--format", "{{.Id}}", tag], 20, phase_started=phase_started).stdout.strip()
    labels = json.loads(run_command(["docker", "image", "inspect", "--format", "{{json .Config.Labels}}", tag], 20, phase_started=phase_started).stdout)
    if labels.get("org.opencontainers.image.revision") != expected_sha:
        raise AcceptanceError(f"loaded image {tag} does not identify source {expected_sha}")
    if not image_id.startswith("sha256:"):
        raise AcceptanceError(f"loaded image {tag} has no immutable Docker image ID")
    return {"source_sha": expected_sha, "image_id": image_id}


def host_identity(phase_started: float) -> dict[str, Any]:
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
        "docker_server": run_command(["docker", "version", "--format", "{{.Server.Version}}"], 20, phase_started=phase_started).stdout.strip(),
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
        "UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET": "retention-acceptance-synthetic-encryption-key",
        "RUST_LOG": "warn",
    }


def host_container_user() -> str:
    return f"{os.getuid()}:{os.getgid()}"


def start_container(
    tag: str,
    data_dir: Path,
    name: str,
    phase_started: float,
    *,
    owned: OwnedContainerLedger,
    version: str,
    trial_index: int | None,
    kind: str = "application",
    root: Path | None = None,
) -> str:
    data_dir.mkdir(parents=True, exist_ok=True)
    owned.register(
        name,
        kind,
        root or data_dir.parent,
        data_dir,
        data_dir,
        version=version,
        trial_index=trial_index,
    )
    command = [
        "docker", "run", "--detach", "--name", name,
        # Keep bind-mounted SQLite and archive files writable by host-side fixture steps.
        "--user", host_container_user(),
        *owned.docker_label_arguments(name),
        "--cpus", str(CPU_LIMIT),
        "--memory", str(MEMORY_BYTES),
        "--memory-swap", str(MEMORY_BYTES),
        "--network", "none",
        "--mount", f"type=bind,source={data_dir.resolve()},target={CONTAINER_DATA}",
    ]
    for key, value in app_environment().items():
        command.extend(["--env", f"{key}={value}"])
    command.append(tag)
    try:
        container_id = run_command(command, 60, phase_started=phase_started).stdout.strip()
        owned.mark_started(name, container_id)
    except BaseException as error:
        owned.mark_failure(name, error)
        raise
    return container_id


def wait_healthy(container: str, started: float) -> None:
    deadline = time.monotonic() + min(60, remaining_budget(started))
    while time.monotonic() < deadline:
        check = run_command(
            ["docker", "exec", container, "curl", "-fsS", "-m", "1", "http://127.0.0.1:8080/health"],
            5,
            check=False,
            phase_started=started,
        )
        if check.returncode == 0 and "ok" in check.stdout.lower():
            return
        time.sleep(0.3)
    raise TimeoutError(f"production image {container} failed its health check")


def inspect_container_identity(
    reference: str,
    phase_started: float,
    cleanup_reserve_seconds: float,
) -> tuple[str, str, dict[str, Any]] | None:
    inspected = run_command(
        ["docker", "inspect", "--format", "{{.Id}} {{.State.Running}} {{json .Config.Labels}}", reference],
        3,
        check=False,
        phase_started=phase_started,
        cleanup=True,
        cleanup_reserve_seconds=cleanup_reserve_seconds,
    )
    if inspected.returncode != 0 and re.search(r"No such (?:object|container)", inspected.stderr, re.IGNORECASE):
        return None
    if inspected.returncode != 0:
        raise RuntimeError(f"could not inspect named container {reference}: {inspected.stderr.strip()[-1000:]}")
    identity = inspected.stdout.strip().split(maxsplit=2)
    if len(identity) != 3 or not re.fullmatch(r"[0-9a-f]{64}", identity[0]) or identity[1] not in {"true", "false"}:
        raise RuntimeError(f"could not parse container identity {reference}: {inspected.stdout.strip()[-1000:]}")
    try:
        actual_labels = json.loads(identity[2])
    except json.JSONDecodeError as error:
        raise RuntimeError(f"could not parse container labels {reference}") from error
    if not isinstance(actual_labels, dict):
        actual_labels = {}
    return identity[0], identity[1], actual_labels


def find_container_by_identity_labels(
    expected_identity_labels: dict[str, str],
    phase_started: float,
    cleanup_reserve_seconds: float,
) -> tuple[str, str, dict[str, Any]] | None:
    if not expected_identity_labels:
        raise AcceptanceError("cannot search for an unconfirmed container without owner identity labels")
    command = ["docker", "ps", "--all", "--quiet", "--no-trunc"]
    for key, value in sorted(expected_identity_labels.items()):
        command.extend(["--filter", f"label={key}={value}"])
    listed = run_command(
        command,
        3,
        check=False,
        phase_started=phase_started,
        cleanup=True,
        cleanup_reserve_seconds=cleanup_reserve_seconds,
    )
    if listed.returncode != 0:
        raise RuntimeError(f"could not search for owned containers: {listed.stderr.strip()[-1000:]}")
    container_ids = [line.strip() for line in listed.stdout.splitlines() if line.strip()]
    if any(not re.fullmatch(r"[0-9a-f]{64}", container_id) for container_id in container_ids):
        raise RuntimeError("container identity search returned a malformed ID")
    if len(container_ids) > 1:
        raise AcceptanceError("multiple containers match the unconfirmed owner identity")
    if not container_ids:
        return None
    return inspect_container_identity(container_ids[0], phase_started, cleanup_reserve_seconds)


def cleanup_named_container(
    container: str,
    phase_started: float,
    cleanup_reserve_seconds: float,
    *,
    expected_container_id: str | None = None,
    expected_identity_labels: dict[str, str] | None = None,
    log_path: Path | None = None,
) -> dict[str, str]:
    identity = inspect_container_identity(container, phase_started, cleanup_reserve_seconds)
    if identity is None:
        if expected_container_id is not None:
            identity = inspect_container_identity(
                expected_container_id,
                phase_started,
                cleanup_reserve_seconds,
            )
            if identity is None:
                identity = inspect_container_identity(container, phase_started, cleanup_reserve_seconds)
                if identity is None:
                    return {
                        "container_name": container,
                        "container_id": expected_container_id,
                        "status": "already_absent",
                        **({"identity_labels": expected_identity_labels} if expected_identity_labels is not None else {}),
                    }
        elif expected_identity_labels is not None:
            identity = find_container_by_identity_labels(
                expected_identity_labels,
                phase_started,
                cleanup_reserve_seconds,
            )
            if identity is None:
                identity = inspect_container_identity(container, phase_started, cleanup_reserve_seconds)
                if identity is None:
                    raise AcceptanceError(
                        f"could not confirm unstarted container {container} is absent by owner identity"
                    )
        else:
            return {"container_name": container, "status": "already_absent"}

    container_id, running, actual_labels = identity
    if expected_container_id and container_id != expected_container_id:
        raise AcceptanceError(f"named container {container} resolved to an unexpected container ID")
    if expected_identity_labels is None:
        if expected_container_id is None:
            raise AcceptanceError(f"named container {container} has no confirmed owner; refusing cleanup by name")
        expected_identity_labels = {}
    if any(actual_labels.get(key) != value for key, value in expected_identity_labels.items()):
        raise AcceptanceError(f"named container {container} has an unexpected run identity; refusing cleanup")
    verified_labels = {key: actual_labels[key] for key in expected_identity_labels}
    log_error = None
    if log_path is not None:
        try:
            logs = run_command(
                ["docker", "logs", container_id],
                4,
                check=False,
                phase_started=phase_started,
                cleanup=True,
                cleanup_reserve_seconds=cleanup_reserve_seconds,
            )
            log_path.parent.mkdir(parents=True, exist_ok=True)
            log_path.write_text(logs.stdout + logs.stderr)
        except BaseException as error:
            log_error = f"{type(error).__name__}: {error}"
            try:
                log_path.parent.mkdir(parents=True, exist_ok=True)
                log_path.write_text(f"container log collection failed: {log_error}\n")
            except OSError:
                pass
    command_errors: list[str] = []
    if running == "true":
        try:
            result = run_command(
                ["docker", "stop", "--time", "3", container_id],
                5,
                check=False,
                phase_started=phase_started,
                cleanup=True,
                cleanup_reserve_seconds=cleanup_reserve_seconds,
            )
            if result.returncode != 0:
                command_errors.append(result.stderr.strip()[-1000:] or "docker stop returned non-zero")
        except BaseException as error:
            command_errors.append(f"docker stop: {type(error).__name__}: {error}")
    try:
        result = run_command(
            ["docker", "rm", "--force", container_id],
            5,
            check=False,
            phase_started=phase_started,
            cleanup=True,
            cleanup_reserve_seconds=cleanup_reserve_seconds,
        )
        if result.returncode != 0:
            command_errors.append(result.stderr.strip()[-1000:] or "docker rm returned non-zero")
    except BaseException as error:
        command_errors.append(f"docker rm: {type(error).__name__}: {error}")
    remaining = run_command(
        ["docker", "inspect", "--format", "{{.Id}}", container_id],
        3,
        check=False,
        phase_started=phase_started,
        cleanup=True,
        cleanup_reserve_seconds=cleanup_reserve_seconds,
    )
    if remaining.returncode == 0:
        raise AcceptanceError(
            f"owned container {container} ({container_id}) remained after cleanup"
            + (": " + "; ".join(command_errors) if command_errors else "")
        )
    if not re.search(r"No such (?:object|container)", remaining.stderr, re.IGNORECASE):
        detail = remaining.stderr.strip()[-1000:]
        if command_errors:
            detail += "; " + "; ".join(command_errors)
        raise RuntimeError(f"could not confirm owned container {container} ({container_id}) cleanup: {detail}")
    replacement = inspect_container_identity(container, phase_started, cleanup_reserve_seconds)
    if replacement is not None:
        raise AcceptanceError(
            f"container name {container} was reused by {replacement[0]} during cleanup; preserving mounted data"
        )
    return {
        "container_name": container,
        "container_id": container_id,
        "status": "removed",
        "identity_labels": verified_labels,
        **({"log_error": log_error} if log_error is not None else {}),
    }


def bounded_rmtree(path: Path, phase_started: float, cleanup_reserve_seconds: float) -> None:
    if not path.exists():
        return
    run_command(
        [sys.executable, "-c", "import shutil,sys;shutil.rmtree(sys.argv[1])", str(path)],
        cleanup_budget(phase_started, cleanup_reserve_seconds),
        phase_started=phase_started,
        cleanup=True,
        cleanup_reserve_seconds=cleanup_reserve_seconds,
    )


def cleanup_path_if_safe(
    path: Path,
    root: Path,
    owned: OwnedContainerLedger,
    phase_started: float,
    cleanup_reserve_seconds: float,
    names: set[str] | None = None,
) -> tuple[dict[str, Any], list[BaseException]]:
    errors = owned.retry_unresolved(phase_started, cleanup_reserve_seconds, names)
    unresolved = owned.unresolved(names)
    if unresolved:
        return {
            "removed": False,
            "status": "preserved",
            "reason": "owned container absence could not be confirmed",
            "recovery": owned.recovery_payload(root, path, names),
        }, errors
    try:
        bounded_rmtree(path, phase_started, cleanup_reserve_seconds)
    except BaseException as error:
        errors.append(error)
        return {
            "removed": False,
            "status": "preserved",
            "reason": "bounded directory cleanup did not complete",
            "recovery": {
                "root_path": str(root),
                "preserved_paths": sorted({str(root), str(path)}),
                "unresolved_containers": [],
                "cleanup_error": {"type": type(error).__name__, "message": str(error)},
            },
        }, errors
    return {"removed": True, "status": "removed"}, errors


def bounded_copy(source: Path, destination: Path, phase_started: float) -> None:
    run_command(
        [sys.executable, "-c", "import shutil,sys;shutil.copyfile(sys.argv[1],sys.argv[2])", str(source), str(destination)],
        min(60, remaining_budget(phase_started)),
        phase_started=phase_started,
    )


def checkpoint_and_reset_history(main_db: Path, maintenance_db: Path, phase_started: float) -> None:
    main = sqlite3.connect(main_db, timeout=min(30, remaining_budget(phase_started)))
    try:
        main.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    finally:
        main.close()
    maintenance = sqlite3.connect(maintenance_db, timeout=min(30, remaining_budget(phase_started)))
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


def prepare_template(
    tag: str,
    version: str,
    root: Path,
    cutoff: str,
    started: float,
    owned: OwnedContainerLedger | None = None,
) -> tuple[Path, str]:
    owned = owned or OwnedContainerLedger()
    directory = root / f"template-{version}"
    directory.mkdir(parents=True)
    container = f"retention-{RUN_ID}-init-{version}"
    try:
        start_container(
            tag,
            directory,
            container,
            started,
            owned=owned,
            kind="init",
            root=root,
            version=version,
            trial_index=None,
        )
        wait_healthy(container, started)
    finally:
        owned.cleanup(
            container,
            started,
            cleanup_reserve_seconds=25,
            log_path=root / "logs" / f"init-{version}.log",
        )
    main_db = directory / "codex_vibe_monitor.db"
    maintenance_db = directory / "codex_vibe_monitor.maintenance.sqlite"
    if not main_db.is_file() or not maintenance_db.is_file():
        raise AcceptanceError(f"{version} production startup did not create both SQLite databases")
    fingerprint = populate_template(main_db, cutoff, budget_check=lambda: remaining_budget(started))
    checkpoint_and_reset_history(main_db, maintenance_db, started)
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


def scrape_metrics(container: str, phase_started: float) -> tuple[dict[str, dict[str, int]], str, str]:
    result = run_command(
        ["docker", "exec", container, "curl", "-fsS", "-m", "5", "http://127.0.0.1:9091/metrics"],
        10,
        phase_started=phase_started,
    )
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


def worker_image_identity(started: float) -> dict[str, str]:
    run_command(["docker", "pull", WORKER_IMAGE], min(120, remaining_budget(started)), phase_started=started)
    image_id = run_command(
        ["docker", "image", "inspect", "--format", "{{.Id}}", WORKER_IMAGE],
        20,
        phase_started=started,
    ).stdout.strip()
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", image_id):
        raise AcceptanceError("foreground HTTP driver image has no immutable image ID")
    return {"image_ref": WORKER_IMAGE, "digest": DRIVER_IMAGE_DIGEST, "image_id": image_id}


def run_foreground(
    container: str,
    driver_image_id: str,
    started: float,
    trial_version: str,
    trial_index: int,
    cleanup_evidence: dict[str, Any],
    owned: OwnedContainerLedger,
    root: Path | None = None,
    recovery_path: Path | None = None,
) -> dict[str, Any]:
    if trial_version not in {"baseline", "candidate"} or not 1 <= trial_index <= TRIALS_PER_VERSION:
        raise AcceptanceError("foreground driver identity must bind to a known trial")
    invocation_count = math.ceil(DATASETS["invocations"]["arrival_rate"] * RUN_SECONDS)
    attempt_count = math.ceil(DATASETS["pool_upstream_attempts"]["arrival_rate"] * RUN_SECONDS)
    retries = attempt_count - invocation_count
    worker = Path(__file__).with_name("foreground.py").resolve()
    name = f"retention-{RUN_ID}-{trial_version}-{trial_index}-driver"
    owned.register(
        name,
        "foreground",
        root or worker.parent,
        recovery_path or worker.parent,
        worker,
        version=trial_version,
        trial_index=trial_index,
    )
    try:
        result = run_command(
            [
                "docker", "run", "--name", name,
                *owned.docker_label_arguments(name),
                "--network", f"container:{container}",
                "--cap-drop", "ALL",
                "--security-opt", "no-new-privileges",
                "--read-only",
                "--pull=never",
                "--tmpfs", "/tmp:rw,noexec,nosuid,size=16m",
                "--mount", f"type=bind,source={worker},target=/foreground.py,readonly",
                "--env", "PYTHONDONTWRITEBYTECODE=1",
                driver_image_id,
                "python3", "/foreground.py",
                "--run-seconds", str(RUN_SECONDS),
                "--retry-count", str(retries),
            ],
            timeout=min(RUN_SECONDS + 90, remaining_budget(started)),
            phase_started=started,
        )
    finally:
        try:
            cleanup = owned.cleanup(name, started, cleanup_reserve_seconds=30)
            cleanup_evidence.update(cleanup)
        except BaseException as error:
            cleanup_evidence.update({
                "container_name": name,
                "status": "failed",
                **({"container_id": owned.record(name)["container_id"]} if owned.record(name)["container_id"] else {}),
                "recovery_path": owned.record(name)["recovery_path"],
                "error": {"type": type(error).__name__, "message": str(error)},
            })
            raise
    try:
        evidence = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise AcceptanceError("foreground HTTP driver did not emit valid JSON evidence") from error
    if not isinstance(evidence, dict):
        raise AcceptanceError("foreground HTTP driver evidence is malformed")
    return evidence


def observed_attempt_states(
    database: Path,
    invocations: list[dict[str, Any]],
    retry_count: int,
    started: float,
) -> list[dict[str, Any]]:
    invoke_ids = [row["invoke_id"] for row in invocations]
    retry_sequences = {
        math.floor(index * len(invocations) / retry_count)
        for index in range(retry_count)
    } if retry_count else set()
    expected_statuses = {
        row["invoke_id"]: (["http_failure", "success"] if row["sequence"] in retry_sequences else ["success"])
        for row in invocations
    }
    expected_counts = {invoke_id: len(statuses) for invoke_id, statuses in expected_statuses.items()}
    placeholders = ",".join("?" for _ in invoke_ids)
    deadline = time.monotonic() + min(30, remaining_budget(started))
    latest_states: list[dict[str, Any]] = []
    latest_counts: dict[str, int] = {}
    while time.monotonic() < deadline:
        connection = sqlite3.connect(f"file:{database}?mode=ro", uri=True, timeout=0.5)
        try:
            connection.execute("PRAGMA busy_timeout=500")
            rows = connection.execute(
                f"SELECT attempt_public_id,invoke_id,attempt_index,status,finished_at FROM pool_upstream_request_attempts WHERE invoke_id IN ({placeholders}) ORDER BY id",
                invoke_ids,
            ).fetchall()
        finally:
            connection.close()
        latest_states = [
            {
                "attempt_public_id": row[0],
                "invoke_id": row[1],
                "attempt_index": row[2],
                "status": row[3],
                "finished_at": row[4],
            }
            for row in rows
        ]
        latest_counts = {}
        for row in rows:
            invoke_id = row[1]
            latest_counts[invoke_id] = latest_counts.get(invoke_id, 0) + 1
        ids = [row["attempt_public_id"] for row in latest_states]
        terminal = all(
            isinstance(row["attempt_public_id"], str)
            and row["attempt_public_id"]
            and isinstance(row["finished_at"], str)
            and row["status"] in {"success", "http_failure"}
            for row in latest_states
        )
        statuses_by_invoke: dict[str, list[dict[str, Any]]] = {}
        for row in latest_states:
            statuses_by_invoke.setdefault(row["invoke_id"], []).append(row)
        states_match = all(
            [row["status"] for row in sorted(
                statuses_by_invoke.get(invoke_id, []),
                key=lambda row: row["attempt_index"] if isinstance(row["attempt_index"], int) else 0,
            )] == expected
            and [row["attempt_index"] for row in sorted(
                statuses_by_invoke.get(invoke_id, []),
                key=lambda row: row["attempt_index"] if isinstance(row["attempt_index"], int) else 0,
            )] == list(range(1, len(expected) + 1))
            for invoke_id, expected in expected_statuses.items()
        )
        if (
            len(ids) == len(rows)
            and len(set(ids)) == len(ids)
            and latest_counts == expected_counts
            and terminal
            and states_match
        ):
            return latest_states
        if len(rows) > sum(expected_counts.values()):
            raise AcceptanceError("production HTTP workload produced unexpected upstream attempt rows")
        time.sleep(0.25)
    raise AcceptanceError(
        "production HTTP attempt IDs did not settle to the expected per-invocation counts "
        f"(rows={len(latest_states)}, expected={sum(expected_counts.values())}, invocations={len(latest_counts)})"
    )


def task_runs(maintenance_db: Path) -> list[dict[str, Any]]:
    connection = sqlite3.connect(f"file:{maintenance_db}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    try:
        observed_at = utc_now()
        progress_rows = connection.execute(
            "SELECT task_key,total,completed,phase,checkpoint,eta_seconds,updated_at,freshness,unit,source_scope,last_progress_at,wait_reason,next_retry_at,next_inspection_at,next_catchup_at,catchup_state,stages "
            "FROM managed_task_progress WHERE task_key IN ('retention_archive','pool_upstream_attempt_archive','raw_orphan_sweep')"
        ).fetchall()
        progress_by_key: dict[str, dict[str, Any]] = {}
        for progress in progress_rows:
            stages: Any = None
            try:
                stages = json.loads(progress[16]) if progress[16] else None
            except (TypeError, json.JSONDecodeError):
                stages = {"raw": progress[16]}
            progress_by_key[progress[0]] = {
                "total": progress[1],
                "completed": progress[2],
                "phase": progress[3],
                "checkpoint": progress[4],
                "eta_seconds": progress[5],
                "updated_at": progress[6],
                "freshness": progress[7],
                "unit": progress[8],
                "source_scope": progress[9],
                "last_progress_at": progress[10],
                "wait_reason": progress[11],
                "next_retry_at": progress[12],
                "next_inspection_at": progress[13],
                "next_catchup_at": progress[14],
                "catchup_state": progress[15],
                "stages": stages,
            }
        rows = connection.execute(
            "SELECT task_key,trigger_kind,status,actual_started_at,actual_finished_at,processed_count,updated_count,details,summary,error_detail,completion FROM managed_task_runs WHERE task_key IN ('retention_archive','pool_upstream_attempt_archive','raw_orphan_sweep') AND actual_started_at IS NOT NULL ORDER BY id"
        ).fetchall()
        result = []
        for row in rows:
            detail: Any = {}
            try:
                detail = json.loads(row["details"] or "{}")
            except (TypeError, json.JSONDecodeError):
                detail = {}
            released = detail.get("filesReleased", 0) if isinstance(detail, dict) else 0
            evidence = {
                "task_key": row["task_key"],
                "trigger_kind": row["trigger_kind"],
                "status": row["status"],
                "actual_started_at": row["actual_started_at"],
                "actual_finished_at": row["actual_finished_at"],
                "processed_count": row["processed_count"],
                "updated_count": row["updated_count"],
                "committed_work": released if row["task_key"] == "raw_orphan_sweep" else (row["processed_count"] or row["updated_count"] or 0),
                "details": detail,
                "error_detail": row["error_detail"],
                "completion": row["completion"],
            }
            progress = progress_by_key.get(row["task_key"])
            if progress is not None:
                evidence["progress"] = progress
            if row["actual_finished_at"] is None:
                evidence["evidence_kind"] = "bounded_active" if task_run_is_bounded_active({
                    **evidence,
                    "observed_at": observed_at,
                }) else "active"
                evidence["observed_at"] = observed_at
            result.append(evidence)
        return result
    finally:
        connection.close()


def archive_proofs_ready(
    database: Path,
    cutoff: str,
    added_counts: dict[str, int],
    version: str,
) -> bool:
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
            identity_column = definition["identity_column"]
            converted = definition["seed_rows"] - connection.execute(
                f"SELECT COUNT(*) FROM {table} WHERE {identity_column} LIKE 'retention-seed-%' AND occurred_at < ?",
                (cutoff,),
            ).fetchone()[0]
            required = math.ceil(10 * added_counts[name]) if version == "candidate" else 1
            if converted < required:
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
    version: str,
    timeout_seconds: float | None = None,
) -> list[dict[str, Any]]:
    wait_budget = MAX_TEST_PHASE_SECONDS if timeout_seconds is None else timeout_seconds
    deadline = time.monotonic() + min(wait_budget, remaining_budget(started))
    latest: list[dict[str, Any]] = []
    last_error: AcceptanceError | None = None
    while time.monotonic() < deadline:
        latest = task_runs(maintenance_db)
        try:
            validate_task_snapshot(latest, version)
            # The fixed seed snapshot intentionally contains 100,000 expired rows. The legacy
            # baseline may still be draining that backlog when its 10x cohort is already proved;
            # the candidate must still complete its schedule/catch-up chain before returning.
            if archive_proofs_ready(main_db, cutoff, added_counts, version):
                return latest
            last_error = AcceptanceError("archive proofs or 10x source conversion are still draining")
        except AcceptanceError as error:
            last_error = error
        time.sleep(0.5)
    if last_error is not None:
        raise AcceptanceError(f"production task/evidence drain timed out: {last_error}") from last_error
    validate_task_snapshot(latest, version)
    raise AcceptanceError("production task/evidence drain timed out")


def validate_task_snapshot(rows: list[dict[str, Any]], version: str = "candidate") -> None:
    by_key: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        if row.get("actual_finished_at") is not None and task_run_is_accepted_terminal(row):
            by_key.setdefault(row["task_key"], []).append(row)
        elif row.get("actual_finished_at") is None and task_run_is_bounded_active(row):
            by_key.setdefault(row["task_key"], []).append(row)
    for key in (DATASETS[name]["task_key"] for name in DATASETS):
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
            parse_time(schedule["actual_finished_at"]) <= parse_time(active["observed_at"])
            for schedule in scheduled
            for active in active_catchups
        ))
        if version == "baseline":
            if not (completed_chain or bounded_chain):
                raise AcceptanceError(f"production dispatcher has not completed both schedule and catch-up runs or produced a bounded baseline observation for {key}")
        elif not completed_chain:
            raise AcceptanceError(f"production dispatcher has not completed both schedule and catch-up runs for {key}")
    competitors = by_key.get("raw_orphan_sweep", [])
    released = sum(row.get("committed_work", 0) for row in competitors)
    if released < COMPETITOR_FILE_COUNT:
        raise AcceptanceError("production raw-orphan competitor has not completed the fixed workload")
    if released > COMPETITOR_FILE_COUNT:
        raise AcceptanceError("production raw-orphan competitor exceeded the fixed workload")


def run_trial(
    version: str,
    index: int,
    tag: str,
    image: dict[str, str],
    driver_image_id: str,
    template: Path,
    fingerprint: str,
    root: Path,
    evidence_dir: Path,
    cutoff: str,
    previous_end: str | None,
    phase_started: float,
    owned: OwnedContainerLedger | None = None,
) -> tuple[dict[str, Any], str]:
    if shanghai_cutoff() != cutoff:
        raise AcceptanceError("Shanghai retention cutoff changed during the experiment")
    trial_start = utc_now()
    trial_dir = root / f"trial-{version}-{index}"
    trial_dir.mkdir()
    for name in ("codex_vibe_monitor.db", "codex_vibe_monitor.maintenance.sqlite"):
        bounded_copy(template / name, trial_dir / name, phase_started)
    main_db = trial_dir / "codex_vibe_monitor.db"
    maintenance_db = trial_dir / "codex_vibe_monitor.maintenance.sqlite"
    raw_dir = trial_dir / "proxy_raw_payloads"
    remaining_budget(phase_started)
    prepare_competitor(
        main_db,
        maintenance_db,
        raw_dir,
        RAW_CONTAINER,
        budget_check=lambda: remaining_budget(phase_started),
    )
    container = f"retention-{RUN_ID}-{version}-{index}"
    containers = owned or OwnedContainerLedger()
    prior_container_names = containers.names()
    trial_container_names: set[str] = set()
    container_id = ""
    trial_error: BaseException | None = None
    cleanup_errors: list[BaseException] = []
    input_samples: dict[str, list[dict[str, Any]]] = {"read": [], "write": []}
    added: dict[str, list[str]] = {name: [] for name in DATASETS}
    attempt_states: list[dict[str, Any]] = []
    driver_cleanup: dict[str, Any] = {}
    foreground_evidence: dict[str, Any] = {}
    task_evidence: list[dict[str, Any]] = []
    write_scrapes: list[dict[str, Any]] = []
    write_buckets: dict[str, int] | None = None
    oracle: dict[str, Any] | None = None
    input_start = input_end = cleanup_end = None
    container_stopped = False
    wall_start = time.monotonic()
    try:
        container_id = start_container(
            tag,
            trial_dir,
            container,
            phase_started,
            owned=containers,
            kind="application",
            root=root,
            version=version,
            trial_index=index,
        )
        wait_healthy(container, phase_started)
        pre_buckets, pre_text, pre_time = scrape_metrics(container, phase_started)
        write_scrapes.append({"observed_at": pre_time, "scope": "pre-input", "bucket_counts": pre_buckets})
        foreground = run_foreground(
            container,
            driver_image_id,
            phase_started,
            version,
            index,
            driver_cleanup,
            owned=containers,
            root=root,
            recovery_path=trial_dir,
        )
        input_start = foreground.get("input_started_at")
        input_end = foreground.get("input_finished_at")
        input_samples = foreground.get("latency_samples_ms", input_samples)
        invocation_records = foreground.get("invocations")
        foreground_evidence = foreground.get("foreground_workload", {})
        if not isinstance(invocation_records, list):
            raise AcceptanceError("foreground HTTP driver omitted invocation identities")
        invocation_sequences = [record.get("sequence") for record in invocation_records if isinstance(record, dict)]
        invocation_ids = [record.get("invoke_id") for record in invocation_records if isinstance(record, dict)]
        invocation_count = math.ceil(DATASETS["invocations"]["arrival_rate"] * RUN_SECONDS)
        if invocation_sequences != list(range(invocation_count)):
            raise AcceptanceError("foreground HTTP driver invocation sequence is incomplete or reordered")
        if any(not isinstance(identity, str) or not identity for identity in invocation_ids):
            raise AcceptanceError("foreground HTTP driver returned an invalid invocation ID")
        if len(set(invocation_ids)) != invocation_count:
            raise AcceptanceError("foreground HTTP driver returned duplicate invocation IDs")
        added["invocations"] = [row.get("invoke_id") for row in invocation_records if isinstance(row, dict)]
        expected_counts = {
            name: math.ceil(definition["arrival_rate"] * RUN_SECONDS)
            for name, definition in DATASETS.items()
        }
        if len(added["invocations"]) != expected_counts["invocations"]:
            raise AcceptanceError("production proxy request count did not match the configured invocation rate")
        retry_count = expected_counts["pool_upstream_attempts"] - expected_counts["invocations"]
        attempt_states = observed_attempt_states(
            main_db,
            invocation_records,
            retry_count,
            phase_started,
        )
        added["pool_upstream_attempts"] = [row["attempt_public_id"] for row in attempt_states]
        if len(added["pool_upstream_attempts"]) != expected_counts["pool_upstream_attempts"]:
            raise AcceptanceError("production upstream attempt count did not match the configured rate")
        task_evidence = wait_for_task_evidence(
            main_db,
            maintenance_db,
            cutoff,
            {name: len(added[name]) for name in DATASETS},
            phase_started,
            version,
        )
        post_buckets, post_text, post_time = scrape_metrics(container, phase_started)
        write_scrapes.append({"observed_at": post_time, "scope": "after-drain", "bucket_counts": post_buckets})
        write_buckets = histogram_delta(pre_buckets, post_buckets)
        (evidence_dir / "metrics").mkdir(parents=True, exist_ok=True)
        (evidence_dir / "metrics" / f"{version}-{index}-pre.prom").write_text(pre_text)
        (evidence_dir / "metrics" / f"{version}-{index}-post.prom").write_text(post_text)
        containers.cleanup(
            container,
            phase_started,
            cleanup_reserve_seconds=15,
            log_path=evidence_dir / "logs" / f"{version}-{index}.log",
        )
        container_stopped = True
        oracle = verify_trial(
            main_db,
            trial_dir / "archives",
            added,
            attempt_states,
            template / "codex_vibe_monitor.db",
            cutoff,
            root / f"oracle-{version}-{index}",
            budget_check=lambda: remaining_budget(phase_started),
        )
    except BaseException as error:
        trial_error = error
    finally:
        trial_container_names = containers.names() - prior_container_names
        if container in trial_container_names and not container_stopped:
            try:
                containers.cleanup(
                    container,
                    phase_started,
                    cleanup_reserve_seconds=12,
                    log_path=evidence_dir / "logs" / f"{version}-{index}.log",
                )
            except BaseException as error:
                cleanup_errors.append(error)
        if input_end is None:
            input_end = utc_now()
        trial_cleanup, retry_errors = cleanup_path_if_safe(
            trial_dir,
            root,
            containers,
            phase_started,
            cleanup_reserve_seconds=5,
        )
        cleanup_errors.extend(retry_errors)
        if not trial_cleanup["removed"]:
            cleanup_errors.append(AcceptanceError(
                "trial data was preserved because owned container cleanup was not confirmed"
            ))
        cleanup_end = utc_now()
    if cleanup_errors:
        if trial_error is None:
            trial_error = cleanup_errors[0]
        elif hasattr(trial_error, "add_note"):
            trial_error.add_note("trial cleanup failed: " + "; ".join(str(error) for error in cleanup_errors))

    if trial_error is not None:
        partial = {
            "run_id": RUN_ID,
            "version": version,
            "trial_index": index,
            "error": {"type": type(trial_error).__name__, "message": str(trial_error)},
            "input_counts": {name: len(added[name]) for name in DATASETS},
            "added_ids": added,
            "attempt_states": attempt_states,
            "foreground_driver_cleanup": driver_cleanup,
            "owned_container_cleanup": containers.records(trial_container_names),
            "latency_samples_ms": input_samples,
            "foreground_workload": foreground_evidence,
            "task_runs": task_evidence,
            "write_hold_samples": write_scrapes,
            "write_hold_buckets": write_buckets,
        }
        if not trial_cleanup["removed"]:
            partial["cleanup_recovery"] = trial_cleanup["recovery"]
        try:
            write_json(evidence_dir / "partial-trial.json", partial)
        except BaseException as error:
            if hasattr(trial_error, "add_note"):
                trial_error.add_note(f"could not persist partial trial evidence: {error}")

    if trial_error is not None:
        raise trial_error
    if input_start is None or input_end is None or cleanup_end is None or write_buckets is None or oracle is None:
        raise AcceptanceError("trial ended without complete measured windows")
    full_started = parse_time(trial_start)
    input_started = parse_time(input_start)
    input_finished = parse_time(input_end)
    full_finished = parse_time(cleanup_end)
    input_duration = (input_finished - input_started).total_seconds()
    full_duration = (full_finished - full_started).total_seconds()
    for name, result in oracle["datasets"].items():
        input_count = len(added[name])
        result["sustained_window_input_seconds"] = input_duration
        result["sustained_window_seconds"] = full_duration
        result["sustained_convergence_factor"] = (
            (result["converted_rows"] / full_duration)
            / (input_count / input_duration)
        )
    oracle["datasets"]["invocations"]["added_ids_sha256"] = sha256_bytes("\n".join(added["invocations"]).encode())
    oracle["datasets"]["pool_upstream_attempts"]["added_ids_sha256"] = sha256_bytes("\n".join(added["pool_upstream_attempts"]).encode())
    return {
        "trial_index": index,
        "source_sha": image["source_sha"],
        "image_id": image["image_id"],
        "input_fingerprint": fingerprint,
        "cutoff": cutoff,
        "container_id": container_id,
        "application_container_cleanup": {
            "container_name": container,
            "container_id": containers.record(container)["container_id"],
            "status": containers.record(container)["status"],
            "identity_labels": containers.record(container)["verified_identity_labels"],
        },
        "attempt_states": attempt_states,
        "foreground_driver_cleanup": driver_cleanup,
        "window": {
            "setup_started_at": trial_start,
            "input_started_at": input_start,
            "input_finished_at": input_end,
            "cleanup_finished_at": cleanup_end,
            "input_duration_seconds": input_duration,
            "full_duration_seconds": full_duration,
            "gap_before_seconds": 0.0 if previous_end is None else (full_started - parse_time(previous_end)).total_seconds(),
            "wall_duration_seconds": time.monotonic() - wall_start,
        },
        "input_counts": {name: len(added[name]) for name in DATASETS},
        "added_ids": added,
        "latency_samples_ms": input_samples,
        "foreground_workload": foreground_evidence,
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
        "run_id": RUN_ID,
        "test_phase": {"started_at": phase_start_iso},
        "comparison_window": {"started_at": phase_start_iso},
        "trial_execution_order": [],
        "contract": {},
        "versions": {},
    }
    root: Path | None = None
    failure: BaseException | None = None
    owned = OwnedContainerLedger()
    try:
        candidate_sha = os.environ.get("CANDIDATE_SHA", "")
        if not re.fullmatch(r"[0-9a-f]{40}", candidate_sha):
            raise AcceptanceError("CANDIDATE_SHA must be the exact 40-character lowercase commit SHA")
        if candidate_sha == BASELINE_SHA:
            raise AcceptanceError("candidate SHA must differ from the baseline SHA")
        cutoff = shanghai_cutoff()
        tags = {"baseline": f"retention-acceptance:{BASELINE_SHA}", "candidate": f"retention-acceptance:{candidate_sha}"}
        images = {
            "baseline": docker_image_identity(tags["baseline"], BASELINE_SHA, phase_started),
            "candidate": docker_image_identity(tags["candidate"], candidate_sha, phase_started),
        }
        driver_image = worker_image_identity(phase_started)
        host = host_identity(phase_started)
        evidence["contract"] = {
            "baseline_sha": BASELINE_SHA,
            "candidate_sha": candidate_sha,
            "driver_image_id": driver_image["image_id"],
            "driver_image_ref": driver_image["image_ref"],
            "driver_image_digest": driver_image["digest"],
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
            templates[version], fingerprints[version] = prepare_template(
                tags[version], version, root, cutoff, phase_started, owned
            )
        if fingerprints["baseline"] != fingerprints["candidate"]:
            raise AcceptanceError("baseline and candidate fixture fingerprints differ")
        fingerprint = fingerprints["baseline"]
        evidence["contract"]["input_fingerprint"] = fingerprint
        for version in ("baseline", "candidate"):
            evidence["versions"][version] = {
                "image": images[version],
                "host": host,
                "resource_limits": {"cpus": CPU_LIMIT, "memory_bytes": MEMORY_BYTES},
                "trial_span": {},
                "trials": [],
            }
        previous_end: str | None = None
        for step in TRIAL_EXECUTION_ORDER:
            version = step["version"]
            index = step["trial_index"]
            group = evidence["versions"][version]
            if not group["trial_span"]:
                group["trial_span"]["started_at"] = utc_now()
            remaining_budget(phase_started)
            trial, end_at = run_trial(
                version,
                index,
                tags[version],
                images[version],
                driver_image["image_id"],
                templates[version],
                fingerprint,
                root,
                evidence_dir,
                cutoff,
                previous_end,
                phase_started,
                owned,
            )
            previous_end = end_at
            group["trials"].append(trial)
            group["trial_span"]["finished_at"] = end_at
            group["trial_span"]["elapsed_seconds"] = (
                parse_time(end_at) - parse_time(group["trial_span"]["started_at"])
            ).total_seconds()
            evidence["trial_execution_order"].append(step)
            write_json(evidence_dir / "evidence.partial.json", evidence)
        if shanghai_cutoff() != cutoff:
            raise AcceptanceError("Shanghai retention cutoff changed before test-phase cleanup completed")
    except BaseException as error:
        failure = error
        evidence["failure"] = {"message": str(error), "type": type(error).__name__}
        (evidence_dir / "failure.traceback.txt").write_text(traceback.format_exc())
    finally:
        if root is not None:
            root_cleanup, root_cleanup_errors = cleanup_path_if_safe(
                root,
                root,
                owned,
                phase_started,
                cleanup_reserve_seconds=8,
            )
            if not root_cleanup["removed"]:
                evidence["cleanup_recovery"] = root_cleanup["recovery"]
                cleanup_error = AcceptanceError(
                    "acceptance root was preserved because owned container cleanup was not confirmed"
                )
                if failure is None:
                    failure = cleanup_error
                elif hasattr(failure, "add_note"):
                    failure.add_note(str(cleanup_error))
            elif root_cleanup_errors and failure is None:
                failure = root_cleanup_errors[0]
        evidence["owned_container_cleanup"] = owned.records()
        if failure is not None:
            evidence["failure"] = {"message": str(failure), "type": type(failure).__name__}
            (evidence_dir / "failure.traceback.txt").write_text(
                "".join(traceback.format_exception(type(failure), failure, failure.__traceback__))
            )
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
