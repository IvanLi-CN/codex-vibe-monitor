"""Bounded synthetic CPU evidence; never a performance certificate."""
from contextlib import contextmanager
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import re
import threading
import time

MAX_SAMPLES = 330
MAX_BYTES = 512 * 1024 * 1024
METRICS = {
    "cvm_process_cpu_seconds_total", "cvm_proxy_invocations_total",
    "cvm_proxy_upstream_attempts_total", "cvm_sqlite_written_rows_total",
    "cvm_sqlite_written_bytes_total", "cvm_sqlite_flush_attempts_total",
    "cvm_sqlite_retries_total", "cvm_sqlite_defers_total",
    "cvm_projection_builds_total", "cvm_projection_live_db_reads_total",
    "cvm_projection_publications_total", "cvm_projection_reconciliations_total",
    "cvm_task_run_duration_seconds_count", "cvm_sqlite_batch_execute_duration_seconds_count",
    "cvm_http_body_duration_seconds_count", "cvm_sqlite_pending_items",
}


def fingerprint(root):
    digest = hashlib.sha256()
    count = size = 0
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise ValueError("synthetic state contains an alias")
        if not path.is_file():
            continue
        count += 1
        size += path.stat().st_size
        if count > 10000 or size > MAX_BYTES:
            raise ValueError("synthetic state exceeds fingerprint bound")
        relative = path.relative_to(root).as_posix().encode()
        digest.update(len(relative).to_bytes(8, "big") + relative)
        digest.update(path.stat().st_size.to_bytes(8, "big"))
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
        digest.update(b"\0")
    return {"sha256": digest.hexdigest(), "files": count, "bytes": size}


def metric_summary(text):
    if len(text.encode()) > 1024 * 1024:
        raise ValueError("metrics exceed diagnostic response bound")
    values = {}
    for line in text.splitlines():
        match = re.fullmatch(r'([a-zA-Z_:][a-zA-Z0-9_:]*)(?:\{.*\})?\s+(\S+)(?:\s+\d+)?', line)
        if match and match[1] in METRICS:
            value = float(match[2])
            if not math.isfinite(value) or value < 0:
                raise ValueError("invalid diagnostic metric")
            values[match[1]] = values.get(match[1], 0) + value
    return {"status": "observed", "values": values, "missing": sorted(METRICS - values.keys())}


def unknown_read(operation):
    try:
        return {"status": "observed", "values": operation()}
    except (OSError, ValueError, IndexError, KeyError, AttributeError) as error:
        return {"status": "unknown", "reason": type(error).__name__}


def process_stat(text):
    # comm can contain whitespace and ')'; numeric fields begin after the last ')'.
    values = text[text.rindex(")") + 2:].split()
    result = {"userTicks": int(values[11]), "systemTicks": int(values[12]),
            "startTicks": int(values[19]), "threads": int(values[17]),
            "processor": int(values[36]), "minorFaults": int(values[7]), "majorFaults": int(values[9])}
    if any(value < 0 for value in result.values()):
        raise ValueError("invalid process CPU fields")
    return result


def key_values(text):
    return {line.split()[0].rstrip(":"): int(line.split()[1]) for line in text.splitlines()}


class Probe:
    def __init__(self, pid, container, proc=Path("/proc"), cgroups=Path("/sys/fs/cgroup"), cpus=Path("/sys/devices/system/cpu")):
        if pid <= 0 or not re.fullmatch(r"[a-f0-9]{64}", container):
            raise ValueError("diagnostic target must be an inspected live container")
        self.pid, self.proc, self.cpus = pid, proc, cpus
        self.start = process_stat((proc / str(pid) / "stat").read_text())["startTicks"]
        relative = next(line[3:] for line in (proc / str(pid) / "cgroup").read_text().splitlines() if line.startswith("0::"))
        self.group = (cgroups / relative.lstrip("/")).resolve(strict=True)
        if not self.group.is_relative_to(cgroups.resolve()) or container not in relative:
            raise ValueError("cgroup does not belong to the inspected container")

    def process(self):
        result = process_stat((self.proc / str(self.pid) / "stat").read_text())
        if result["startTicks"] != self.start:
            raise ValueError("diagnostic PID identity changed")
        return result

    def switches(self):
        tasks = list((self.proc / str(self.pid) / "task").iterdir())
        if len(tasks) > 512:
            raise ValueError("diagnostic thread bound exceeded")
        totals = {"voluntary": 0, "involuntary": 0, "threadsRead": 0, "threadsDisappeared": 0}
        for task in tasks:
            try:
                fields = dict(line.split(":", 1) for line in (task / "status").read_text().splitlines() if ":" in line)
                totals["voluntary"] += int(fields["voluntary_ctxt_switches"])
                totals["involuntary"] += int(fields["nonvoluntary_ctxt_switches"])
                totals["threadsRead"] += 1
            except FileNotFoundError:
                totals["threadsDisappeared"] += 1
        return totals

    def frequencies(self):
        affinity = sorted(os.sched_getaffinity(0))
        result = []
        for cpu in affinity[:64]:
            path = self.cpus / f"cpu{cpu}" / "cpufreq/scaling_cur_freq"
            result.append({"cpu": cpu, **unknown_read(lambda p=path: {"kHz": int(p.read_text())})})
        return {"cpus": result, "truncated": len(affinity) > 64}

    def host_ticks(self):
        values = (self.proc / "stat").read_text().splitlines()[0].split()[1:]
        if len(values) < 8 or any(int(value) < 0 for value in values):
            raise ValueError("incomplete host CPU fields")
        return {name: int(value) for name, value in zip(
            ["user", "nice", "system", "idle", "iowait", "irq", "softirq", "steal", "guest", "guestNice"], values)}

    def cgroup(self):
        result = key_values((self.group / "cpu.stat").read_text())
        required = {"usage_usec", "user_usec", "system_usec", "nr_periods", "nr_throttled", "throttled_usec"}
        if not required.issubset(result) or any(value < 0 for value in result.values()):
            raise ValueError("incomplete cgroup CPU fields")
        return result

    def sample(self):
        process = unknown_read(self.process)
        valid = process["status"] == "observed"
        unknown = {"status": "unknown", "reason": "target_identity_unavailable"}
        result = {"cgroup": unknown_read(self.cgroup), "process": process,
                  "contextSwitches": unknown_read(self.switches) if valid else unknown,
                  "processIO": unknown_read(lambda: key_values((self.proc / str(self.pid) / "io").read_text())) if valid else unknown,
                  "hostTicks": unknown_read(self.host_ticks), "frequency": unknown_read(self.frequencies)}
        # Recheck after reading the remaining PID-bound fields to detect reuse
        # during this sample; never retain evidence about a replacement process.
        if valid and unknown_read(self.process)["status"] != "observed":
            result.update(process=unknown, contextSwitches=unknown, processIO=unknown)
        return result


class Observer:
    def __init__(self, root, window, probe):
        self.root, self.window, self.probe = root, window, probe
        self.stop = threading.Event()
        self.worker = None
        self.errors = set()
        self.count = 0
        self.started = time.monotonic()
        self.previous = self.started
        self.max_gap = 0
        self.output = None

    def sample(self):
        if self.count >= MAX_SAMPLES:
            raise ValueError("diagnostic sample count exceeded")
        now = time.monotonic()
        self.max_gap = max(self.max_gap, now - self.previous)
        self.previous = now
        row = {"windowId": self.window["windowId"], "utcSeconds": time.time(),
               "monotonicSeconds": now, **self.probe.sample()}
        if row["cgroup"]["status"] != "observed" or row["process"]["status"] != "observed":
            self.errors.add("target_cpu_unknown")
        encoded = json.dumps(row)
        if len(encoded.encode()) > 128 * 1024:
            raise ValueError("diagnostic sample size exceeded")
        self.output.write(encoded + "\n")
        self.output.flush()
        self.count += 1

    def start(self):
        self.output = (self.root / "cpu-timeseries.jsonl").open("a")
        self.sample()
        self.worker = threading.Thread(target=self.observe, name="diagnostic-cpu-observer", daemon=True)
        self.worker.start()

    def observe(self):
        try:
            while not self.stop.wait(1):
                self.sample()
        except Exception:
            self.errors.add("collector_error")

    def finish(self):
        self.stop.set()
        if self.worker:
            self.worker.join(timeout=10)
        alive = self.worker is not None and self.worker.is_alive()
        if alive:
            self.errors.add("collector_unavailable")
        elif self.output:
            try:
                self.sample()
            except Exception:
                self.errors.add("collector_error")
            self.output.close()
        if self.max_gap > 3 or self.count < 2:
            self.errors.add("sample_gap")
        self.window["cpuObserver"] = {"status": "unavailable" if self.errors else "complete",
                                      "sampleCount": self.count, "maximumGapSeconds": self.max_gap,
                                      "reasonCodes": sorted(self.errors), "clockTicksPerSecond": os.sysconf("SC_CLK_TCK")}


@contextmanager
def observe_cpu(root, window, probe):
    observer = Observer(root, window, probe)
    try:
        observer.start()
        yield observer
    finally:
        observer.finish()


def verify_profile(root, candidate, container, expected_build_id):
    manifests = list(root.glob("*.manifest.json"))
    if len(manifests) != 1:
        raise ValueError("diagnostic capture requires exactly one manifest")
    if manifests[0].is_symlink() or manifests[0].stat().st_size > 16384:
        raise ValueError("invalid diagnostic manifest")
    manifest = json.loads(manifests[0].read_text())
    if (manifest.get("kind") != "cvm-cpu-profile-v1" or manifest["revision"] != candidate or manifest["containerId"] != container
            or manifest["buildId"] != expected_build_id or manifest["durationSeconds"] != 30
            or manifest["rateHz"] != 100 or manifest["sampleCount"] <= 0):
        raise ValueError("diagnostic capture identity mismatch")
    total = 0
    for key, digest_key in [("profile", "sha256"), ("symbols", "symbolsSha256")]:
        name = manifest[key]
        path = root / name
        if Path(name).name != name or path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_BYTES:
            raise ValueError("invalid diagnostic capture artifact")
        total += path.stat().st_size
        if total > MAX_BYTES:
            raise ValueError("diagnostic capture capacity exceeded")
        with path.open("rb") as stream:
            checksum = hashlib.file_digest(stream, "sha256").hexdigest()
        if checksum != manifest[digest_key]:
            raise ValueError("diagnostic capture digest mismatch")
    with gzip.open(root / manifest["profile"], "rb") as stream:
        raw = stream.read(64 * 1024 * 1024 + 1)
    if len(raw) > 64 * 1024 * 1024:
        raise ValueError("diagnostic profile exceeds validation bound")
    profile = json.loads(raw)
    if sum(thread["samples"]["length"] for thread in profile["threads"]) != manifest["sampleCount"]:
        raise ValueError("diagnostic sample count mismatch")
    if not any(lib.get("name") == "codex-vibe-monitor" and lib.get("codeId") == expected_build_id for lib in profile["libs"]):
        raise ValueError("diagnostic profile has no matching library")
    return manifest


def diagnostic_card(candidate, context, windows, error=None):
    expected = [f"{pair}-{enabled}" for pair in range(3)
                for enabled in (["false", "true"] if pair % 2 == 0 else ["true", "false"])]
    complete = (len(windows) == 6 and [w.get("windowId") for w in windows] == expected
                and all(w.get("enabled") == w["windowId"].split("-")[1]
                        and w.get("cpuObserver", {}).get("status") == "complete"
                        and w.get("load", {}).get("completed") == w.get("load", {}).get("offered") == 1500
                        and w.get("baselineFingerprint") is not None
                        and w.get("initialFingerprint") == w.get("baselineFingerprint") for w in windows)
                and all(w.get("profile", {}).get("status") == "verified" for w in windows if w["enabled"] == "true"))
    return {"kind": "cvm-cpu-diagnosis-v1", "candidate": candidate, "context": context,
            "budgetCertification": "not-issued", "profilingAltersWorkload": True,
            "diagnosticStatus": "complete" if complete and error is None else "unavailable",
            "errorClass": error, "windows": len(windows),
            "limitations": ["Diagnostic observer/profiler changes workload cost; no budget certification.",
                            "One 30-second capture per enabled window may miss intermittent work.",
                            "Unavailable hardware and process fields remain unknown."]}
