"""Bounded, read-only CPU attribution evidence for acceptance windows."""
from __future__ import annotations

import math
import os
from pathlib import Path
import re


ATTRIBUTION_SCHEMA_VERSION = 1
SAMPLE_INTERVAL_SECONDS = 10
MAX_FILE_BYTES = 16 * 1024
HOST_COUNTER_FIELDS = (
    "user",
    "nice",
    "system",
    "idle",
    "iowait",
    "irq",
    "softirq",
    "steal",
    "guest",
    "guestNice",
)
APP_COUNTER_FIELDS = (
    "usage_usec",
    "user_usec",
    "system_usec",
    "nr_periods",
    "nr_throttled",
    "throttled_usec",
)


class CpuAttributionError(ValueError):
    """A fixed, safe error code for missing or invalid attribution evidence."""

    def __init__(self, code: str, source: str):
        self.code = code
        self.source = source
        super().__init__(f"{code}:{source}")


def _fail(code: str, source: str) -> None:
    raise CpuAttributionError(code, source)


def _read_text(path: Path, source: str) -> str:
    try:
        with path.open("rb") as stream:
            payload = stream.read(MAX_FILE_BYTES + 1)
    except FileNotFoundError as error:
        raise CpuAttributionError("cpu_attribution_unsupported", source) from error
    except OSError as error:
        raise CpuAttributionError("cpu_read_failed", source) from error
    if len(payload) > MAX_FILE_BYTES:
        _fail("cpu_read_failed", source)
    try:
        return payload.decode("utf-8")
    except UnicodeDecodeError as error:
        raise CpuAttributionError("cpu_read_failed", source) from error


def parse_cpu_list(value: str, source: str = "effective_affinity") -> list[int]:
    if not isinstance(value, str) or not value or len(value) > 256:
        _fail("cpu_read_failed", source)
    result: set[int] = set()
    for part in value.split(","):
        if re.fullmatch(r"\d+", part):
            start = end = int(part)
        else:
            match = re.fullmatch(r"(\d+)-(\d+)", part)
            if match is None:
                _fail("cpu_read_failed", source)
            start, end = (int(item) for item in match.groups())
        if start > end or end > 4096:
            _fail("cpu_read_failed", source)
        result.update(range(start, end + 1))
    if not result:
        _fail("cpu_read_failed", source)
    return sorted(result)


def parse_proc_stat(text: str, cpus: list[int], source: str = "/proc/stat") -> dict[str, dict[str, int]]:
    requested = sorted(set(int(cpu) for cpu in cpus))
    rows: dict[str, dict[str, int]] = {}
    for line in text.splitlines():
        match = re.fullmatch(r"cpu(\d+)\s+(.+)", line.strip())
        if match is None:
            continue
        cpu = int(match.group(1))
        if cpu not in requested:
            continue
        if str(cpu) in rows:
            _fail("cpu_read_failed", source)
        try:
            fields = [int(value) for value in match.group(2).split()]
        except ValueError as error:
            raise CpuAttributionError("cpu_read_failed", source) from error
        if len(fields) < len(HOST_COUNTER_FIELDS) or any(value < 0 for value in fields):
            _fail("cpu_read_failed", source)
        row = {name: fields[index] for index, name in enumerate(HOST_COUNTER_FIELDS) if index < len(fields)}
        row.update(
            rawTicks=fields,
            busyTicks=sum(fields[index] for index in (0, 1, 2, 5, 6, 7)),
            idleTicks=fields[3],
            iowaitTicks=fields[4],
            stealTicks=fields[7],
        )
        rows[str(cpu)] = row
    if any(str(cpu) not in rows for cpu in requested):
        _fail("cpu_read_failed", source)
    return rows


def parse_cpu_stat(text: str, source: str = "app.cpuStat") -> dict[str, int]:
    values: dict[str, int] = {}
    for line in text.splitlines():
        parts = line.split()
        if len(parts) != 2 or not re.fullmatch(r"[a-z_]+", parts[0]):
            _fail("cpu_read_failed", source)
        key = parts[0]
        if key in values:
            _fail("cpu_read_failed", source)
        try:
            value = int(parts[1])
        except ValueError as error:
            raise CpuAttributionError("cpu_read_failed", source) from error
        if value < 0:
            _fail("cpu_read_failed", source)
        values[key] = value
    if any(key not in values for key in APP_COUNTER_FIELDS):
        _fail("cpu_attribution_unsupported", source)
    return {key: values[key] for key in sorted(values)}


def _parse_pressure_line(line: str, source: str) -> dict[str, float | int]:
    parts = line.split()
    if len(parts) < 5:
        _fail("cpu_read_failed", source)
    values: dict[str, float | int] = {}
    for part in parts[1:]:
        key, separator, raw = part.partition("=")
        if not separator or key in values:
            _fail("cpu_read_failed", source)
        try:
            value: float | int = int(raw) if key == "total" else float(raw)
        except ValueError as error:
            raise CpuAttributionError("cpu_read_failed", source) from error
        if not math.isfinite(value) or value < 0 or (key != "total" and value > 100):
            _fail("cpu_read_failed", source)
        values[key] = value
    if not {"avg10", "avg60", "avg300", "total"}.issubset(values):
        _fail("cpu_attribution_unsupported", source)
    return values


def parse_cpu_pressure(text: str, source: str = "app.cpuPressure") -> dict[str, dict[str, float | int]]:
    result: dict[str, dict[str, float | int]] = {}
    for line in text.splitlines():
        parts = line.split(maxsplit=1)
        if len(parts) != 2 or parts[0] not in {"some", "full"} or parts[0] in result:
            _fail("cpu_read_failed", source)
        result[parts[0]] = _parse_pressure_line(line, source)
    if set(result) != {"some", "full"}:
        _fail("cpu_attribution_unsupported", source)
    return result


def parse_cpu_max(text: str, source: str = "app.cpuMax") -> dict[str, int | str]:
    parts = text.split()
    if len(parts) != 2:
        _fail("cpu_read_failed", source)
    quota: int | str
    if parts[0] == "max":
        quota = "max"
    elif re.fullmatch(r"\d+", parts[0]):
        quota = int(parts[0])
    else:
        _fail("cpu_read_failed", source)
    if not re.fullmatch(r"\d+", parts[1]) or int(parts[1]) <= 0:
        _fail("cpu_read_failed", source)
    return {"quota": quota, "periodUsec": int(parts[1])}


def parse_process_stat(text: str, uptime_text: str, source: str) -> dict[str, int | float]:
    separator = text.rfind(") ")
    if separator <= 0:
        _fail("cpu_read_failed", source)
    fields = text[separator + 2 :].split()
    if len(fields) < 20:
        _fail("cpu_read_failed", source)
    try:
        start_ticks = int(fields[19])
        user_ticks = int(fields[11])
        system_ticks = int(fields[12])
        uptime = float(uptime_text.split()[0])
    except (IndexError, ValueError) as error:
        raise CpuAttributionError("cpu_read_failed", source) from error
    if min(start_ticks, user_ticks, system_ticks) < 0 or not math.isfinite(uptime):
        _fail("cpu_read_failed", source)
    runtime = uptime - start_ticks / os.sysconf(os.sysconf_names["SC_CLK_TCK"])
    if runtime < 0 or not math.isfinite(runtime):
        _fail("cpu_read_failed", source)
    return {
        "startTimeTicks": start_ticks,
        "userTicks": user_ticks,
        "systemTicks": system_ticks,
        "runtimeSeconds": runtime,
    }


def parse_effective_affinity(text: str, source: str) -> list[int]:
    match = re.search(r"^Cpus_allowed_list:\s*(\S+)\s*$", text, re.M)
    if match is None:
        _fail("cpu_attribution_unsupported", source)
    return parse_cpu_list(match.group(1), source)


def parse_cgroup_v2_path(text: str, source: str = "process.cgroup") -> str:
    paths: list[str] = []
    for line in text.splitlines():
        parts = line.split(":", 2)
        if len(parts) == 3 and parts[0] == "0":
            paths.append(parts[2])
    if len(paths) != 1 or not paths[0].startswith("/") or ".." in Path(paths[0]).parts:
        _fail("cpu_attribution_unsupported", source)
    if paths[0] == "/":
        _fail("cpu_attribution_unsupported", source)
    return paths[0]


def container_cgroup_matches(container_id: str, cgroup_path: str) -> bool:
    parts = cgroup_path.strip("/").split("/")
    return container_id in parts or f"docker-{container_id}.scope" in parts


def _counter_delta(before: dict[str, int], after: dict[str, int], source: str) -> dict[str, int]:
    result: dict[str, int] = {}
    for key, old in before.items():
        if key not in after or not isinstance(after[key], int):
            _fail("cpu_read_failed", source)
        delta = after[key] - old
        if delta < 0:
            _fail("cpu_counter_reset", source)
        result[key] = delta
    return result


def _host_counter_delta(before: dict[str, object], after: dict[str, object], source: str) -> dict[str, int]:
    try:
        before_counters = {key: before[key] for key in HOST_COUNTER_FIELDS}
        after_counters = {key: after[key] for key in HOST_COUNTER_FIELDS}
    except KeyError as error:
        raise CpuAttributionError("cpu_read_failed", source) from error
    return _counter_delta(before_counters, after_counters, source)


class PreparedCpuAttribution:
    """Resolve container process/cgroup files once, then sample only those paths."""

    def __init__(
        self,
        allowed_cpus: list[int],
        measurement_cpu: str,
        containers: list[dict[str, object]],
        *,
        proc_root: Path = Path("/proc"),
        cgroup_root: Path = Path("/sys/fs/cgroup"),
    ):
        self.proc_root = Path(proc_root)
        self.cgroup_root = Path(cgroup_root)
        self.allowed_cpus = sorted({int(cpu) for cpu in allowed_cpus})
        self.measurement_cpu = str(measurement_cpu)
        if not self.allowed_cpus or int(self.measurement_cpu) not in self.allowed_cpus:
            _fail("cpu_metadata_mismatch", "runner.cpuAffinity")
        self._containers: dict[str, dict[str, object]] = {}
        self._cgroup_paths: dict[str, Path] = {}
        self._app_cgroup_files: dict[str, Path] = {}
        self._prepared_cpu_max: dict[str, int | str] | None = None
        self._first: dict[str, object] | None = None
        self._last: dict[str, object] | None = None
        self._last_usage: int | None = None
        self._errors: set[str] = set()
        self._prepare(containers)

    def _resolve_cgroup(self, relative: str, source: str) -> Path:
        root = self.cgroup_root.resolve()
        try:
            candidate = (root / relative.lstrip("/")).resolve(strict=True)
        except FileNotFoundError as error:
            raise CpuAttributionError("cpu_attribution_unsupported", source) from error
        except OSError as error:
            raise CpuAttributionError("cpu_read_failed", source) from error
        try:
            candidate.relative_to(root)
        except ValueError as error:
            raise CpuAttributionError("cpu_attribution_unsupported", source) from error
        return candidate

    def _read_process(self, service: str, pid: int, expected: dict[str, object] | None = None) -> dict[str, object]:
        process_root = self.proc_root / str(pid)
        try:
            stat = parse_process_stat(
                _read_text(process_root / "stat", f"{service}.processStat"),
                _read_text(self.proc_root / "uptime", "proc.uptime"),
                f"{service}.processStat",
            )
            affinity = parse_effective_affinity(
                _read_text(process_root / "status", f"{service}.affinity"),
                f"{service}.affinity",
            )
            cgroup = parse_cgroup_v2_path(
                _read_text(process_root / "cgroup", f"{service}.cgroup"),
                f"{service}.cgroup",
            )
        except CpuAttributionError as error:
            if expected is not None and error.code in {"cpu_attribution_unsupported", "cpu_read_failed"}:
                raise CpuAttributionError("pid_replaced", f"{service}.pid") from error
            raise
        if expected is not None:
            if stat["startTimeTicks"] != expected["startTimeTicks"] or cgroup != expected["cgroupPath"]:
                _fail("pid_replaced", f"{service}.pid")
            if not container_cgroup_matches(str(expected["containerId"]), cgroup):
                _fail("cpu_metadata_mismatch", f"{service}.containerId")
        return {**stat, "pid": pid, "effectiveAffinity": affinity, "cgroupPath": cgroup}

    def _prepare(self, containers: list[dict[str, object]]) -> None:
        if not isinstance(containers, list) or len(containers) == 0:
            _fail("cpu_read_failed", "containers")
        for container in containers:
            service = container.get("service")
            pid = container.get("pid")
            container_id = container.get("containerId")
            if not isinstance(service, str) or not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", service):
                _fail("cpu_read_failed", "containers")
            if service in self._containers:
                _fail("cpu_read_failed", "containers")
            if isinstance(pid, bool) or not isinstance(pid, int) or pid <= 0:
                _fail("cpu_read_failed", f"{service}.pid")
            if not isinstance(container_id, str) or not re.fullmatch(r"[a-f0-9]{64}", container_id):
                _fail("cpu_read_failed", f"{service}.containerId")
            info = self._read_process(service, pid)
            if not container_cgroup_matches(container_id, str(info["cgroupPath"])):
                _fail("cpu_metadata_mismatch", f"{service}.containerId")
            self._containers[service] = {
                "service": service,
                "containerId": container_id,
                **info,
            }
            self._cgroup_paths[service] = self._resolve_cgroup(info["cgroupPath"], f"{service}.cgroup")
        if "app" not in self._containers:
            _fail("cpu_read_failed", "app.container")
        if any(
            not set(info["effectiveAffinity"]).issubset(self.allowed_cpus)
            for info in self._containers.values()
        ):
            _fail("cpu_metadata_mismatch", "container.effectiveAffinity")
        app = self._containers["app"]
        if app["effectiveAffinity"] != [int(self.measurement_cpu)]:
            _fail("cpu_metadata_mismatch", "app.effectiveAffinity")
        app_cgroup = self._cgroup_paths["app"]
        self._app_cgroup_files = {
            "cpuStat": app_cgroup / "cpu.stat",
            "cpuPressure": app_cgroup / "cpu.pressure",
            "cpuMax": app_cgroup / "cpu.max",
        }
        for key, path in self._app_cgroup_files.items():
            if not path.is_file():
                _fail("cpu_attribution_unsupported", f"app.{key}")
        parse_cpu_stat(_read_text(self._app_cgroup_files["cpuStat"], "app.cpuStat"))
        parse_cpu_pressure(_read_text(self._app_cgroup_files["cpuPressure"], "app.cpuPressure"))
        cpu_max = parse_cpu_max(_read_text(self._app_cgroup_files["cpuMax"], "app.cpuMax"))
        if cpu_max["quota"] == "max" or cpu_max["quota"] != cpu_max["periodUsec"]:
            _fail("cpu_metadata_mismatch", "app.cpuMax")
        self._prepared_cpu_max = cpu_max

    def _read_app_cpu_max(self) -> dict[str, int | str]:
        current = parse_cpu_max(_read_text(self._app_cgroup_files["cpuMax"], "app.cpuMax"))
        if current != self._prepared_cpu_max:
            _fail("cpu_metadata_mismatch", "app.cpuMax")
        return current

    def metadata(self) -> dict[str, object]:
        containers = []
        for service in sorted(self._containers):
            info = self._containers[service]
            row = {
                "service": service,
                "containerId": info["containerId"],
                "pid": info["pid"],
                "startTimeTicks": info["startTimeTicks"],
                "effectiveAffinity": info["effectiveAffinity"],
                "cgroupPath": info["cgroupPath"],
            }
            if service == "app":
                row["cgroup"] = {
                    "version": "v2",
                    "relativePath": info["cgroupPath"],
                    "cpuMax": self._read_app_cpu_max(),
                }
            containers.append(row)
        return {
            "schemaVersion": ATTRIBUTION_SCHEMA_VERSION,
            "status": "prepared",
            "sampleIntervalSeconds": SAMPLE_INTERVAL_SECONDS,
            "measurementCpu": self.measurement_cpu,
            "allowedCpus": self.allowed_cpus,
            "containers": containers,
        }

    def _current(self) -> dict[str, object]:
        errors: list[dict[str, str]] = []
        host_cpu: dict[str, dict[str, int]] | None = None
        processes: dict[str, dict[str, object]] = {}
        app_stat: dict[str, int] | None = None
        app_pressure: dict[str, dict[str, float | int]] | None = None
        app_max: dict[str, int | str] | None = None
        try:
            host_cpu = parse_proc_stat(
                _read_text(self.proc_root / "stat", "proc.stat"),
                self.allowed_cpus,
            )
        except CpuAttributionError as error:
            errors.append({"code": error.code, "source": error.source})
        for service, info in self._containers.items():
            try:
                process = self._read_process(service, int(info["pid"]), expected=info)
                processes[service] = {"containerId": info["containerId"], **process}
            except CpuAttributionError as error:
                errors.append({"code": error.code, "source": error.source})
        try:
            app_stat = parse_cpu_stat(_read_text(self._app_cgroup_files["cpuStat"], "app.cpuStat"))
            app_pressure = parse_cpu_pressure(_read_text(self._app_cgroup_files["cpuPressure"], "app.cpuPressure"))
            app_max = self._read_app_cpu_max()
        except CpuAttributionError as error:
            errors.append({"code": error.code, "source": error.source})
        snapshot: dict[str, object] = {
            "schemaVersion": ATTRIBUTION_SCHEMA_VERSION,
            "status": "passed" if not errors else "unavailable",
            "sampleIntervalSeconds": SAMPLE_INTERVAL_SECONDS,
            "measurementCpu": self.measurement_cpu,
            "allowedCpus": self.allowed_cpus,
            "hostCpu": host_cpu,
            "containers": processes,
            "counterDelta": None,
            "counterDeltaStatus": "baseline" if self._last is None else "unavailable",
            "app": {
                "pid": self._containers["app"]["pid"],
                "startTimeTicks": self._containers["app"]["startTimeTicks"],
                "effectiveAffinity": processes.get("app", {}).get("effectiveAffinity"),
                "processRuntimeSeconds": processes.get("app", {}).get("runtimeSeconds"),
                "cpuStat": app_stat,
                "cpuPressure": app_pressure,
                "cpuMax": app_max,
            },
        }
        if not errors and self._last is not None:
            try:
                snapshot["counterDelta"] = {
                    "hostCpu": {
                        str(cpu): _host_counter_delta(
                            self._last["hostCpu"][str(cpu)],
                            host_cpu[str(cpu)],
                            "proc.stat",
                        )
                        for cpu in self.allowed_cpus
                    },
                    "appCpuStat": _counter_delta(self._last["app"]["cpuStat"], app_stat, "app.cpuStat"),
                }
                snapshot["counterDeltaStatus"] = "delta"
            except (CpuAttributionError, KeyError, TypeError) as error:
                if isinstance(error, CpuAttributionError):
                    errors.append({"code": error.code, "source": error.source})
                else:
                    errors.append({"code": "cpu_read_failed", "source": "counterDelta"})
                snapshot["status"] = "unavailable"
        if errors:
            snapshot["status"] = "unavailable"
            snapshot["counterDeltaStatus"] = "unavailable"
            snapshot["errorCodes"] = sorted({item["code"] for item in errors})
            snapshot["errors"] = errors
        else:
            snapshot["errorCodes"] = []
            snapshot["errors"] = []
        return snapshot

    def sample(self) -> dict[str, object]:
        snapshot = self._current()
        self._last = snapshot
        if self._first is None:
            self._first = snapshot
        self._errors.update(snapshot["errorCodes"])
        return snapshot

    def begin_window(self) -> None:
        self._first = None
        self._last = None
        self._last_usage = None
        self._errors.clear()

    def cpu_usage_usec(self) -> int:
        current = self._read_process("app", int(self._containers["app"]["pid"]), expected=self._containers["app"])
        self._read_app_cpu_max()
        stat = parse_cpu_stat(_read_text(self._app_cgroup_files["cpuStat"], "app.cpuStat"))
        usage = stat["usage_usec"]
        if self._last_usage is not None and usage < self._last_usage:
            self._errors.add("cpu_counter_reset")
            _fail("cpu_counter_reset", "app.cpuStat")
        self._last_usage = usage
        if current["effectiveAffinity"] != [int(self.measurement_cpu)]:
            self._errors.add("cpu_metadata_mismatch")
            _fail("cpu_metadata_mismatch", "app.effectiveAffinity")
        return usage

    def window_summary(self) -> dict[str, object]:
        summary: dict[str, object] = {
            "schemaVersion": ATTRIBUTION_SCHEMA_VERSION,
            "status": "unavailable" if self._errors else "passed",
            "sampleIntervalSeconds": SAMPLE_INTERVAL_SECONDS,
            "measurementCpu": self.measurement_cpu,
            "allowedCpus": self.allowed_cpus,
            "errorCodes": sorted(self._errors),
        }
        if not self._errors and self._first is not None and self._last is not None:
            try:
                summary["counterDelta"] = {
                    "hostCpu": {
                        str(cpu): _host_counter_delta(
                            self._first["hostCpu"][str(cpu)],
                            self._last["hostCpu"][str(cpu)],
                            "proc.stat",
                        )
                        for cpu in self.allowed_cpus
                    },
                    "appCpuStat": _counter_delta(
                        self._first["app"]["cpuStat"],
                        self._last["app"]["cpuStat"],
                        "app.cpuStat",
                    ),
                }
            except (CpuAttributionError, KeyError, TypeError) as error:
                summary["status"] = "unavailable"
                summary["errorCodes"] = sorted(set(summary["errorCodes"]) | {getattr(error, "code", "cpu_read_failed")})
        return summary
