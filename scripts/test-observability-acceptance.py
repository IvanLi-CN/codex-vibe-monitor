#!/usr/bin/env python3
"""Regression checks for performance evidence admission and certificate scope."""
import importlib.util
import copy
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SOURCE / "scripts/observability-acceptance"))
import environment
from run import Run
import run as acceptance
from classify import ClassificationError, classify_result
spec = importlib.util.spec_from_file_location("performance_gate_contract", SOURCE / ".github/scripts/check_quality_gates_contract.py")
contract = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = contract
spec.loader.exec_module(contract)


def psi(cpu=0, io=0, memory=0):
    return {name: f"some avg10={value} avg60={value} avg300=0 total=0\nfull avg10=0 avg60=0 avg300=0 total=0"
            for name, value in [("cpu", cpu), ("io", io), ("memory", memory)]}


class ContainerMountTests(unittest.TestCase):
    def test_candidate_does_not_receive_shared_tempo_ingest_credential(self):
        source = (SOURCE / "scripts/observability-acceptance/run.py").read_text()
        self.assertIn("tempo-runtime-token", source)
        self.assertIn("/internal/v1/traces", source)
        self.assertNotIn('self.private/"tempo-ingest-token")+":/run/secrets/tempo-ingest-token:ro"', source)

    def test_https_entry_certificate_is_a_server_leaf_and_covers_its_hostname(self):
        with tempfile.TemporaryDirectory() as directory:
            private = Path(directory)
            acceptance.create_entry_certificate(private)
            certificate = acceptance.execute(["openssl", "x509", "-in", str(private / "tls.crt"), "-noout", "-text"])
            self.assertIn("CA:FALSE", certificate)
            self.assertIn("TLS Web Server Authentication", certificate)
            self.assertIn("DNS:entry", certificate)
            result = acceptance.execute(["openssl", "verify", "-CAfile", str(private / "tls.crt"), "-purpose", "sslserver", "-verify_hostname", "entry", str(private / "tls.crt")])
            self.assertTrue(result.endswith(": OK"))

    def execute_mounted(self, name):
        script = SOURCE / "scripts/observability-acceptance" / name
        namespace = {"__name__": "mounted_fixture", "__file__": "/work/" + name}
        with patch.object(sys, "path", [str(SOURCE / "ops/observability"), *sys.path]):
            exec(compile(script.read_text(), str(script), "exec"), namespace)
        return namespace

    def test_client_bootstrap_works_at_container_mount_depth(self):
        namespace = self.execute_mounted("client.py")
        self.assertTrue(callable(namespace["seed"]))

    def test_tempo_trace_ids_restore_omitted_leading_zero(self):
        namespace = self.execute_mounted("client.py")
        self.assertEqual(namespace["normalized_trace_id"]("1" * 30), "00" + "1" * 30)
        self.assertEqual(namespace["normalized_trace_id"]("1" * 31), "0" + "1" * 31)
        self.assertEqual(namespace["normalized_trace_id"]("2" * 32), "2" * 32)
        with self.assertRaises(AssertionError):
            namespace["normalized_trace_id"]("1" * 33)

    def test_peer_bootstrap_reaches_dispatch_at_container_mount_depth(self):
        with patch.object(sys, "argv", ["fixture.py", "unsupported-fixture-command"]):
            with self.assertRaisesRegex(SystemExit, "unknown fixture mode"):
                self.execute_mounted("fixture.py")


class ResourceAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.clock = 100.0
        self.window = {"windowId": "0-false", "pairIndex": 0, "enabled": "false"}

    def test_measurement_service_profiles_keep_only_required_observers_running(self):
        self.assertEqual(acceptance.measurement_services("off"), {"prometheus": False, "grafana": False, "tempo": False, "entry": False})
        self.assertEqual(acceptance.measurement_services("metrics"), {"prometheus": True, "grafana": False, "tempo": False, "entry": False})
        self.assertEqual(acceptance.measurement_services("full"), {"prometheus": True, "grafana": False, "tempo": True, "entry": True})

    def sleep(self, seconds):
        self.assertLessEqual(seconds, 20)
        self.clock += seconds

    def fake_clock(self):
        return patch.object(environment.time, "monotonic", side_effect=lambda: self.clock)

    def test_thresholds_are_strict_and_invalid_values_are_unknown(self):
        self.assertTrue(environment.pressure_eligible(psi(1.99, 4.99, 0.09)))
        for values in [(2, 0, 0), (0, 5, 0), (0, 0, 0.1)]:
            self.assertFalse(environment.pressure_eligible(psi(*values)))
        for value in ["nan", "inf", -1, 101]:
            with self.assertRaises(ValueError): environment.pressure_eligible(psi(cpu=value))
        missing = psi(); del missing["memory"]
        with self.assertRaises(ValueError): environment.pressure_eligible(missing)
        invalid = psi(); invalid["cpu"] = "full avg10=0 avg60=0"
        with patch.object(environment, "pressure", return_value=invalid):
            sample = environment.pressure_sample(self.window)
        self.assertFalse(sample["eligible"])
        self.assertEqual(sample["hostPressure"], invalid)
        self.assertIn("errorClass", sample)

    def test_three_consecutive_samples_after_warmup_consume_shared_budget(self):
        budget = environment.AdmissionBudget()
        with self.fake_clock(), patch.object(environment.time, "sleep", side_effect=self.sleep), \
             patch.object(environment, "pressure", side_effect=[psi(), psi(cpu=3), psi(), psi(), psi()]):
            elapsed = environment.quiet_admission(self.root, timeout=300, window=self.window, budget=budget)
        self.assertEqual(elapsed, 80)
        self.assertEqual(budget.remaining_seconds, 820)
        rows = [json.loads(row) for row in (self.root / "environment-admission.jsonl").read_text().splitlines()]
        self.assertEqual([row["consecutive"] for row in rows], [1, 0, 1, 2, 3])
        self.assertTrue(all(row["windowId"] == "0-false" and row["phase"] == "admission" for row in rows))

    def test_per_window_and_cumulative_timeouts_do_not_retry_measurement(self):
        budget = environment.AdmissionBudget()
        with self.fake_clock(), patch.object(environment.time, "sleep", side_effect=self.sleep), \
             patch.object(environment, "pressure", return_value=psi(cpu=3)):
            for index in range(3):
                window = {**self.window, "windowId": f"{index}-false", "pairIndex": index}
                with self.assertRaises(OSError): environment.quiet_admission(self.root, timeout=300, window=window, budget=budget)
            self.assertEqual(self.clock, 1000)
            self.assertEqual(budget.remaining_seconds, 0)
            with self.assertRaisesRegex(OSError, "cumulative"):
                environment.quiet_admission(self.root, timeout=300, window=self.window, budget=budget)
        self.assertFalse((self.root / "measurement-windows.json").exists())

    def test_collector_fault_is_unavailable_and_raw_failure_is_preserved(self):
        with patch.object(environment, "pressure", side_effect=OSError("synthetic collector fault")):
            with self.assertRaisesRegex(OSError, "invalid"):
                environment.quiet_admission(self.root, window=self.window, budget=environment.AdmissionBudget())
        rows = [json.loads(row) for row in (self.root / "environment-admission.jsonl").read_text().splitlines()]
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["errorClass"], "OSError")

    def test_mid_window_pressure_cannot_be_hidden_by_quiet_final_sample(self):
        with self.fake_clock(), patch.object(environment, "pressure", return_value=psi()):
            with environment.observe_resources(self.root, self.window) as observer:
                self.clock += 10
                with patch.object(environment, "pressure", return_value=psi(cpu=3)):
                    observer.sample()
                self.clock += 10
        windows = json.loads((self.root / "measurement-windows.json").read_text())
        self.assertEqual(windows[0]["status"], "passed")
        self.assertEqual(windows[0]["sampleCount"], 3)
        self.assertEqual(windows[0]["pressureExceededSamples"], 1)
        rows = [json.loads(row) for row in (self.root / "resource-observer.jsonl").read_text().splitlines()]
        self.assertEqual([row["eligible"] for row in rows], [True, False, True])
        self.assertTrue(all(row["windowId"] == "0-false" for row in rows))

    def test_sample_gap_and_malformed_pressure_block_window(self):
        for reason in ["sample_gap", "invalid_sample"]:
            with self.subTest(reason=reason), tempfile.TemporaryDirectory() as directory:
                with self.fake_clock(), patch.object(environment, "pressure", return_value=psi()):
                    with self.assertRaisesRegex(OSError, reason):
                        with environment.observe_resources(Path(directory), self.window) as observer:
                            if reason == "sample_gap":
                                self.clock += 21
                            else:
                                with patch.object(environment, "pressure", return_value={}): observer.sample()
                self.assertEqual(json.loads((Path(directory) / "measurement-windows.json").read_text())[0]["status"], "unavailable")

    def test_valid_windows_keep_explicit_boundaries_and_separate_identity(self):
        with self.fake_clock(), patch.object(environment, "pressure", return_value=psi()):
            for mode in ["false", "true"]:
                window = {**self.window, "enabled": mode, "windowId": f"0-{mode}"}
                with environment.observe_resources(self.root, window): self.clock += 10
        windows = json.loads((self.root / "measurement-windows.json").read_text())
        self.assertEqual([row["windowId"] for row in windows], ["0-false", "0-true"])
        self.assertTrue(all(row["status"] == "passed" and row["sampleCount"] == 2 for row in windows))
        self.assertTrue(all(row["endedMonotonicSeconds"] - row["startedMonotonicSeconds"] == 10 for row in windows))

    def test_nine_windows_require_real_trace_ingestion_without_drops(self):
        samples = {mode: [] for mode in ("off", "metrics", "full")}
        budget = environment.AdmissionBudget()
        with self.fake_clock(), patch.object(environment.time, "sleep", side_effect=self.sleep), \
             patch.object(environment, "pressure", return_value=psi()):
            for index in range(3):
                for mode in samples:
                    window = {"windowId": f"{index}-{mode}", "pairIndex": index, "enabled": mode}
                    window["admissionWaitSeconds"] = environment.quiet_admission(self.root, timeout=300, window=window, budget=budget)
                    with environment.observe_resources(self.root, window): self.clock += 10
                    samples[mode].append({"windowId": window["windowId"], "durationSeconds": 10,
                        "traceEvidence": {"enabled": True, "searchableTrace": True, "exportedSpans": 10, "failedSpans": 0, "droppedSpans": 0}})
        (self.root / "run-config.json").write_text(json.dumps({"appCpuQuota": 1, "appCpuSet": "0", "auxiliaryCpuSet": "runner-default"}))
        (self.root / "runner-context.json").write_text(json.dumps({"cpuAffinity": [0, 1, 2, 3]}))
        path = self.root / "ab-samples.json"
        path.write_text(json.dumps(samples))
        environment.verify_measurement_evidence(self.root)
        for key, value in [("enabled", False), ("searchableTrace", False), ("exportedSpans", 0), ("failedSpans", 1), ("droppedSpans", 1)]:
            changed = copy.deepcopy(samples); changed["full"][1]["traceEvidence"][key] = value
            path.write_text(json.dumps(changed))
            with self.assertRaises(OSError): environment.verify_measurement_evidence(self.root)

    def test_raw_measurement_evidence_must_exist_even_with_successful_scenarios(self):
        run = object.__new__(Run)
        run.root = self.root; run.source = SOURCE; run.args = SimpleNamespace(candidate="a" * 40)
        run.context = {"evidenceLocator": "https://github.com/owner/repo/actions/runs/123/attempts/2"}
        run.suite = "full"
        run.results = {name: {"status": "passed"} for name in ["https-auth-query", "tempo-cases-tenant", "monitoring-fault-isolation", "original-process-cpu", "default-observability-ab"]}
        self.assertFalse(run.finish())
        self.assertEqual(json.loads((self.root / "empirical-card.json").read_text())["empirical_evidence_status"], "unavailable")

    def test_six_window_certificate_rechecks_raw_pressure_and_completeness(self):
        samples = {"false": [], "true": []}
        budget = environment.AdmissionBudget()
        with self.fake_clock(), patch.object(environment.time, "sleep", side_effect=self.sleep), \
             patch.object(environment, "pressure", return_value=psi()):
            for index in range(3):
                for mode in ["false", "true"]:
                    window = {"windowId": f"{index}-{mode}", "pairIndex": index, "enabled": mode}
                    window["admissionWaitSeconds"] = environment.quiet_admission(self.root, timeout=300, window=window, budget=budget)
                    with environment.observe_resources(self.root, window): self.clock += 10
                    samples[mode].append({"windowId": window["windowId"], "durationSeconds": 10})
        (self.root / "run-config.json").write_text(json.dumps({"appCpuQuota": 1, "appCpuSet": "0", "auxiliaryCpuSet": "runner-default"}))
        (self.root / "runner-context.json").write_text(json.dumps({"cpuAffinity": [0, 1, 2, 3]}))
        (self.root / "ab-samples.json").write_text(json.dumps(samples))
        environment.verify_measurement_evidence(self.root, modes=("false", "true"))
        path = self.root / "resource-observer.jsonl"
        original = path.read_text()
        rows = [json.loads(row) for row in original.splitlines()]
        rows[1]["hostPressure"] = psi(cpu=3)
        rows[1]["eligible"] = False
        path.write_text("\n".join(json.dumps(row) for row in rows) + "\n")
        windows = json.loads((self.root / "measurement-windows.json").read_text())
        windows[0]["pressureExceededSamples"] = 1
        (self.root / "measurement-windows.json").write_text(json.dumps(windows))
        environment.verify_measurement_evidence(self.root, modes=("false", "true"))
        path.write_text("\n".join(original.splitlines()[:-1]) + "\n")
        with self.assertRaises(OSError): environment.verify_measurement_evidence(self.root, modes=("false", "true"))

    def test_ineligible_first_measurement_sample_blocks_window(self):
        with self.fake_clock(), patch.object(environment, "pressure", return_value=psi(cpu=3)):
            with self.assertRaisesRegex(OSError, "pressure_exceeded"):
                with environment.observe_resources(self.root, self.window):
                    pass


class CpuIsolationTests(unittest.TestCase):
    def test_layout_pins_app_and_leaves_helpers_on_runner_default_affinity(self):
        self.assertEqual(environment.measurement_cpu_layout({3, 1, 2, 0}), {"app": "0", "auxiliary": "runner-default"})
        self.assertEqual(environment.measurement_cpu_layout({3, 1, 2, 0}, selected="2"), {"app": "2", "auxiliary": "runner-default"})

    def test_selected_cpu_must_be_within_runner_affinity(self):
        with self.assertRaisesRegex(ValueError, "outside runner affinity"):
            environment.measurement_cpu_layout({0, 1}, selected="2")

    def test_layout_requires_a_helper_cpu(self):
        with self.assertRaisesRegex(ValueError, "at least two runner CPUs"):
            environment.measurement_cpu_layout({0})


class AdmissionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.sha = "a" * 40
        self.env = {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted", "RUNNER_TEMP": str(self.root), "GITHUB_REPOSITORY": "owner/repo", "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "2"}

    def context(self, root=None):
        with patch.object(environment.subprocess, "check_output", return_value=self.sha + "\n"), patch.object(Path, "read_text", return_value="model name: Fixture CPU\n"), patch.object(os, "sched_getaffinity", return_value={0, 1, 2, 3}, create=True):
            return environment.actions_context(SOURCE, root or self.root / "run", self.sha)

    def test_local_and_self_hosted_cannot_certify_performance(self):
        for variant in [{}, {**self.env, "RUNNER_ENVIRONMENT": "self-hosted"}]:
            with patch.dict(os.environ, variant, clear=True):
                with self.assertRaisesRegex(ValueError, "GitHub-hosted"):
                    self.context()

    def test_evidence_must_stay_in_runner_temporary_child(self):
        with patch.dict(os.environ, self.env, clear=True):
            for root in [self.root, self.root.parent / "unrelated"]:
                with self.assertRaisesRegex(ValueError, "RUNNER_TEMP"):
                    self.context(root)

    def test_wrong_checkout_is_rejected(self):
        with patch.dict(os.environ, self.env, clear=True), patch.object(environment.subprocess, "check_output", return_value="b" * 40):
            with self.assertRaisesRegex(ValueError, "Candidate SHA"):
                environment.actions_context(SOURCE, self.root / "run", self.sha)

    def test_run_locator_binds_attempt_and_candidate(self):
        with patch.dict(os.environ, self.env, clear=True):
            context = self.context()
        self.assertEqual(context["evidenceLocator"], "https://github.com/owner/repo/actions/runs/123/attempts/2")
        self.assertEqual(context["candidate"], self.sha)

    def test_full_suite_rejects_testbox_before_creating_state(self):
        args = SimpleNamespace(source=str(SOURCE), run=str(self.root / "run"), environment="shared-testbox", suite="full")
        with self.assertRaisesRegex(ValueError, "GitHub Actions"):
            Run(args)
        self.assertFalse((self.root / "run").exists())

    def test_full_suite_requires_an_immutable_prebuilt_image(self):
        for image in [None, "example:latest"]:
            args = SimpleNamespace(source=str(SOURCE), run=str(self.root / "run"), environment="github-actions", suite="full", candidate=self.sha, image=image)
            with patch.object(acceptance, "actions_context", return_value={"candidate":self.sha}):
                with self.assertRaisesRegex(ValueError, "prebuilt image ID"):
                    Run(args)
            self.assertFalse((self.root / "run").exists())


class ComparisonTests(unittest.TestCase):
    def samples(self, enabled_cpu=1.04, enabled_p95=1.04):
        return {"false": [{"cpuSecondsPerRequest": 1.0, "p95Seconds": 1.0} for _ in range(3)], "true": [{"cpuSecondsPerRequest": enabled_cpu, "p95Seconds": enabled_p95} for _ in range(3)]}

    def test_three_modes_share_one_budget_and_report_trace_increment(self):
        samples = {mode: [{"cpuSecondsPerRequest": value, "p95Seconds": value} for _ in range(3)]
                   for mode, value in [("off", 1.0), ("metrics", 1.04), ("full", 1.06)]}
        report = environment.comparison_report(samples)
        for metric in report["metrics"].values():
            self.assertTrue(metric["comparisonValid"])
            self.assertFalse(metric["withinBudget"])
            self.assertAlmostEqual(metric["traceIncrement"], 1.06 / 1.04 - 1)
        samples["metrics"][2]["p95Seconds"] = 2.0
        self.assertFalse(environment.comparison_report(samples)["metrics"]["p95Seconds"]["comparisonValid"])

    def test_both_stable_metrics_must_fit_budget(self):
        report = environment.comparison_report(self.samples())
        self.assertTrue(all(metric["withinBudget"] for metric in report["metrics"].values()))
        self.assertEqual(report["overheadLimit"], 0.05)

    def test_stable_excess_is_not_a_pass(self):
        for cpu, p95 in [(1.06, 1.0), (1.0, 1.06)]:
            report = environment.comparison_report(self.samples(cpu, p95))
            self.assertFalse(all(metric["withinBudget"] for metric in report["metrics"].values()))
            self.assertTrue(all(metric["comparisonValid"] for metric in report["metrics"].values()))

    def test_exact_five_percent_is_inclusive(self):
        report = environment.comparison_report(self.samples(1.05, 1.05))
        self.assertTrue(all(metric["withinBudget"] for metric in report["metrics"].values()))

    def test_unstable_baseline_never_reports_accepted_overhead(self):
        samples = self.samples(1.0, 1.0)
        samples["false"][2]["cpuSecondsPerRequest"] = 2.0
        metric = environment.comparison_report(samples)["metrics"]["cpuSecondsPerRequest"]
        self.assertFalse(metric["comparisonValid"])
        self.assertIsNone(metric["increase"])
        self.assertFalse(metric["withinBudget"])

    def test_incomplete_or_zero_windows_are_rejected(self):
        for incomplete in [True, False]:
            samples = self.samples()
            if incomplete: samples["true"].pop()
            else: samples["true"][0]["p95Seconds"] = 0
            with self.assertRaises(ValueError): environment.comparison_report(samples)


class CertificateTests(unittest.TestCase):
    def test_runtime_certificate_cannot_replace_full_empirical_card(self):
        with tempfile.TemporaryDirectory() as directory:
            run = object.__new__(Run)
            run.root = Path(directory)
            run.source = SOURCE
            run.args = SimpleNamespace(candidate="a" * 40)
            run.context = None
            run.suite = "runtime"
            run.results = {name: {"status": "passed"} for name in ["https-auth-query", "tempo-cases-tenant", "monitoring-fault-isolation", "original-process-cpu"]}
            self.assertTrue(run.finish())
            self.assertTrue((run.root / "runtime-card.json").exists())
            self.assertFalse((run.root / "empirical-card.json").exists())
            run.suite = "full"
            run.context = {"evidenceLocator": "https://github.com/owner/repo/actions/runs/123/attempts/2"}
            self.assertFalse(run.finish())
            card = json.loads((run.root / "empirical-card.json").read_text())
            self.assertEqual(card["empirical_evidence_status"], "failed")
            self.assertEqual(len(card), 7)


class WorkflowGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = contract.load_yaml(SOURCE / ".github/workflows/ci-observability-performance.yml")

    def verify(self, workflow):
        contract.require_observability_performance_contract(workflow)

    def test_current_workflow_fails_closed(self):
        self.verify(self.workflow)

    def test_self_hosted_measurement_is_rejected(self):
        workflow = copy.deepcopy(self.workflow)
        workflow["jobs"]["observability-performance"]["runs-on"] = ["self-hosted", "linux"]
        with self.assertRaises(contract.ContractError): self.verify(workflow)

    def test_missing_separate_image_producer_is_rejected(self):
        workflow = copy.deepcopy(self.workflow)
        workflow["jobs"]["observability-performance"]["needs"] = "build-pr-smoke-artifacts"
        with self.assertRaises(contract.ContractError): self.verify(workflow)

    def test_build_artifacts_does_not_depend_on_performance_result(self):
        self.assertNotIn("build", self.workflow["jobs"])

    def test_failures_must_still_upload_evidence(self):
        workflow = copy.deepcopy(self.workflow)
        step = next(x for x in workflow["jobs"]["observability-performance"]["steps"] if x["name"] == "Upload performance acceptance evidence")
        step.pop("if")
        with self.assertRaises(contract.ContractError): self.verify(workflow)

    def test_resource_unavailability_requires_the_classifier(self):
        workflow = copy.deepcopy(self.workflow)
        steps = workflow["jobs"]["observability-performance"]["steps"]
        steps[:] = [step for step in steps if step["name"] != "Classify performance acceptance"]
        with self.assertRaises(contract.ContractError): self.verify(workflow)

    def test_classifier_must_run_after_a_non_blocking_acceptance_step(self):
        workflow = copy.deepcopy(self.workflow)
        steps = workflow["jobs"]["observability-performance"]["steps"]
        acceptance = next(step for step in steps if step["name"] == "Run candidate-bound runtime and performance acceptance")
        acceptance["continue-on-error"] = False
        with self.assertRaises(contract.ContractError): self.verify(workflow)
        acceptance["continue-on-error"] = True
        classifier = next(step for step in steps if step["name"] == "Classify performance acceptance")
        classifier["if"] = "success()"
        with self.assertRaises(contract.ContractError): self.verify(workflow)

    def test_window_evidence_is_required_and_sensitive_artifacts_are_refused(self):
        for extra in [False, True]:
            workflow = copy.deepcopy(self.workflow)
            upload = next(x for x in workflow["jobs"]["observability-performance"]["steps"] if x["name"] == "Upload performance acceptance evidence")
            if extra:
                upload["with"]["path"] += "\n${{ runner.temp }}/observability-acceptance/private/*"
            else:
                upload["with"]["path"] = upload["with"]["path"].replace("${{ runner.temp }}/observability-acceptance/measurement-windows.json", "")
            with self.assertRaises(contract.ContractError): self.verify(workflow)

    def test_shortened_windows_are_rejected(self):
        workflow = copy.deepcopy(self.workflow)
        step = next(x for x in workflow["jobs"]["observability-performance"]["steps"] if x["name"] == "Run candidate-bound runtime and performance acceptance")
        step["run"] = step["run"].replace("--seconds 300", "--seconds 60")
        with self.assertRaises(contract.ContractError): self.verify(workflow)


class PerformanceDispositionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def write_case(self, status, scenarios):
        (self.root / "empirical-card.json").write_text(json.dumps({"empirical_evidence_status": status}) + "\n")
        (self.root / "scenarios.json").write_text(json.dumps(scenarios) + "\n")

    @staticmethod
    def passed_scenarios():
        return {
            "https-auth-query": {"status": "passed"},
            "tempo-cases-tenant": {"status": "passed"},
            "monitoring-fault-isolation": {"status": "passed"},
            "original-process-cpu": {"status": "passed"},
            "default-observability-ab": {
                "status": "unavailable",
                "error": "measured resource environment unavailable: pressure_exceeded",
            },
        }

    def test_pressure_unavailable_is_neutral(self):
        self.write_case("unavailable", self.passed_scenarios())
        self.assertEqual(classify_result(self.root, "failure"), "neutral-unavailable")

    def test_functional_failure_remains_blocking(self):
        scenarios = self.passed_scenarios()
        scenarios["https-auth-query"] = {"status": "failed", "error": "query mismatch"}
        self.write_case("failed", scenarios)
        with self.assertRaises(ClassificationError): classify_result(self.root, "failure")

    def test_missing_or_invalid_measurement_evidence_remains_blocking(self):
        scenarios = self.passed_scenarios()
        scenarios["default-observability-ab"] = {
            "status": "unavailable",
            "error": "measurement environment evidence is incomplete or invalid",
        }
        self.write_case("unavailable", scenarios)
        with self.assertRaises(ClassificationError): classify_result(self.root, "failure")

    def test_successful_acceptance_requires_successful_step(self):
        scenarios = {name: {"status": "passed"} for name in self.passed_scenarios()}
        self.write_case("passed", scenarios)
        self.assertEqual(classify_result(self.root, "success"), "passed")
        with self.assertRaises(ClassificationError): classify_result(self.root, "failure")

    def test_unstable_windows_are_neutral_only_when_raw_budget_is_within_limit(self):
        scenarios = self.passed_scenarios()
        report = {
            "metrics": {
                "cpuSecondsPerRequest": {"observedWithinBudget": True},
                "p95Seconds": {"observedWithinBudget": True},
            }
        }
        (self.root / "ab-summary.json").write_text(json.dumps(report) + "\n")
        scenarios["default-observability-ab"] = {
            "status": "unavailable",
            "error": "unstable measurement windows",
        }
        self.write_case("unavailable", scenarios)
        self.assertEqual(classify_result(self.root, "failure"), "neutral-unavailable")

    def test_unstable_windows_with_raw_budget_overrun_remain_blocking(self):
        scenarios = self.passed_scenarios()
        report = {
            "metrics": {
                "cpuSecondsPerRequest": {"observedWithinBudget": False},
                "p95Seconds": {"observedWithinBudget": True},
            }
        }
        (self.root / "ab-summary.json").write_text(json.dumps(report) + "\n")
        scenarios["default-observability-ab"] = {
            "status": "unavailable",
            "error": "unstable measurement windows",
        }
        self.write_case("unavailable", scenarios)
        with self.assertRaises(ClassificationError): classify_result(self.root, "failure")


if __name__ == "__main__":
    unittest.main()
