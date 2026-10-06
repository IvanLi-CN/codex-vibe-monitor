#!/usr/bin/env python3
"""Regressions for bounded CPU evidence and non-certifying diagnostic isolation."""
import copy
import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SOURCE / "scripts/observability-diagnostics"))
import evidence
import run as diagnostic

spec = importlib.util.spec_from_file_location("diagnostic_gate_contract", SOURCE / ".github/scripts/check_quality_gates_contract.py")
contract = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = contract
spec.loader.exec_module(contract)
CONTAINER, CANDIDATE, BUILD = "a" * 64, "b" * 40, "c" * 40


def stat(start=99):
    fields = ["0"] * 40
    fields[0], fields[11], fields[12], fields[17], fields[19] = "S", "10", "20", "2", str(start)
    return "123 (worker ) with spaces) " + " ".join(fields)


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def probe(self):
        proc, groups, cpus = [self.root / name for name in ["proc", "groups", "cpus"]]
        pid = proc / "123"
        pid.mkdir(parents=True)
        (pid / "stat").write_text(stat())
        (pid / "cgroup").write_text("0::/docker/" + CONTAINER + "\n")
        (pid / "io").write_text("read_bytes: 50\nwrite_bytes: 60\n")
        task = pid / "task/123"
        task.mkdir(parents=True)
        (task / "status").write_text("voluntary_ctxt_switches: 2\nnonvoluntary_ctxt_switches: 3\n")
        (proc / "stat").write_text("cpu 1 2 3 4 5 6 7 8 9 10\n")
        group = groups / "docker" / CONTAINER
        group.mkdir(parents=True)
        (group / "cpu.stat").write_text("usage_usec 30\nuser_usec 10\nsystem_usec 20\nnr_periods 1\nnr_throttled 0\nthrottled_usec 0\n")
        return evidence.Probe(123, CONTAINER, proc, groups, cpus)

    def test_numeric_cpu_and_missing_hardware_remain_distinct(self):
        probe = self.probe()
        with patch.object(evidence.os, "sched_getaffinity", create=True, return_value={0}):
            row = probe.sample()
        self.assertEqual(row["process"]["values"]["systemTicks"], 20)
        self.assertEqual(row["contextSwitches"]["values"]["involuntary"], 3)
        self.assertEqual(row["hostTicks"]["values"]["steal"], 8)
        self.assertEqual(row["frequency"]["values"]["cpus"][0]["status"], "unknown")
        self.assertEqual(row["cgroup"]["status"], "observed")

    def test_pid_reuse_discards_all_pid_bound_fields(self):
        probe = self.probe()
        (probe.proc / "123/stat").write_text(stat(start=100))
        with patch.object(evidence.os, "sched_getaffinity", create=True, return_value=set()), patch.object(probe, "switches") as switches:
            row = probe.sample()
        switches.assert_not_called()
        for key in ["process", "contextSwitches", "processIO"]:
            self.assertEqual(row[key]["status"], "unknown")

    def test_missing_cgroup_cpu_is_not_a_zero_or_complete_sample(self):
        probe = self.probe()
        (probe.group / "cpu.stat").write_text("usage_usec 0\n")
        with patch.object(evidence.os, "sched_getaffinity", create=True, return_value=set()):
            self.assertEqual(probe.sample()["cgroup"]["status"], "unknown")

    def test_unrelated_or_escaped_cgroup_is_rejected(self):
        probe = self.probe()
        (probe.proc / "123/cgroup").write_text("0::/docker/" + CONTAINER + "/../../..\n")
        with self.assertRaises(ValueError): evidence.Probe(123, CONTAINER, probe.proc, self.root / "groups")
        (probe.proc / "123/cgroup").write_text("0::/docker\n")
        with self.assertRaises(ValueError): evidence.Probe(123, CONTAINER, probe.proc, self.root / "groups")

    def test_snapshot_imports_client_without_executing_cli(self):
        # Match the fresh /diagnostics/snapshot.py process, whose argv has no mode.
        probe = r"""
import json
from pathlib import Path
import sys
import tempfile
sys.path[:0] = [sys.argv[1], sys.argv[2]]
sys.argv = ["snapshot.py"]
import snapshot
import client
with tempfile.TemporaryDirectory() as directory:
    client.ROOT = Path(directory)
    (client.ROOT / "metrics-token").write_text("synthetic-scrape-token")
    def request(base, path, **kwargs):
        assert (base, path) == ("http://app:9091", "/metrics")
        assert kwargs == {"token": "synthetic-scrape-token"}
        return 200, b"cvm_sqlite_retries_total 2\n"
    client.request = request
    result = snapshot.main()
    assert result["status"] == "observed", result
    assert result["values"] == {"cvm_sqlite_retries_total": 2.0}, result
    print(json.dumps({"status": result["status"], "values": result["values"]}))
"""
        result = subprocess.run([sys.executable, "-c", probe,
                                 str(SOURCE / "scripts/observability-diagnostics"),
                                 str(SOURCE / "scripts/observability-acceptance")],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["status"], "observed")

    def test_metrics_emit_fixed_numeric_sums_without_labels(self):
        name = "cvm_sqlite_retries_total"
        result = evidence.metric_summary(f'{name}{{sql="secret",token="private"}} 2\n{name}{{sql="other"}} 3\nprivate_metric 88\n')
        self.assertEqual(result["values"], {name: 5})
        self.assertNotIn("secret", json.dumps(result))
        self.assertIn("cvm_process_cpu_seconds_total", result["missing"])
        for value in ["NaN", "Inf", "-1"]:
            with self.assertRaises(ValueError): evidence.metric_summary(name + " " + value)
        with self.assertRaises(ValueError): evidence.metric_summary("x" * (1024 * 1024 + 1))

    def test_fingerprint_detects_change_rejects_alias_and_capacity(self):
        state = self.root / "state"; state.mkdir()
        data = state / "db"; data.write_bytes(b"synthetic")
        before = evidence.fingerprint(state)
        data.write_bytes(b"different")
        self.assertNotEqual(before, evidence.fingerprint(state))
        with patch.object(evidence, "MAX_BYTES", 2):
            with self.assertRaises(ValueError): evidence.fingerprint(state)
        (state / "alias").symlink_to(data)
        with self.assertRaises(ValueError): evidence.fingerprint(state)

    def test_observer_stops_own_thread_and_bounds_samples(self):
        probe = SimpleNamespace(sample=lambda: {"cgroup": {"status": "observed"}, "process": {"status": "observed"}})
        window = {"windowId": "0-false"}
        with evidence.observe_cpu(self.root, window, probe) as observer:
            pass
        self.assertFalse(observer.worker.is_alive())
        self.assertEqual(window["cpuObserver"]["status"], "complete")
        self.assertEqual(len((self.root / "cpu-timeseries.jsonl").read_text().splitlines()), 2)
        observer.count = evidence.MAX_SAMPLES
        with self.assertRaises(ValueError): observer.sample()

    def profile(self):
        profile = self.root / "capture.json.gz"
        with gzip.open(profile, "wt") as stream:
            json.dump({"libs": [{"name": "codex-vibe-monitor", "codeId": BUILD}], "threads": [{"samples": {"length": 4}}]}, stream)
        symbols = self.root / "capture.syms.json"; symbols.write_text("{}")
        manifest = {"kind": "cvm-cpu-profile-v1", "revision": CANDIDATE, "containerId": CONTAINER,
                    "buildId": BUILD, "durationSeconds": 30, "rateHz": 100, "sampleCount": 4,
                    "profile": profile.name, "symbols": symbols.name,
                    "sha256": hashlib.sha256(profile.read_bytes()).hexdigest(),
                    "symbolsSha256": hashlib.sha256(symbols.read_bytes()).hexdigest()}
        path = self.root / "capture.manifest.json"; path.write_text(json.dumps(manifest))
        return path, manifest

    def test_capture_requires_matching_revision_container_symbols_and_samples(self):
        path, manifest = self.profile()
        evidence.verify_profile(self.root, CANDIDATE, CONTAINER, BUILD)
        for key, value in [("revision", "d" * 40), ("containerId", "d" * 64), ("buildId", "d" * 40),
                           ("symbolsSha256", "d" * 64), ("sampleCount", 5), ("profile", "../escape.gz")]:
            path.write_text(json.dumps({**manifest, key: value}))
            with self.assertRaises(ValueError): evidence.verify_profile(self.root, CANDIDATE, CONTAINER, BUILD)

    def test_complete_diagnosis_can_never_certify_budget(self):
        windows = [{"windowId": f"{pair}-{enabled}", "enabled": enabled,
                    "cpuObserver": {"status": "complete"}, "profile": {"status": "verified"},
                    "load": {"offered": 1500, "completed": 1500}, "baselineFingerprint": {"sha256": "x"},
                    "initialFingerprint": {"sha256": "x"}}
                   for pair in range(3) for enabled in (["false", "true"] if pair % 2 == 0 else ["true", "false"])]
        card = evidence.diagnostic_card(CANDIDATE, {}, windows)
        self.assertEqual(card["diagnosticStatus"], "complete")
        self.assertEqual(card["budgetCertification"], "not-issued")
        self.assertNotIn("withinBudget", card)
        for changed in [windows[:-1], [windows[0]] * 6]:
            self.assertEqual(evidence.diagnostic_card(CANDIDATE, {}, changed)["diagnosticStatus"], "unavailable")
        # Without the disabled-mode stack, the collector cannot distinguish
        # shared application work from work added by observability.
        missing_disabled_profile = [{**window} for window in windows]
        missing_disabled_profile[0].pop("profile")
        self.assertEqual(
            evidence.diagnostic_card(CANDIDATE, {}, missing_disabled_profile)["diagnosticStatus"],
            "unavailable",
        )
        windows[0]["load"]["completed"] = 1499
        self.assertEqual(evidence.diagnostic_card(CANDIDATE, {}, windows)["diagnosticStatus"], "unavailable")

    def test_driver_creation_failure_does_not_remove_existing_name(self):
        run = object.__new__(diagnostic.DiagnosticRun)
        run.root, run.project, run.args = self.root, "testbox-owned", SimpleNamespace(agent="owned", candidate=CANDIDATE)
        with patch.object(diagnostic.acceptance, "execute", side_effect=subprocess.CalledProcessError(1, "create")) as execute:
            with self.assertRaises(subprocess.CalledProcessError):
                run.capture({"windowId": "0-true"}, CONTAINER, ("fixed-driver-image", self.root / "profile-source"))
        self.assertEqual(execute.call_count, 1)
        self.assertEqual(execute.call_args.args[0][:2], ["docker", "create"])

    def test_failed_driver_start_removes_only_newly_created_id(self):
        run = object.__new__(diagnostic.DiagnosticRun)
        run.root, run.project, run.args = self.root, "testbox-owned", SimpleNamespace(agent="owned", candidate=CANDIDATE)
        driver = "e" * 64
        with patch.object(diagnostic.acceptance, "execute", return_value=driver) as execute, \
             patch.object(diagnostic.subprocess, "run", side_effect=subprocess.TimeoutExpired("start", 180)):
            with self.assertRaises(subprocess.TimeoutExpired):
                run.capture({"windowId": "0-true"}, CONTAINER, ("fixed-driver-image", self.root / "profile-source"))
        self.assertEqual(execute.call_args.args[0], ["docker", "rm", "-f", driver])


class WorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = contract.load_yaml(SOURCE / ".github/workflows/ci-pr.yml")

    def test_separate_hosted_diagnosis_preserves_certification_contract(self):
        contract.require_observability_diagnostic_contract(self.workflow)
        contract.require_observability_performance_contract(self.workflow, self.workflow["jobs"]["build"])

    def test_private_unbounded_and_certificate_artifacts_rejected(self):
        for suffix in ["private/**", "*.log", "empirical-card.json", "**", "data/*.db"]:
            workflow = copy.deepcopy(self.workflow)
            step = workflow["jobs"]["observability-diagnostics"]["steps"][-1]
            step["with"]["path"] += "\n${{ runner.temp }}/observability-diagnostics/" + suffix
            with self.assertRaises(contract.ContractError): contract.require_observability_diagnostic_contract(workflow)

    def test_wrong_runner_certifying_entrypoint_or_budget_dependency_rejected(self):
        for key, value in [("runs-on", "self-hosted"), ("needs", "observability-performance"), ("if", "always()")]:
            workflow = copy.deepcopy(self.workflow)
            workflow["jobs"]["observability-diagnostics"][key] = value
            with self.assertRaises(contract.ContractError): contract.require_observability_diagnostic_contract(workflow)
        workflow = copy.deepcopy(self.workflow)
        workflow["jobs"]["build"]["needs"].append("observability-diagnostics")
        with self.assertRaises(contract.ContractError): contract.require_observability_diagnostic_contract(workflow)


if __name__ == "__main__":
    unittest.main()
