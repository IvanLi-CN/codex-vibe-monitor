#!/usr/bin/env python3
"""Unit tests for the PostgreSQL acceptance runner contract."""

from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


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

    def test_ci_workflow_requires_live_service_and_uploads_evidence(self) -> None:
        workflow = (
            REPO_ROOT / ".github" / "workflows" / "ci-postgresql-acceptance.yml"
        ).read_text(encoding="utf-8")
        for required_fragment in (
            "name: PostgreSQL Migration Acceptance",
            "image: postgres:16.6",
            "POSTGRES_TEST_DATABASE_URL:",
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
