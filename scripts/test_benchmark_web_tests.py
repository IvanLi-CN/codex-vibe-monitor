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
    environment = {"runner": "same machine", "bun": "1.3.14"}
    return {"group": name, "sha": name.lower() * 40, "manifest_digest": "frozen",
            "environment": environment,
            "phase_environments": {phase: dict(environment) for phase in MODULE.WORKLOAD},
            "wall_seconds": seconds, "status": status, "score": int(status == "passed"),
            "phases": [{"name": phase, "status": "passed", "wall_seconds": 1} for phase in MODULE.WORKLOAD],
            "reports": {"success": True, "unit": {"cases": [case]}, "storybook": {"cases": [case]},
                        "browser": {"pr": copy.deepcopy(browser), "main": copy.deepcopy(browser)}}}


class AcceptanceTests(unittest.TestCase):
    def test_budget_is_one_fixed_three_minute_group_and_ten_minute_ab_total(self):
        self.assertEqual(MODULE.GROUP_SECONDS, 180)
        self.assertEqual(MODULE.AB_TOTAL_SECONDS, 600)

    def test_local_and_agent_vm_execution_are_rejected(self):
        for value in ("", "false"):
            with self.subTest(value=value), patch.dict(MODULE.os.environ, {"GITHUB_ACTIONS": value}):
                with self.assertRaisesRegex(SystemExit, "only in GitHub Actions"):
                    MODULE.actions_only()

    def test_old_timeout_new_complete_immediately_passes(self):
        result = MODULE.compare(group("A", 180.01, "timeout"), group("B", 179.9), manifest())
        self.assertTrue(result["passed"])
        self.assertTrue(result["delta_is_lower_bound"])
        self.assertEqual(result["reason"], "baseline_timeout_candidate_complete")

    def test_only_an_expected_baseline_timeout_keeps_the_producer_successful(self):
        self.assertEqual(MODULE.phase_exit_code("A", "timeout"), 0)
        for group_name, status in (("B", "timeout"), ("A", "failed"), ("B", "failed"), ("B", "environment_failed")):
            with self.subTest(group=group_name, status=status):
                self.assertEqual(MODULE.phase_exit_code(group_name, status), 1)

    def test_both_complete_require_ten_percent_and_five_seconds(self):
        for a, b, expected in ((100, 90, True), (100, 91, False), (40, 36, False), (50, 45, True), (90, 100, False)):
            with self.subTest(a=a, b=b):
                result = MODULE.compare(group("A", a), group("B", b), manifest())
                self.assertEqual(result["passed"], expected)
                self.assertEqual(result["delta_seconds"], a - b)

    def test_candidate_timeout_failure_or_incomplete_never_passes(self):
        for status in ("timeout", "failed", "environment_failed", "running"):
            with self.subTest(status=status):
                self.assertFalse(MODULE.compare(group("A", 181, "timeout"), group("B", 50, status), manifest())["passed"])
        for seconds in (0, -1, 180.001, float("nan"), float("inf")):
            self.assertFalse(MODULE.compare(group("A"), group("B", seconds), manifest())["passed"])
        b = group("B", 50)
        b["phases"].pop()
        self.assertFalse(MODULE.compare(group("A"), b, manifest())["passed"])

    def test_infrastructure_failure_is_not_an_old_timeout(self):
        self.assertEqual(MODULE.compare(group("A", 180, "environment_failed"), group("B", 50), manifest())["reason"], "baseline_failed_or_missing")
        self.assertFalse(MODULE.compare(group("A", 1, "timeout"), group("B", 50), manifest())["passed"])

    def test_a_plus_b_wall_budget_is_enforced(self):
        result = MODULE.compare(group("A", 150), group("B", 451), manifest())
        self.assertEqual(result["reason"], "ab_total_wall_budget_exceeded")
        self.assertEqual(result["total_wall_seconds"], 601)

    def test_source_manifest_and_environment_must_match(self):
        for key, value in (("sha", "wrong"), ("group", "A"), ("manifest_digest", "different"),
                           ("environment", {"runner": "different"}),
                           ("phase_environments", {"unit": {"runner": "different"}})):
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

    def test_demo_route_matrix_requires_terminal_and_request_contracts(self):
        audit = MODULE.demo_coverage_audit(MODULE.ROOT)
        self.assertTrue(audit["verified"], audit["issues"])

    def test_timeline_wait_tuning_is_the_only_normalized_e2e_change(self):
        baseline = subprocess.check_output(
            ["git", "show", "76beb053ac529bde47961dddb1e14b4d176700f8:web/tests/e2e/dashboard-timeline-capacity.spec.ts"],
            text=True,
        )
        candidate = (MODULE.ROOT / "web/tests/e2e/dashboard-timeline-capacity.spec.ts").read_text()
        path = MODULE.ROOT / "web/tests/e2e/dashboard-timeline-capacity.spec.ts"
        self.assertEqual(
            MODULE.normalize_approved_e2e_performance_change(path, baseline),
            MODULE.normalize_approved_e2e_performance_change(path, candidate),
        )
        self.assertNotEqual(baseline, candidate)
        altered = candidate.replace('expect(diagnostics?.releaseFailures).toBe(0);', 'expect(diagnostics?.releaseFailures).toBe(1);')
        self.assertNotEqual(
            MODULE.normalize_approved_e2e_performance_change(path, baseline),
            MODULE.normalize_approved_e2e_performance_change(path, altered),
        )

    def test_timeline_cadence_proof_remains_in_two_client_test(self):
        candidate = (MODULE.ROOT / "web/tests/e2e/dashboard-timeline-capacity.spec.ts").read_text()
        self.assertIn(
            "startedAt - diagnostics.traversalStarts[index - 1]! >= MINIMUM_REFRESH_INTERVAL_MS",
            candidate,
        )

    def test_matrix_phase_paths_are_rebased_to_the_hosted_job(self):
        manifest = {"source_roots": {"A": "/old/A"}, "output_root": "/old/output"}
        phase = MODULE.materialize_phase(
            MODULE.Phase("unit", "cd /old/A/web\nbun --outputFile=/old/output/A/reports/unit.json"),
            manifest,
            "A",
            Path("/new/A"),
            Path("/new/output/results/A/unit"),
        )
        self.assertIn("/new/A/web", phase.command)
        self.assertIn("/new/output/results/A/unit/reports/unit.json", phase.command)
        self.assertNotIn("/old/", phase.command)

    def test_paired_browser_phases_use_disjoint_ports_for_a_and_b(self):
        phase = MODULE.Phase(
            "e2e-main",
            "bun run dev -- --port 60080\nE2E_BASE_URL=http://127.0.0.1:60083\nE2E_DEMO_BASE_URL=http://127.0.0.1:60084",
        )
        baseline = MODULE.rebase_phase(phase, Path("/old"), Path("/new/A"), "A")
        candidate = MODULE.rebase_phase(phase, Path("/old"), Path("/new/B"), "B")
        self.assertIn("--port 60280", baseline.command)
        self.assertIn("127.0.0.1:60283", baseline.command)
        self.assertIn("127.0.0.1:60284", baseline.command)
        self.assertIn("--port 60480", candidate.command)
        self.assertIn("127.0.0.1:60483", candidate.command)
        self.assertIn("127.0.0.1:60484", candidate.command)
        self.assertNotEqual(baseline.command, candidate.command)

    def test_phase_mode_rebases_materialized_browser_ports_for_b(self):
        phase = MODULE.Phase(
            "e2e-main",
            "bun run dev -- --port 60080\nE2E_BASE_URL=http://127.0.0.1:60083\nE2E_DEMO_BASE_URL=http://127.0.0.1:60084",
        )
        rebased = MODULE.rebase_phase_ports(phase, "B")
        self.assertIn("--port 60480", rebased.command)
        self.assertIn("127.0.0.1:60483", rebased.command)
        self.assertIn("127.0.0.1:60484", rebased.command)
        self.assertNotIn("127.0.0.1:6008", rebased.command)

    def test_aggregate_group_requires_every_phase_and_uses_max_wall_time(self):
        data = manifest()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for group_name in ("A", "B"):
                for index, phase_name in enumerate(MODULE.WORKLOAD, start=1):
                    record = {"name": phase_name, "group": group_name, "status": "passed", "wall_seconds": index,
                              "cpu_seconds": 1, "children_peak_rss_kb_cumulative": 10,
                              "started_unix": 100 + index, "finished_unix": 100 + index + index,
                              "environment": {"bun": "1.3.14", "runner_image": "ubuntu-24.04",
                                               "runner_image_version": "fixed", "kernel": "fixed",
                                               "chromium_descriptor": []}}
                    if phase_name == "unit":
                        record["report"] = {"unit": {"success": True,
                            "suites": {suite: {"seconds": 1, "cases": 1, "status": "passed"} for suite in MODULE.SLOW_SUITES},
                            "cases": [{"file": "src/real.test.tsx", "name": "keeps behavior", "status": "passed"}]}}
                    elif phase_name == "storybook-a11y":
                        record["report"] = {"storybook": {"success": True, "cases": [{"file": "src/real.test.tsx", "name": "keeps behavior", "status": "passed"}]}}
                    elif phase_name in ("e2e-pr", "e2e-main"):
                        profile = phase_name.removeprefix("e2e-")
                        specs = MODULE.E2E_SPECS if profile == "main" else set(MODULE.E2E_SPECS) - {"dashboard-timeline-capacity.spec.ts"}
                        record["report"] = {"browser": {profile: {"success": True,
                            "cases": [{"file": spec, "name": spec, "passed": True} for spec in specs]}}}
                    MODULE.write_json(root / group_name / phase_name / "result.json", record)
            result = MODULE.aggregate_group("A", data, root)
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["wall_seconds"], len(MODULE.WORKLOAD))
            self.assertEqual(result["phase_wall_seconds_sum"], sum(range(1, len(MODULE.WORKLOAD) + 1)))
            self.assertTrue(result["reports"]["success"])
            (root / "A" / MODULE.WORKLOAD[0] / "result.json").unlink()
            self.assertEqual(MODULE.aggregate_group("A", data, root)["status"], "failed")


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
        for job_id in ("prepare", "measure-phase", "aggregate"):
            job = workflow["jobs"][job_id]
            checkouts = [step for step in job["steps"] if step.get("uses", "").startswith("actions/checkout@")]
            self.assertEqual([step["with"]["path"] for step in checkouts],
                             {"prepare": ["harness", "A", "B"],
                              "measure-phase": ["harness", "A", "B"],
                              "aggregate": ["harness"]}[job_id])
            self.assertTrue(all(step["with"]["persist-credentials"] is False for step in checkouts))
            self.assertNotIn("continue-on-error", job)
        self.assertEqual(workflow["jobs"]["measure-phase"]["strategy"]["matrix"]["phase"], list(MODULE.WORKLOAD))
        paired = next(step["run"] for step in workflow["jobs"]["measure-phase"]["steps"]
                      if step["name"] == "Run paired workloads with independent hard deadlines")
        self.assertIn("--group A", paired)
        self.assertIn("--group B", paired)
        self.assertIn("a_status=$?", paired)
        self.assertIn("b_status=$?", paired)
        self.assertIn("exit $(( a_status || b_status ))", paired)
        prepare = workflow["jobs"]["prepare"]["steps"]
        pack = next(step["run"] for step in prepare if step["name"] == "Install identical dependency surface without building or testing")
        self.assertIn('tar -cf "$RUNNER_TEMP/web-test-ab-browsers.tar"', pack)
        upload = next(step["with"] for step in prepare if step["name"] == "Upload run-bound dependencies")
        self.assertIn("/web-test-ab-browsers.tar", upload["path"])
        self.assertNotIn("/web-test-ab/browsers", upload["path"])
        restore = next(step["run"] for step in workflow["jobs"]["measure-phase"]["steps"] if step["name"] == "Restore isolated dependency directories")
        self.assertIn('tar -xf "$RUNNER_TEMP/web-test-ab-inputs/web-test-ab-browsers.tar"', restore)
        aggregate = workflow["jobs"]["aggregate"]
        self.assertEqual(aggregate["needs"], ["prepare", "measure-phase"])
        self.assertEqual(aggregate["if"], "always()")
        self.assertIn("benchmark-web-tests.py aggregate", next(step["run"] for step in aggregate["steps"] if step["name"] == "Aggregate complete A/B evidence"))


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
