#!/usr/bin/env python3
"""Unit tests for the PostgreSQL acceptance runner contract."""

from __future__ import annotations

import importlib.util
import json
import os
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
    @classmethod
    def setUpClass(cls) -> None:
        cls.runner = load_runner()

    def invoke(self, root: Path, command: list[str], database_url: str | None) -> tuple[int, dict]:
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
            "--",
            *command,
        ]
        with patch.dict(os.environ, environment, clear=True), patch.object(sys, "argv", argv):
            exit_code = self.runner.main()
        return exit_code, json.loads(output.read_text(encoding="utf-8"))

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
        self.assertTrue(report["raw_result"]["executed"])
        self.assertIn("acceptance ok", stdout)
        self.assertNotIn("secret", json.dumps(report))

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
            "name: PostgreSQL Migration Acceptance",
            "image: postgres:16.6",
            "POSTGRES_TEST_DATABASE_URL:",
            "workflow_dispatch:\n    inputs:\n      base_sha:",
            "candidate_sha:",
            "ref: ${{ github.event_name == 'workflow_dispatch' && inputs.candidate_sha || github.event.pull_request.head.sha }}",
            "github.event_name == 'workflow_dispatch' && inputs.base_sha || github.event.pull_request.base.sha",
            "github.event_name == 'workflow_dispatch' && inputs.candidate_sha || github.event.pull_request.head.sha",
            "--base-sha",
            "--candidate-sha",
            'test "$(git rev-parse HEAD)" = "$CANDIDATE_SHA"',
            "python3 scripts/postgres-acceptance/run.py",
            "--fixture-version 4.13.11",
            "-- bash scripts/postgres-acceptance/run-live-suite.sh",
            "actions/upload-artifact@v7",
            "if: always()",
            "if-no-files-found: error",
        ):
            self.assertIn(required_fragment, workflow)


if __name__ == "__main__":
    unittest.main()
