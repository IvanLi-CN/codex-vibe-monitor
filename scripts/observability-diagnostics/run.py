#!/usr/bin/env python3
"""One hosted diagnostic experiment; preserve all windows, issue no budget card."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
from evidence import Probe, diagnostic_card, fingerprint, observe_cpu, verify_profile

ACCEPTANCE = Path(__file__).resolve().parent.parent / "observability-acceptance"
sys.path.insert(0, str(ACCEPTANCE))
spec = importlib.util.spec_from_file_location("diagnostic_acceptance", ACCEPTANCE / "run.py")
acceptance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(acceptance)
from environment import AdmissionBudget, actions_context, observe_resources, quiet_admission


class DiagnosticRun(acceptance.Run):
    def __init__(self, args):
        if not re.fullmatch(r"sha256:[a-f0-9]{64}", args.image):
            raise ValueError("diagnosis requires the immutable producer image")
        super().__init__(args)
        self.context = actions_context(self.candidate_source, self.root, args.candidate)
        self.windows = []
        self.diagnostic_error = None
        self.profile_build_id = None
        config = json.loads((self.root / "run-config.json").read_text())
        config.update(role="diagnosis-only", budgetCertification="not-issued", windowSeconds=300,
                      warmupSeconds=60, sampleIntervalSeconds=1, captureSeconds=30, captureRateHz=100,
                      captureModes=["false", "true"])
        (self.root / "run-config.json").write_text(json.dumps(config, indent=2) + "\n")

    def configure(self):
        super().configure()
        client = self.definition["services"]["client"]
        client["volumes"] = [*client["volumes"],
                             str(self.source / "scripts/observability-diagnostics") + ":/diagnostics:ro"]
        self.compose_file.write_text(json.dumps(self.definition, indent=2))

    def counters(self):
        try:
            return json.loads(self.compose("exec", "-T", "client", "python", "/diagnostics/snapshot.py", timeout=20))
        except (ValueError, subprocess.SubprocessError) as error:
            return {"status": "unknown", "reason": type(error).__name__}

    def inspected_target(self):
        container = self.compose("ps", "-q", "app")
        metadata = json.loads(acceptance.execute(["docker", "inspect", container]))[0]
        if not metadata["State"]["Running"] or metadata["Image"] != self.image:
            raise ValueError("diagnostic target does not match the live candidate image")
        return metadata["Id"], metadata["State"]["Pid"]

    def prepare_capture(self, window, container):
        name = window["windowId"]
        build = self.root / ("profile-driver-" + name)
        build.mkdir()
        profiles = self.root / "diagnostic-profiles" / name
        profiles.mkdir(parents=True)
        profiles.chmod(0o770)
        binding = json.loads((self.root / "cpu-build/binding.json").read_text())
        binding.update(container=container, profileRoot=str(profiles))
        (build / "binding.json").write_text(json.dumps(binding))
        # Reuse the setup's inspected sampler and exact-image symbols; rebake only
        # the root-owned binding for the current recreated application container.
        (build / "Dockerfile").write_text("FROM " + self.project + ":profiler\nCOPY binding.json /etc/cvm-observability/cpu.json\n")
        image = self.project + ":driver-" + name
        with (self.root / ("profile-build-" + name + ".log")).open("w") as log:
            subprocess.run(["docker", "build", "-t", image, str(build)], stdout=log, stderr=subprocess.STDOUT,
                           check=True, timeout=120)
        return image, profiles

    def capture(self, window, container, prepared):
        image, profiles = prepared
        profiler = self.project + "-capture-" + window["windowId"]
        output = self.root / "profiles" / window["windowId"]
        output.mkdir(parents=True)
        started = time.monotonic()
        command = ["docker", "create", "--name", profiler, "--label", "codex.testbox.agent=" + self.args.agent,
                   "--cap-drop=ALL", "--pid=host", "-v", str(self.root) + ":" + str(self.root),
                   "-v", "/usr/bin/docker:/usr/local/bin/docker:ro",
                   "-v", "/var/run/docker.sock:/var/run/docker.sock", image, "capture", "30"]
        # A failed name reservation must not remove a pre-existing container.
        driver = acceptance.execute(command, timeout=30)
        if not re.fullmatch(r"[a-f0-9]{64}", driver):
            raise ValueError("invalid newly created diagnostic driver identity")
        try:
            with (self.root / ("capture-" + window["windowId"] + ".log")).open("w") as log:
                subprocess.run(["docker", "start", "-a", driver], stdout=log, stderr=subprocess.STDOUT, check=True, timeout=180)
            acceptance.execute(["docker", "cp", driver + ":" + str(profiles) + "/.", str(output)], timeout=30)
            # The paired profile family shares the existing capacity bound.
            fingerprint(self.root / "profiles")
            manifest = verify_profile(output, self.args.candidate, container, self.profile_build_id)
            return {"status": "verified", "launchMonotonicSeconds": started,
                    "finishedMonotonicSeconds": time.monotonic(), "manifest": window["windowId"] + "/" + next(output.glob("*.manifest.json")).name,
                    "startedUTC": manifest["startedAt"], "finishedUTC": manifest["finishedAt"],
                    "buildId": manifest["buildId"], "samples": manifest["sampleCount"]}
        finally:
            # Remove only the ID returned by this create call. The guarded
            # cvm-hotpath-cpu driver owns its separate unique sampler cleanup.
            acceptance.execute(["docker", "rm", "-f", driver], timeout=20)

    def persist_windows(self):
        (self.root / "diagnostic-windows.json").write_text(json.dumps(self.windows, indent=2) + "\n")

    def diagnose(self):
        quiet_admission(self.root)
        self.compose("stop", "app")
        baseline = self.root / "baseline-data"
        shutil.copytree(self.data, baseline)
        baseline_hash = fingerprint(baseline)
        budget = AdmissionBudget()
        for index in range(3):
            for enabled in ["false", "true"] if index % 2 == 0 else ["true", "false"]:
                window = {"windowId": f"{index}-{enabled}", "pairIndex": index, "enabled": enabled,
                          "budgetCertification": "not-issued", "baselineFingerprint": baseline_hash}
                self.windows.append(window)
                directory = self.root / ("diagnostic-data-" + window["windowId"])
                shutil.copytree(baseline, directory)
                directory.chmod(0o770)
                for path in directory.rglob("*"):
                    if path.is_symlink():
                        raise ValueError("synthetic diagnostic state contains an alias")
                    path.chmod(0o770 if path.is_dir() else 0o660)
                window["initialFingerprint"] = fingerprint(directory)
                if window["initialFingerprint"] != baseline_hash:
                    raise ValueError("diagnostic initial state differs from baseline")
                app = self.definition["services"]["app"]
                app["environment"]["OBSERVABILITY_ENABLED"] = enabled
                app["volumes"][0] = str(directory) + ":/srv/app/data"
                self.compose_file.write_text(json.dumps(self.definition, indent=2))
                self.compose("up", "-d", "app")
                self.wait_app()
                container, pid = self.inspected_target()
                window.update(containerId=container, pid=pid, imageId=self.image)
                # Sample both modes at the same point in each load window.
                # Enabled-only stacks cannot explain the incremental CPU cost.
                prepared = self.prepare_capture(window, container)
                self.client("load", "--seconds", "60", "--rate", "5")
                window["admissionWaitSeconds"] = quiet_admission(self.root, timeout=300, window=window, budget=budget)
                window["countersBefore"] = self.counters()
                probe = Probe(pid, container)
                try:
                    with observe_resources(self.root, window), observe_cpu(self.root, window, probe):
                        window.update(startedUTC=time.time(), startedMonotonicSeconds=time.monotonic())
                        before = self.cpu_usec()
                        with ThreadPoolExecutor(max_workers=1) as executor:
                            load = executor.submit(self.client, "load", "--seconds", "300", "--rate", "5")
                            if prepared:
                                time.sleep(60)
                                try:
                                    window["profile"] = self.capture(window, container, prepared)
                                except Exception as error:
                                    window["profile"] = {"status": "unavailable", "reason": type(error).__name__}
                            result = load.result()
                        window["load"] = {**result, "cpuSecondsPerRequest": (self.cpu_usec() - before) / 1e6 / result["completed"]}
                        window.update(endedUTC=time.time(), endedMonotonicSeconds=time.monotonic())
                finally:
                    window["countersAfter"] = self.counters()
                    self.compose("stop", "app")
                    window["finalFingerprint"] = fingerprint(directory)
                    self.persist_windows()
                if result["offered"] != result["completed"] or result["completed"] != 1500:
                    raise ValueError("diagnostic load incomplete")

    def finish(self):
        # Deliberately override the inherited certificate writer. Success here
        # means diagnostic evidence collected; it cannot satisfy acceptance.
        card = diagnostic_card(self.args.candidate, self.context, self.windows, self.diagnostic_error)
        (self.root / "diagnostic-card.json").write_text(json.dumps(card, indent=2) + "\n")
        return card["diagnosticStatus"] == "complete"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["source", "run", "agent", "candidate", "samply", "image"]:
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--candidate-source")
    args = parser.parse_args()
    args.environment, args.suite, args.seconds, args.rate = "github-actions", "runtime", 300, 5
    run = DiagnosticRun(args)
    try:
        run.build()
        run.configure()
        run.start()
        run.checkpoint("https-auth-query", lambda: run.client("functional"))
        run.checkpoint("monitoring-fault-isolation", run.isolation)
        run.checkpoint("original-process-cpu", run.cpu)
        if not all(row["status"] == "passed" for row in run.results.values()):
            raise ValueError("diagnostic setup scenario incomplete")
        run.profile_build_id = run.results["original-process-cpu"]["result"]["buildId"]
        run.diagnose()
        return 0 if run.finish() else 1
    except Exception as error:
        run.diagnostic_error = type(error).__name__
        run.persist_windows()
        run.finish()
        print("CPU diagnostic unavailable: " + type(error).__name__, flush=True)
        return 1
    finally:
        if run.compose_file.exists():
            run.compose("down", "--remove-orphans", timeout=60)


if __name__ == "__main__":
    sys.exit(main())
