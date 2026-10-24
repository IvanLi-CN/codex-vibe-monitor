from __future__ import annotations

from subprocess import CompletedProcess
import time
import unittest
from unittest.mock import patch

from contract import CLEANUP_RESERVE_SECONDS
from measure import cleanup_budget, cleanup_named_container, remaining_budget, run_command


class MeasurementBudgetTests(unittest.TestCase):
    def test_work_budget_reserves_time_for_cleanup(self) -> None:
        with patch("measure.time.monotonic", return_value=1_000.0):
            self.assertEqual(remaining_budget(200.0), 900.0 - 800.0 - CLEANUP_RESERVE_SECONDS)
            self.assertEqual(cleanup_budget(200.0), 100.0)
        with patch("measure.time.monotonic", return_value=200.0 + 900.0 - CLEANUP_RESERVE_SECONDS):
            with self.assertRaisesRegex(TimeoutError, "work cutoff"):
                remaining_budget(200.0)

    def test_run_command_caps_timeout_at_the_work_deadline(self) -> None:
        with patch("measure.time.monotonic", return_value=1_000.0), patch("measure.subprocess.run") as run:
            run.return_value = CompletedProcess(["true"], 0, "", "")
            run_command(["true"], timeout=300, phase_started=200.0)
        self.assertEqual(run.call_args.kwargs["timeout"], 900.0 - 800.0 - CLEANUP_RESERVE_SECONDS)

    def test_missing_container_is_distinguished_from_docker_failure(self) -> None:
        missing = CompletedProcess(["docker"], 1, "", "Error: No such object: retention-test-driver")
        with patch("measure.run_command", return_value=missing):
            result = cleanup_named_container("retention-test-driver", time.monotonic(), 30)
        self.assertEqual(result["status"], "already_absent")

        unavailable = CompletedProcess(["docker"], 1, "", "Cannot connect to the Docker daemon")
        with patch("measure.run_command", return_value=unavailable):
            with self.assertRaisesRegex(RuntimeError, "could not inspect named container"):
                cleanup_named_container("retention-test-driver", time.monotonic(), 30)


if __name__ == "__main__":
    unittest.main()
