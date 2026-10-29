#!/usr/bin/env python3
"""Unit tests for the PostgreSQL acceptance runner contract."""

from __future__ import annotations

import importlib.util
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch


REPO_ROOT = Path(__file__).resolve().parents[1]
RUNNER_PATH = REPO_ROOT / "scripts" / "postgres-acceptance" / "run.py"


def load_runner():
    spec = importlib.util.spec_from_file_location("postgres_acceptance_runner", RUNNER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load PostgreSQL acceptance runner")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PostgresAcceptanceRunnerTests(unittest.TestCase):
    def test_automatic_acceptance_runs_only_the_functional_suite(self) -> None:
        for filename in ("ci-postgresql-acceptance.yml", "ci-main.yml"):
            workflow = (REPO_ROOT / ".github/workflows" / filename).read_text()
            self.assertIn("bash scripts/postgres-acceptance/run-functional-suite.sh", workflow)
            self.assertIn("python3 scripts/postgres-acceptance/run.py", workflow)
            self.assertIn("bash scripts/postgres-acceptance/validate-identity.sh", workflow)
            self.assertNotIn("run-live-suite.sh", workflow)

    def test_identity_requires_existing_ancestor_and_exact_head(self) -> None:
        validator = REPO_ROOT / "scripts/postgres-acceptance/validate-identity.sh"
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            env = dict(os.environ, GIT_AUTHOR_NAME="Test", GIT_AUTHOR_EMAIL="test@example.invalid",
                       GIT_COMMITTER_NAME="Test", GIT_COMMITTER_EMAIL="test@example.invalid")
            tree = subprocess.check_output(["git", "mktree"], cwd=root, input=b"", env=env).decode().strip()
            def commit(message: str, parent: str | None = None) -> str:
                command = ["git", "commit-tree", tree, "-m", message]
                if parent:
                    command += ["-p", parent]
                return subprocess.check_output(command, cwd=root, env=env).decode().strip()
            base = commit("base")
            candidate = commit("candidate", base)
            unrelated = commit("unrelated")
            subprocess.run(["git", "update-ref", "HEAD", candidate], cwd=root, check=True, env=env)
            def validate(base_sha: str, candidate_sha: str) -> int:
                return subprocess.run(["bash", str(validator)], cwd=root,
                    env=dict(env, BASE_SHA=base_sha, CANDIDATE_SHA=candidate_sha),
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode
            self.assertEqual(validate(base, candidate), 0)
            args = self.runner.argparse.Namespace(repo_root=root, base_sha=base, candidate_sha=candidate)
            self.assertTrue(self.runner.validate_identity(args))
            for bad_base, bad_candidate in (("bad", candidate), ("0" * 40, candidate),
                                             (unrelated, candidate), (base, base)):
                self.assertNotEqual(validate(bad_base, bad_candidate), 0)
                args.base_sha, args.candidate_sha = bad_base, bad_candidate
                self.assertFalse(self.runner.validate_identity(args))

    def test_audited_backend_selectors_are_explicit_and_exist(self) -> None:
        names = (REPO_ROOT / "scripts/postgres-acceptance/non-pressure-backend-tests.txt").read_text().splitlines()
        self.assertEqual(len(names), 96)
        self.assertEqual(len(set(names)), len(names))
        for name in names:
            self.assertRegex(name, r"^(?:tests|upstream_accounts::tests)::(?:lightweight|stateful_sqlite|archive_file_io)::[a-z0-9_]+::[a-z0-9_]+$")
            self.assertNotRegex(name, r"(?i)pressure|stress|performance|benchmark")
            parts = name.split("::")
            if parts[0] == "tests":
                source = REPO_ROOT / "src/tests" / parts[1] / f"{parts[2]}.rs"
            else:
                source = REPO_ROOT / "src/upstream_accounts/tests" / parts[2] / f"{parts[3]}.rs"
            self.assertRegex(source.read_text(), rf"\bfn\s+{re.escape(parts[-1])}\s*\(")

    @classmethod
    def setUpClass(cls) -> None:
        cls.runner = load_runner()

    def invoke(self, root: Path, command: list[str], database_url: str | None, *, identity_valid: bool = True) -> tuple[int, dict]:
        output = root / "report.json"
        evidence = root / "evidence"
        environment = dict(os.environ)
        environment.pop("POSTGRES_TEST_DATABASE_URL", None)
        if database_url is not None:
            environment["POSTGRES_TEST_DATABASE_URL"] = database_url
        argv = [
            str(RUNNER_PATH),
            "--output",
            str(output),
            "--evidence-dir",
            str(evidence),
            "--repo-root",
            str(REPO_ROOT),
            "--base-sha",
            "1" * 40,
            "--candidate-sha",
            "2" * 40,
            "--scope",
            "functional-recovery",
            "--",
            *command,
        ]
        with patch.dict(os.environ, environment, clear=True), patch.object(sys, "argv", argv), patch.object(self.runner, "validate_identity", return_value=identity_valid):
            exit_code = self.runner.main()
        return exit_code, json.loads(output.read_text(encoding="utf-8"))

    def test_invalid_identity_blocks_before_running_the_command(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            with patch.object(self.runner.subprocess, "Popen") as popen:
                exit_code, report = self.invoke(Path(temporary_directory), [sys.executable, "-c", "pass"], "postgresql://postgres@", identity_valid=False)
        self.assertEqual(exit_code, 78)
        self.assertEqual(report["reason"]["code"], "acceptance_identity_invalid")
        self.assertFalse(report["raw_result"]["executed"])
        popen.assert_not_called()

    def test_missing_database_url_is_blocked_and_not_run(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            exit_code, report = self.invoke(
                Path(temporary_directory),
                [sys.executable, "-c", "raise SystemExit(99)"],
                None,
            )
        self.assertEqual(exit_code, 78)
        self.assertEqual(report["status"], "blocked")
        self.assertEqual(report["result"], "not-run")
        self.assertFalse(report["raw_result"]["executed"])
        self.assertEqual(report["reason"]["code"], "postgres_database_url_missing")

    def test_unreachable_database_is_blocked_and_not_run(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            with patch.object(self.runner.socket, "create_connection", side_effect=OSError("down")):
                exit_code, report = self.invoke(
                    Path(temporary_directory),
                    [sys.executable, "-c", "raise SystemExit(99)"],
                    "postgresql://postgres:secret@127.0.0.1:5432/test",
                )
        self.assertEqual(exit_code, 78)
        self.assertEqual(report["status"], "blocked")
        self.assertEqual(report["result"], "not-run")
        self.assertEqual(report["reason"]["code"], "postgres_service_unavailable")
        self.assertNotIn("secret", json.dumps(report))

    def test_success_records_raw_result_and_redacts_database_url(self) -> None:
        database_url = "postgresql://postgres:secret@"
        with tempfile.TemporaryDirectory() as temporary_directory:
            exit_code, report = self.invoke(
                Path(temporary_directory),
                [sys.executable, "-c", "print('acceptance ok')"],
                database_url,
            )
            evidence = Path(report["raw_result"]["stdout_file"])
            stdout = evidence.read_text(encoding="utf-8")
        self.assertEqual(exit_code, 0)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["result"], "passed")
        self.assertEqual(report["scope"], "functional-recovery")
        self.assertTrue(report["raw_result"]["executed"])
        self.assertIn("acceptance ok", stdout)
        self.assertNotIn("secret", json.dumps(report))

    def test_success_passes_explicit_identity_and_evidence_environment(self) -> None:
        command = [
            sys.executable,
            "-c",
            (
                "import json, os; print(json.dumps({k: os.environ[k] for k in "
                "('BASE_SHA', 'CANDIDATE_SHA', 'POSTGRES_ACCEPTANCE_EVIDENCE_DIR', "
                "'POSTGRES_ACCEPTANCE_FIXTURE_VERSION', 'POSTGRES_ACCEPTANCE_REQUIRED')}))"
            ),
        ]
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            exit_code, report = self.invoke(
                root,
                command,
                "postgresql://postgres@",
            )
            stdout = Path(report["raw_result"]["stdout_file"]).read_text(encoding="utf-8")
            values = json.loads(stdout)
        self.assertEqual(exit_code, 0)
        self.assertEqual(values["BASE_SHA"], "1" * 40)
        self.assertEqual(values["CANDIDATE_SHA"], "2" * 40)
        self.assertEqual(
            values["POSTGRES_ACCEPTANCE_EVIDENCE_DIR"],
            str((root / "evidence").resolve()),
        )
        self.assertEqual(values["POSTGRES_ACCEPTANCE_FIXTURE_VERSION"], "4.13.11")
        self.assertEqual(values["POSTGRES_ACCEPTANCE_REQUIRED"], "1")

    def test_failed_command_is_failed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            exit_code, report = self.invoke(
                Path(temporary_directory),
                [sys.executable, "-c", "raise SystemExit(7)"],
                "postgresql://postgres@",
            )
        self.assertEqual(exit_code, 7)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["result"], "failed")
        self.assertEqual(report["raw_result"]["exit_code"], 7)

    def test_timeout_writes_report_when_subprocess_output_is_bytes(self) -> None:
        database_url = "postgresql://postgres:p%40ss@"
        timeout = subprocess.TimeoutExpired(
            ["acceptance"],
            1,
            output=b"password=p%40ss",
            stderr=b"postgresql://postgres:p%40ss@db.internal:5432/app",
        )
        process = Mock()
        process.pid = 417
        process.returncode = -15
        process.communicate.side_effect = [timeout, (b"", b"")]
        with tempfile.TemporaryDirectory() as temporary_directory:
            with (
                patch.object(self.runner.subprocess, "Popen", return_value=process) as popen,
                patch.object(self.runner.os, "killpg") as killpg,
            ):
                exit_code, report = self.invoke(
                    Path(temporary_directory),
                    [sys.executable, "-c", "raise SystemExit(99)"],
                    database_url,
                )
            stdout = Path(report["raw_result"]["stdout_file"]).read_text(encoding="utf-8")
            stderr = Path(report["raw_result"]["stderr_file"]).read_text(encoding="utf-8")
        self.assertEqual(exit_code, 124)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["reason"]["code"], "acceptance_command_timeout")
        self.assertIn("<redacted-postgres-credential>", stdout)
        self.assertIn("<redacted-postgres-url>", stderr)
        self.assertNotIn("p%40ss", stdout + stderr)
        popen.assert_called_once()
        self.assertTrue(popen.call_args.kwargs["start_new_session"])
        killpg.assert_called_once_with(417, self.runner.signal.SIGTERM)

    def test_redacts_postgres_url_variants_and_standalone_password(self) -> None:
        database_url = "postgresql://postgres:p%40ss@db.internal:5432/codex"
        variant_url = "postgres://postgres:p%40ss@localhost:5432/codex?sslmode=disable"
        command = [
            sys.executable,
            "-c",
            f"print({variant_url!r}); print('postgresql+psycopg://postgres:p%40ss@db.internal:5432/codex'); print('password=p@ss')",
        ]
        with tempfile.TemporaryDirectory() as temporary_directory:
            with patch.object(self.runner.socket, "create_connection"):
                exit_code, report = self.invoke(
                    Path(temporary_directory), command, database_url
                )
            stdout = Path(report["raw_result"]["stdout_file"]).read_text(encoding="utf-8")
            serialized = json.dumps(report)
        self.assertEqual(exit_code, 0)
        self.assertIn("<redacted-postgres-url>", stdout)
        self.assertNotIn("p%40ss", stdout + serialized)
        self.assertNotIn("p@ss", stdout + serialized)

    def test_redacts_query_passwords_and_sentence_punctuation(self) -> None:
        database_url = "postgresql://postgres@db.internal:5432/codex?password=p%40ss"
        command = [
            sys.executable,
            "-c",
            "print('password=p%40ss.'); print('password=p@ss.')",
        ]
        with tempfile.TemporaryDirectory() as temporary_directory:
            with patch.object(self.runner.socket, "create_connection"):
                exit_code, report = self.invoke(
                    Path(temporary_directory), command, database_url
                )
            stdout = Path(report["raw_result"]["stdout_file"]).read_text(encoding="utf-8")
            serialized = json.dumps(report)
        self.assertEqual(exit_code, 0)
        self.assertNotIn("p%40ss", stdout + serialized)
        self.assertNotIn("p@ss", stdout + serialized)
        self.assertIn("<redacted-postgres-credential>", stdout)

    def test_does_not_redact_known_password_inside_evidence_path(self) -> None:
        path = "scripts/postgres-acceptance/run-live-suite.sh"
        self.assertEqual(
            self.runner.redact_text(path, "postgresql://postgres:postgres@"),
            path,
        )

    def test_ci_workflow_requires_live_service_and_uploads_evidence(self) -> None:
        workflow = (
            REPO_ROOT / ".github" / "workflows" / "ci-postgresql-acceptance.yml"
        ).read_text(encoding="utf-8")
        for required_fragment in (
            "name: PostgreSQL Migration Functional Checks",
            "image: postgres:18.6@sha256:979bccfde35445db618c65107d27d08c6c21339d8dc3ad4c830c6633947be0e4",
            "POSTGRES_TEST_DATABASE_URL:",
            "workflow_dispatch:\n    inputs:\n      base_sha:",
            "candidate_sha:",
            "merge_group:\n    types:\n      - checks_requested",
            "github.event_name == 'merge_group' && github.event.merge_group.base_sha",
            "github.event_name == 'merge_group' && github.event.merge_group.head_sha",
            '"status": "blocked", "result": "not-run"',
            "bash scripts/postgres-acceptance/run-functional-suite.sh",
            "actions/upload-artifact@v7",
            "if-no-files-found: error",
        ):
            self.assertIn(required_fragment, workflow)

        functional_suite = (
            REPO_ROOT / "scripts" / "postgres-acceptance" / "run-functional-suite.sh"
        ).read_text(encoding="utf-8")
        for required_fragment in (
            "optional_postgres_migration_engine_recovers_after_process_sigkill",
            "optional_postgres_migration_engine_uses_real_source_and_target_fixture",
            "optional_postgres_migration_target_uses_real_pool_transactions",
            "migration_source_ack_uncertainty_is_idempotent_after_commit",
        ):
            self.assertIn(required_fragment, functional_suite)
        for forbidden in ("pressure", "continuous_load", "performance"):
            self.assertNotIn(forbidden, functional_suite)
        engine = "tests::lightweight::postgres_migration_engine::"
        postgres = "tests::lightweight::postgres_tests::"
        archive = "tests::archive_file_io::archive_postgres_adapter::"
        expected = {engine + name for name in (
            "optional_postgres_migration_engine_uses_real_source_and_target_fixture",
            "optional_postgres_migration_target_uses_real_pool_transactions",
            "optional_postgres_migration_engine_recovers_after_process_sigkill",
            "migration_source_ack_uncertainty_is_idempotent_after_commit",
            "migration_engine_restarts_after_apply_ack_failure_without_loss_or_resurrection",
            "migration_engine_copies_catches_up_verifies_and_publishes_authority",
            "switch_catches_up_source_writes_during_large_pre_fence_verification",
            "migration_switch_serializes_concurrent_authority_attempts",
            "migration_verification_stops_when_shutdown_is_cancelled",
            "migration_switch_keeps_source_fenced_after_indeterminate_publication",
            "migration_switch_retries_after_recovery_with_next_authority_generation",
            "migration_switch_abort_does_not_leave_source_fenced",
            "migration_switch_retries_after_tail_failure_without_generation_drift",
            "migration_switch_fails_closed_for_pre_fenced_source_authority",
            "migration_switch_fails_closed_when_source_fence_rollback_fails",
        )}
        expected.update(postgres + name for name in (
            "native_sqlite_capture_triggers_capture_and_fence_legacy_writes",
            "archive_native_insert_path_uses_owner_target_columns_when_postgres_is_available",
            "optional_postgres_storage_transaction_smoke_test",
        ))
        expected.update(archive + name for name in (
            "postgres_archive_ownership_is_cross_batch_and_release_retry_monotonic",
            "postgres_archive_release_and_prune_wait_for_live_raw_link_transactions",
            "postgres_archive_prune_ledger_upgrade_backfills_only_recoverable_paths",
        ))
        expected.update({
            "tests::lightweight::storage_migration_status::postgres_cutover_publishes_postgres_authority_and_completed_module",
            "tests::archive_file_io::terminal_journal_recovery::valid_attempt_ack_does_not_create_a_false_interruption_after_isolation",
            "tests::archive_file_io::shutdown_terminal_recovery::shutdown_recovery_ack_failure_keeps_obligation_without_poisoning_main_journal",
        })
        self.assertEqual(set(re.findall(r"test\(=([^)]*)\)", functional_suite)), expected)
        self.assertNotIn("|| true", functional_suite)


if __name__ == "__main__":
    unittest.main()
