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
spec = importlib.util.spec_from_file_location("performance_gate_contract", SOURCE / ".github/scripts/check_quality_gates_contract.py")
contract = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = contract
spec.loader.exec_module(contract)


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
            run.results = {name: {"status": "passed"} for name in ["https-auth-query", "monitoring-fault-isolation", "original-process-cpu"]}
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
        cls.workflow = contract.load_yaml(SOURCE / ".github/workflows/ci-pr.yml")

    def verify(self, workflow):
        contract.require_observability_performance_contract(workflow, workflow["jobs"]["build"])

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

    def test_ignored_or_skipped_performance_result_is_rejected(self):
        for skip in [True, False]:
            workflow = copy.deepcopy(self.workflow)
            step = next(x for x in workflow["jobs"]["build"]["steps"] if x["name"] == "Verify observability performance budget")
            if skip: step["if"] = "failure()"
            else: step["run"] = "echo accepted"
            with self.assertRaises(contract.ContractError): self.verify(workflow)

    def test_failures_must_still_upload_evidence(self):
        workflow = copy.deepcopy(self.workflow)
        step = next(x for x in workflow["jobs"]["observability-performance"]["steps"] if x["name"] == "Upload performance acceptance evidence")
        step.pop("if")
        with self.assertRaises(contract.ContractError): self.verify(workflow)

    def test_shortened_windows_are_rejected(self):
        workflow = copy.deepcopy(self.workflow)
        step = next(x for x in workflow["jobs"]["observability-performance"]["steps"] if x["name"] == "Run candidate-bound runtime and performance acceptance")
        step["run"] = step["run"].replace("--seconds 300", "--seconds 60")
        with self.assertRaises(contract.ContractError): self.verify(workflow)


if __name__ == "__main__":
    unittest.main()
