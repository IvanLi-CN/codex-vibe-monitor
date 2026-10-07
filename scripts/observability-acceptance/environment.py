"""Execution identity and raw resource evidence for the Actions performance gate."""
from contextlib import contextmanager
import json
import math
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import threading
import time


def actions_context(source, run, candidate):
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise ValueError("performance acceptance requires a GitHub-hosted Actions runner")
    temporary = os.environ.get("RUNNER_TEMP", "")
    if not temporary or not run.is_relative_to(Path(temporary).resolve()) or run == Path(temporary).resolve():
        raise ValueError("Actions evidence must be in a child of RUNNER_TEMP")
    repo = os.environ.get("GITHUB_REPOSITORY", "")
    run_id = os.environ.get("GITHUB_RUN_ID", "")
    attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo) or not run_id.isdigit() or not attempt.isdigit():
        raise ValueError("invalid Actions run identity")
    head = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
    if head != candidate:
        raise ValueError("checked-out source does not match performance Candidate SHA")
    models = [line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")]
    return {
        "candidate": candidate,
        "repository": repo,
        "runId": run_id,
        "attempt": attempt,
        "runnerEnvironment": "github-hosted",
        "runnerImage": os.environ.get("ImageOS"),
        "runnerImageVersion": os.environ.get("ImageVersion"),
        "kernel": platform.release(),
        "cpuCount": os.cpu_count(),
        "cpuModel": sorted(set(models)),
        "cpuAffinity": sorted(os.sched_getaffinity(0)),
        "evidenceLocator": f"https://github.com/{repo}/actions/runs/{run_id}/attempts/{attempt}",
    }


def pressure():
    return {name: Path("/proc/pressure", name).read_text().strip() for name in ("cpu", "io", "memory")}


PRESSURE_LIMITS = {"cpu": 2.0, "io": 5.0, "memory": 0.1}


def measurement_cpu_layout(affinity):
    """Pin the app to one runner CPU and leave helpers on runner-default affinity."""
    cpus = sorted({int(cpu) for cpu in affinity})
    if len(cpus) < 2 or any(cpu < 0 for cpu in cpus):
        raise ValueError("performance acceptance requires at least two runner CPUs for isolation")
    return {"app": str(cpus[0]), "auxiliary": "runner-default"}


def pressure_eligible(raw):
    if not isinstance(raw, dict) or set(raw) != set(PRESSURE_LIMITS):
        raise ValueError("missing pressure resources")
    eligible = True
    for name, limit in PRESSURE_LIMITS.items():
        some = re.search(r"^some\s+([^\n]+)", raw[name], re.M)
        if some is None:
            raise ValueError("missing some pressure line")
        pieces = some.group(1).split()
        fields = dict(piece.split("=", 1) for piece in pieces)
        if len(fields) != len(pieces):
            raise ValueError("duplicate pressure fields")
        for key in ("avg10", "avg60"):
            value = float(fields[key])
            if not math.isfinite(value) or not 0 <= value <= 100:
                raise ValueError("invalid pressure percentage")
            eligible = eligible and value < limit
    return eligible


def pressure_sample(identity):
    sample = dict(identity)
    try:
        sample["hostPressure"] = pressure()
        sample["eligible"] = pressure_eligible(sample["hostPressure"])
    except Exception as error:
        sample.update(eligible=False, errorClass=type(error).__name__)
    sample.update(utcSeconds=time.time(), monotonicSeconds=time.monotonic())
    return sample


class AdmissionBudget:
    def __init__(self):
        self.remaining_seconds = 900.0


def quiet_admission(root, timeout=600, window=None, budget=None):
    started = time.monotonic()
    allowed = min(timeout, budget.remaining_seconds) if budget is not None else timeout
    if allowed <= 0:
        raise OSError("cumulative quiet admission budget exhausted")
    deadline = started + allowed
    consecutive = 0
    identity = {**(window or {}), "phase": "admission" if window else "initial-admission"}
    try:
        with (root / "environment-admission.jsonl").open("a" if window else "w") as output:
            while True:
                if time.monotonic() > deadline:
                    raise OSError("quiet admission budget exhausted")
                sample = pressure_sample(identity)
                consecutive = consecutive + 1 if sample["eligible"] else 0
                sample["consecutive"] = consecutive
                output.write(json.dumps(sample) + "\n")
                output.flush()
                if "errorClass" in sample:
                    raise OSError("runner pressure evidence is invalid")
                if consecutive >= 3 and time.monotonic() <= deadline:
                    return time.monotonic() - started
                if time.monotonic() >= deadline:
                    raise OSError("runner did not reach a quiet window; performance acceptance unavailable")
                time.sleep(min(20, max(0, deadline - time.monotonic())))
    finally:
        if budget is not None:
            budget.remaining_seconds = max(0, budget.remaining_seconds - (time.monotonic() - started))


class MeasurementObserver:
    """Own one measured window; resource failure cannot issue a passing certificate."""
    interval_seconds = 10
    maximum_gap_seconds = 20

    def __init__(self, root, window):
        self.root = root
        self.identity = {**window, "phase": "measurement"}
        self.stop = threading.Event()
        self.errors = set()
        self.samples = 0
        self.maximum_gap = 0.0
        self.started_utc = time.time()
        self.started_monotonic = time.monotonic()
        self.previous_time = self.started_monotonic
        self.output = None
        self.worker = None

    def sample(self):
        row = pressure_sample(self.identity)
        now = row["monotonicSeconds"]
        if self.previous_time is not None:
            gap = now - self.previous_time
            self.maximum_gap = max(self.maximum_gap, gap)
            if gap < 0 or gap > self.maximum_gap_seconds:
                self.errors.add("sample_gap")
        self.previous_time = now
        self.samples += 1
        self.output.write(json.dumps(row) + "\n")
        self.output.flush()
        if "errorClass" in row:
            self.errors.add("invalid_sample")
        elif not row["eligible"]:
            self.errors.add("pressure_exceeded")

    def start(self):
        self.output = (self.root / "resource-observer.jsonl").open("a")
        self.sample()
        if self.errors:
            raise OSError("measurement began without valid quiet resource evidence")
        self.worker = threading.Thread(target=self.observe, name="acceptance-resource-observer", daemon=True)
        self.worker.start()

    def observe(self):
        try:
            while not self.stop.wait(self.interval_seconds):
                self.sample()
        except Exception:
            self.errors.add("collector_error")

    def finish(self):
        self.stop.set()
        if self.worker is not None:
            self.worker.join(timeout=15)
        alive = self.worker is not None and self.worker.is_alive()
        if alive:
            self.errors.add("collector_unavailable")
        elif self.output is not None:
            try:
                self.sample()
            except Exception:
                self.errors.add("collector_error")
            finally:
                self.output.close()
        if self.samples < 2:
            self.errors.add("missing_boundary_sample")
        ended_monotonic = time.monotonic()
        trailing_gap = ended_monotonic - self.previous_time
        self.maximum_gap = max(self.maximum_gap, trailing_gap)
        if trailing_gap < 0 or trailing_gap > self.maximum_gap_seconds:
            self.errors.add("sample_gap")
        summary = {**self.identity, "startedUTCSeconds": self.started_utc,
                   "startedMonotonicSeconds": self.started_monotonic, "endedUTCSeconds": time.time(),
                   "endedMonotonicSeconds": ended_monotonic, "sampleCount": self.samples,
                   "maximumGapSeconds": self.maximum_gap, "pressureLimits": PRESSURE_LIMITS,
                   "status": "unavailable" if self.errors else "passed", "reasonCodes": sorted(self.errors)}
        path = self.root / "measurement-windows.json"
        try:
            windows = json.loads(path.read_text()) if path.exists() else []
        except (ValueError, UnicodeError) as error:
            raise OSError("invalid measurement window evidence") from error
        if not isinstance(windows, list) or len(windows) >= 9 or any(not isinstance(row, dict) or row.get("windowId") == summary["windowId"] for row in windows):
            raise OSError("invalid measurement window evidence")
        windows.append(summary)
        path.write_text(json.dumps(windows, indent=2) + "\n")
        if self.errors:
            raise OSError("measured resource environment unavailable: " + ",".join(sorted(self.errors)))


@contextmanager
def observe_resources(root, window):
    observer = MeasurementObserver(root, window)
    try:
        observer.start()
        yield observer
    finally:
        observer.finish()


def verify_measurement_evidence(root, modes=("off", "metrics", "full")):
    """Admit only the complete mode/round windows with independently readable quiet samples."""
    try:
        run_config = json.loads((root / "run-config.json").read_text())
        runner_context = json.loads((root / "runner-context.json").read_text())
        windows = json.loads((root / "measurement-windows.json").read_text())
        raw = [json.loads(line) for line in (root / "resource-observer.jsonl").read_text().splitlines()]
        admissions = [json.loads(line) for line in (root / "environment-admission.jsonl").read_text().splitlines()]
        loads = json.loads((root / "ab-samples.json").read_text())
        cpu_layout = measurement_cpu_layout(runner_context["cpuAffinity"])
        assert run_config["appCpuQuota"] == 1
        assert run_config["appCpuSet"] == cpu_layout["app"]
        assert run_config["auxiliaryCpuSet"] == cpu_layout["auxiliary"]
        expected = {f"{index}-{mode}" for index in range(3) for mode in modes}
        assert len(windows) == 3 * len(modes) and {row["windowId"] for row in windows} == expected
        assert {row["windowId"] for row in raw} == expected
        assert set(loads) == set(modes) and all(len(rows) == 3 for rows in loads.values())
        if "full" in modes:
            assert all(row["traceEvidence"]["enabled"] and row["traceEvidence"]["searchableTrace"] and row["traceEvidence"]["exportedSpans"] > 0 and row["traceEvidence"]["failedSpans"] == 0 and row["traceEvidence"]["droppedSpans"] == 0 for row in loads["full"])
        load_rows = {row["windowId"]: row for rows in loads.values() for row in rows}
        assert set(load_rows) == expected
        assert sum(window["admissionWaitSeconds"] for window in windows) <= 900
        for window in windows:
            window_id = window["windowId"]
            assert window_id == f"{window['pairIndex']}-{window['enabled']}"
            assert window["status"] == "passed" and not window["reasonCodes"]
            assert window["pressureLimits"] == PRESSURE_LIMITS and 0 <= window["admissionWaitSeconds"] <= 300
            start, end = window["startedMonotonicSeconds"], window["endedMonotonicSeconds"]
            assert all(math.isfinite(value) for value in (start, end)) and end >= start
            assert end - start >= load_rows[window_id]["durationSeconds"] > 0
            rows = [row for row in raw if row["windowId"] == window_id]
            assert len(rows) == window["sampleCount"] and len(rows) >= 2
            times = [start]
            for row in rows:
                assert row["pairIndex"] == window["pairIndex"] and row["enabled"] == window["enabled"]
                assert row["phase"] == "measurement" and row["eligible"] is True and "errorClass" not in row
                assert pressure_eligible(row["hostPressure"])
                assert math.isfinite(row["monotonicSeconds"]) and math.isfinite(row["utcSeconds"])
                times.append(row["monotonicSeconds"])
            times.append(end)
            gaps = [after - before for before, after in zip(times, times[1:])]
            assert all(0 <= gap <= 20 for gap in gaps) and window["maximumGapSeconds"] <= 20
            admitted = [row for row in admissions if row.get("windowId") == window_id]
            assert len(admitted) >= 3
            for consecutive, row in enumerate(admitted[-3:], 1):
                assert row["phase"] == "admission" and row["consecutive"] == consecutive
                assert row["eligible"] is True and "errorClass" not in row and pressure_eligible(row["hostPressure"])
            assert admitted[-1]["monotonicSeconds"] <= start
    except (AssertionError, KeyError, TypeError, ValueError, OverflowError) as error:
        raise OSError("measurement environment evidence is incomplete or invalid") from error


def comparison_report(samples):
    modes = ("off", "metrics", "full") if set(samples) == {"off", "metrics", "full"} else ("false", "true")
    if set(samples) != set(modes):
        raise ValueError("unexpected observation modes")
    metrics = {}
    for key in ("cpuSecondsPerRequest", "p95Seconds"):
        metric = {}
        for enabled in modes:
            values = [row[key] for row in samples[enabled]]
            if len(values) != 3 or any(value <= 0 for value in values):
                raise ValueError("performance acceptance needs three complete positive windows per mode")
            cv = statistics.pstdev(values) / statistics.mean(values)
            metric[enabled] = {"values": values, "cv": cv, "stable": cv <= 0.05}
        valid = all(metric[mode]["stable"] for mode in modes)
        baseline = statistics.median(metric[modes[0]]["values"])
        enabled = statistics.median(metric[modes[-1]]["values"])
        increase = enabled / baseline - 1
        metric.update({"comparisonValid": valid, "increase": increase if valid else None, "withinBudget": valid and enabled <= baseline * 1.05})
        if len(modes) == 3:
            metrics_only = statistics.median(metric["metrics"]["values"])
            metric["traceIncrement"] = enabled / metrics_only - 1 if valid else None
        metrics[key] = metric
    return {"stabilityLimit": 0.05, "overheadLimit": 0.05, "metrics": metrics}
