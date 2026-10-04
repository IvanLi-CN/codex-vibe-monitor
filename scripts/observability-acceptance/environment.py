"""Execution identity and raw resource evidence for the Actions performance gate."""
from contextlib import contextmanager
import json
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


def quiet_admission(root, timeout=600):
    deadline = time.monotonic() + timeout
    consecutive = 0
    with (root / "environment-admission.jsonl").open("w") as output:
        while True:
            raw = pressure()
            values = {name: {key: float(re.search(r"\b" + key + r"=([0-9.]+)", text).group(1)) for key in ("avg10", "avg60")} for name, text in raw.items()}
            eligible = all(values["cpu"][key] < 2 and values["io"][key] < 5 and values["memory"][key] < 0.1 for key in ("avg10", "avg60"))
            consecutive = consecutive + 1 if eligible else 0
            output.write(json.dumps({"utcSeconds": time.time(), "pressure": raw, "eligible": eligible, "consecutive": consecutive}) + "\n")
            output.flush()
            if consecutive >= 3:
                return
            if time.monotonic() >= deadline:
                raise OSError("runner did not reach a quiet window; performance acceptance unavailable")
            time.sleep(min(20, max(0, deadline - time.monotonic())))


@contextmanager
def observe_resources(root):
    stop = threading.Event()
    errors = []

    def observe():
        try:
            with (root / "resource-observer.jsonl").open("w") as output:
                while not stop.is_set():
                    output.write(json.dumps({"utcSeconds": time.time(), "hostPressure": pressure()}) + "\n")
                    output.flush()
                    stop.wait(10)
        except Exception as error:
            errors.append(error)

    worker = threading.Thread(target=observe, name="acceptance-resource-observer")
    worker.start()
    try:
        yield
    finally:
        stop.set()
        worker.join(timeout=15)
        if worker.is_alive() or errors:
            raise OSError("runner resource evidence could not be recorded")


def comparison_report(samples):
    metrics = {}
    for key in ("cpuSecondsPerRequest", "p95Seconds"):
        metric = {}
        for enabled in ("false", "true"):
            values = [row[key] for row in samples[enabled]]
            if len(values) != 3 or any(value <= 0 for value in values):
                raise ValueError("performance acceptance needs three complete positive windows per mode")
            cv = statistics.pstdev(values) / statistics.mean(values)
            metric[enabled] = {"values": values, "cv": cv, "stable": cv <= 0.05}
        valid = all(metric[mode]["stable"] for mode in ("false", "true"))
        baseline = statistics.median(metric["false"]["values"])
        enabled = statistics.median(metric["true"]["values"])
        increase = enabled / baseline - 1
        metric.update({"comparisonValid": valid, "increase": increase if valid else None, "withinBudget": valid and enabled <= baseline * 1.05})
        metrics[key] = metric
    return {"stabilityLimit": 0.05, "overheadLimit": 0.05, "metrics": metrics}
