#!/usr/bin/env python3
"""Synthetic contract tests only: never execute an A/B performance workload here."""

import copy
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

SPEC = importlib.util.spec_from_file_location("benchmark_web_tests", Path(__file__).with_name("benchmark-web-tests.py"))
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


def manifest():
    return {"digest": "frozen", "identities": {"A": {"sha": "a" * 40}, "B": {"sha": "b" * 40}},
            "workload": list(MODULE.WORKLOAD), "unimplemented_slow_suites": [],
            "demo_coverage_audit": {"verified": True, "issues": []}}


def group(name, seconds=100, status="passed"):
    case = {"file": "src/real.test.tsx", "name": "keeps behavior", "status": "passed"}
    browser = {"success": True, "cases": [{"file": "demo-runtime.spec.ts", "name": "real route", "passed": True}]}
    return {"group": name, "sha": name.lower() * 40, "manifest_digest": "frozen",
            "environment": {"runner": "same machine", "bun": "1.3.14"},
            "wall_seconds": seconds, "status": status, "score": int(status == "passed"),
            "phases": [{"name": phase, "status": "passed", "wall_seconds": 1} for phase in MODULE.WORKLOAD],
            "reports": {"success": True, "unit": {"cases": [case]}, "storybook": {"cases": [case]},
                        "browser": {"pr": copy.deepcopy(browser), "main": copy.deepcopy(browser)}}}


class AcceptanceTests(unittest.TestCase):
    def test_budget_is_one_fixed_ten_minute_group(self):
        self.assertEqual(MODULE.GROUP_SECONDS, 600)

    def test_local_and_agent_vm_execution_are_rejected(self):
        for value in ("", "false"):
            with self.subTest(value=value), patch.dict(MODULE.os.environ, {"GITHUB_ACTIONS": value}):
                with self.assertRaisesRegex(SystemExit, "only in GitHub Actions"):
                    MODULE.actions_only()

    def test_old_timeout_new_complete_immediately_passes(self):
        result = MODULE.compare(group("A", 600.01, "timeout"), group("B", 599.9), manifest())
        self.assertTrue(result["passed"])
        self.assertTrue(result["delta_is_lower_bound"])
        self.assertEqual(result["reason"], "baseline_timeout_candidate_complete")

    def test_both_complete_require_ten_percent_and_five_seconds(self):
        for a, b, expected in ((100, 90, True), (100, 91, False), (40, 36, False), (50, 45, True), (90, 100, False)):
            with self.subTest(a=a, b=b):
                result = MODULE.compare(group("A", a), group("B", b), manifest())
                self.assertEqual(result["passed"], expected)
                self.assertEqual(result["delta_seconds"], a - b)

    def test_candidate_timeout_failure_or_incomplete_never_passes(self):
        for status in ("timeout", "failed", "environment_failed", "running"):
            with self.subTest(status=status):
                self.assertFalse(MODULE.compare(group("A", 601, "timeout"), group("B", 50, status), manifest())["passed"])
        for seconds in (0, -1, 600.001, float("nan"), float("inf")):
            self.assertFalse(MODULE.compare(group("A"), group("B", seconds), manifest())["passed"])
        b = group("B", 50)
        b["phases"].pop()
        self.assertFalse(MODULE.compare(group("A"), b, manifest())["passed"])

    def test_infrastructure_failure_is_not_an_old_timeout(self):
        self.assertEqual(MODULE.compare(group("A", 600, "environment_failed"), group("B", 50), manifest())["reason"], "baseline_failed_or_missing")
        self.assertFalse(MODULE.compare(group("A", 1, "timeout"), group("B", 50), manifest())["passed"])

    def test_source_manifest_and_environment_must_match(self):
        for key, value in (("sha", "wrong"), ("group", "A"), ("manifest_digest", "different"), ("environment", {"runner": "different"})):
            b = group("B", 50)
            b[key] = value
            self.assertEqual(MODULE.compare(group("A"), b, manifest())["reason"], "incomparable_identity_or_environment")

    def test_removing_or_skipping_existing_unit_case_fails(self):
        for cases in ([], [{"file": "src/real.test.tsx", "name": "keeps behavior", "status": "pending"}]):
            b = group("B", 50)
            b["reports"]["unit"]["cases"] = cases
            self.assertEqual(MODULE.compare(group("A"), b, manifest())["reason"], "candidate_removed_unit_cases")

    def test_removing_storybook_or_browser_case_fails(self):
        for key in ("storybook", "pr", "main"):
            b = group("B", 50)
            target = b["reports"][key] if key == "storybook" else b["reports"]["browser"][key]
            target["cases"] = []
            self.assertFalse(MODULE.compare(group("A"), b, manifest())["passed"])


class ExecutionContractTests(unittest.TestCase):
    def test_step_exit_is_scoped_to_its_original_shell(self):
        workflow = {"jobs": {"e2e": {"steps": [
            {"name": "ready", "run": "exit 0"}, {"name": "test", "run": "printf actual-test"}]}}}
        with patch.object(MODULE, "load_workflow", return_value=workflow):
            command = MODULE.named_steps(Path.cwd(), "pr", "e2e", ("ready", "test"))
        # A shell-shape assertion, not an A/B or frontend workload execution.
        self.assertEqual(subprocess.check_output(["bash", "-euo", "pipefail", "-c", command], text=True, timeout=2), "actual-test")

    def test_reports_do_not_change_worker_or_retry_policy_or_scheduling(self):
        command = "bun run test:e2e -- demo-runtime.spec.ts --workers=1 &\nwait $!\n"
        actual = MODULE.instrument_playwright(command, Path("/reports"))
        self.assertIn("--workers=1 &\nwait $!", actual)
        self.assertNotIn("--retries", actual)
        self.assertIn("PLAYWRIGHT_JSON_OUTPUT_NAME=", actual)

    def test_deadline_does_not_restart_for_next_phase(self):
        with tempfile.TemporaryDirectory() as temporary:
            process = MagicMock(pid=12345)
            process.wait.return_value = 0
            with patch.object(MODULE.time, "monotonic", side_effect=[590, 590, 590, 595, 595, 601]), \
                    patch.object(MODULE.subprocess, "Popen", return_value=process) as popen, \
                    patch.object(MODULE.os, "killpg") as kill:
                first = MODULE.execute_phase(MODULE.Phase("first", "test"), Path(temporary), Path(temporary), 600, {})
                second = MODULE.execute_phase(MODULE.Phase("second", "test"), Path(temporary), Path(temporary), 600, {})
            self.assertEqual(first["status"], "passed")
            self.assertEqual(second["status"], "timeout")
            self.assertEqual(popen.call_count, 1)
            process.wait.assert_called_once_with(timeout=10)
            kill.assert_called_once_with(12345, MODULE.signal.SIGKILL)

    def test_timeout_kills_the_whole_process_group(self):
        with tempfile.TemporaryDirectory() as temporary:
            process = MagicMock(pid=12345)
            process.wait.side_effect = [subprocess.TimeoutExpired("test", 1), 0]
            with patch.object(MODULE.time, "monotonic", return_value=599), \
                    patch.object(MODULE.subprocess, "Popen", return_value=process), \
                    patch.object(MODULE.os, "killpg") as kill:
                result = MODULE.execute_phase(MODULE.Phase("timeout", "test"), Path(temporary), Path(temporary), 600, {})
            self.assertEqual(result["status"], "timeout")
            self.assertEqual(result["returncode"], 124)
            self.assertTrue(all(call.args == (12345, MODULE.signal.SIGKILL) for call in kill.call_args_list))

    def test_whole_pr_workload_and_both_source_schedules_are_included(self):
        with tempfile.TemporaryDirectory() as temporary:
            phases = MODULE.phases(MODULE.ROOT, Path(temporary))
        self.assertEqual(tuple(phase.name for phase in phases), MODULE.WORKLOAD)
        self.assertIn("test_benchmark_web_tests.py", phases[-1].command)
        self.assertIn("assemble-pages-site.sh", phases[3].command)
        self.assertIn("--project=fixtures-functional", phases[4].command)
        self.assertIn("--workers=1", phases[4].command)
        self.assertIn("--workers=2", phases[5].command)
        self.assertIn("--project=timeline-capacity", phases[5].command)

    def test_known_landmark_only_migration_is_not_a_coverage_certificate(self):
        self.assertFalse(MODULE.demo_coverage_audit(MODULE.ROOT)["verified"])


class ReporterTests(unittest.TestCase):
    def test_vitest_reports_keep_existing_skips_visible_and_reject_failures(self):
        report = {"success": True, "testResults": [{"name": "/A/web/src/test.tsx", "status": "passed", "startTime": 1000, "endTime": 2500,
                  "assertionResults": [{"fullName": "existing case", "status": "passed"}, {"fullName": "existing skip", "status": "pending"}]}]}
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "unit.json"
            path.write_text(json.dumps(report))
            result = MODULE.vitest_results(path)
            self.assertTrue(result["success"])
            self.assertEqual(result["suites"]["src/test.tsx"]["seconds"], 1.5)
            report["testResults"][0]["assertionResults"][0]["status"] = "failed"
            path.write_text(json.dumps(report))
            self.assertFalse(MODULE.vitest_results(path)["success"])

    def test_playwright_retry_skip_failure_or_empty_report_is_not_success(self):
        for status, retry, expected in (("passed", 0, True), ("passed", 1, False), ("skipped", 0, False), ("failed", 0, False)):
            with self.subTest(status=status, retry=retry), tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary)
                report = {"suites": [{"title": "suite", "specs": [{"title": "case", "file": "test.spec.ts", "tests": [
                    {"status": "expected", "results": [{"status": status, "retry": retry}]}]}]}]}
                (path / "playwright-1.json").write_text(json.dumps(report))
                self.assertEqual(MODULE.playwright_results(path)["success"], expected)
        with tempfile.TemporaryDirectory() as temporary:
            self.assertFalse(MODULE.playwright_results(Path(temporary))["success"])


class WorkflowTests(unittest.TestCase):
    def test_dispatch_is_manual_exclusive_and_keeps_existing_timeouts(self):
        workflow = MODULE.load_workflow(MODULE.ROOT, "observability-performance")
        checker = sys.modules["web_ab_contract"]
        path = MODULE.ROOT / ".github/workflows/ci-observability-performance.yml"
        with patch.object(checker, "load_yaml", return_value=workflow):
            checker.validate_observability_performance(path)
        for job_id, field, value in (("frontend-test-ab", "if", "always()"),
                                      ("frontend-test-ab", "with", {}),
                                      ("observability-performance-image", "if", "github.event_name == 'workflow_dispatch'")):
            changed = copy.deepcopy(workflow)
            changed["jobs"][job_id][field] = value
            with patch.object(checker, "load_yaml", return_value=changed), self.assertRaises(checker.ContractError):
                checker.validate_observability_performance(path)
        self.assertEqual(workflow["jobs"]["observability-performance-image"]["timeout-minutes"], 15)
        self.assertEqual(workflow["jobs"]["observability-performance"]["timeout-minutes"], 60)

    def test_new_workflow_has_no_automatic_event_or_shared_source_tree(self):
        MODULE.load_workflow(MODULE.ROOT, "pr")
        checker = sys.modules["web_ab_contract"]
        workflow = checker.load_yaml(MODULE.ROOT / ".github/workflows/web-test-ab.yml")
        self.assertEqual(set(checker.mapping_get(workflow, "on")), {"workflow_dispatch", "workflow_call"})
        for job in workflow["jobs"].values():
            checkouts = [step for step in job["steps"] if step.get("uses", "").startswith("actions/checkout@")]
            self.assertEqual([step["with"]["path"] for step in checkouts], ["harness", "A", "B"])
            self.assertTrue(all(step["with"]["persist-credentials"] is False for step in checkouts))
            self.assertNotIn("continue-on-error", job)


class CleanupTests(unittest.TestCase):
    def test_rejects_unrelated_or_running_workflow(self):
        for status, event, path in (("in_progress", "workflow_dispatch", ".github/workflows/web-test-ab.yml"),
                                    ("completed", "push", ".github/workflows/web-test-ab.yml"),
                                    ("completed", "workflow_dispatch", ".github/workflows/ci-pr.yml")):
            with patch.object(MODULE, "command_output", return_value=json.dumps({"status": status, "event": event, "path": path})), \
                    patch.object(MODULE.subprocess, "run") as run:
                with self.assertRaises(ValueError):
                    MODULE.cleanup("123", Path("/not-created"), "owner/repo")
                run.assert_not_called()

    def test_export_and_verify_evidence_before_deleting_exact_run(self):
        with tempfile.TemporaryDirectory() as temporary:
            archive = Path(temporary) / "archive"
            calls = []
            def execute(command, **kwargs):
                calls.append(command)
                if "download" in command:
                    (archive / "summary.json").write_text('{"run_id":"123"}')
            with patch.object(MODULE, "command_output", return_value=json.dumps({"status": "completed", "event": "workflow_dispatch", "path": ".github/workflows/web-test-ab.yml"})), \
                    patch.object(MODULE.subprocess, "run", side_effect=execute):
                MODULE.cleanup("123", archive, "owner/repo")
            self.assertIn("download", calls[0])
            self.assertEqual(calls[1][-1], "repos/owner/repo/actions/runs/123")
            self.assertTrue((archive / "github-run.json").is_file())


if __name__ == "__main__":
    unittest.main()
