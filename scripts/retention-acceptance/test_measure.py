from __future__ import annotations

from pathlib import Path
import shutil
import subprocess
from subprocess import CompletedProcess
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from contract import CLEANUP_RESERVE_SECONDS, TRIAL_EXECUTION_ORDER
from measure import (
    OwnedContainerLedger,
    cleanup_budget,
    cleanup_named_container,
    cleanup_path_if_safe,
    remaining_budget,
    run_command,
    run_foreground,
)


class MeasurementBudgetTests(unittest.TestCase):
    def test_foreground_driver_lifecycle_uses_unique_trial_names_on_one_ledger(self) -> None:
        run_id = "123456abcdef"
        owned = OwnedContainerLedger()
        started = time.monotonic()
        run_names: list[str] = []
        inspect_names: list[str] = []

        def fake_docker(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
            if command[0:2] == ["docker", "run"]:
                name = command[command.index("--name") + 1]
                run_names.append(name)
                return CompletedProcess(command, 0, "{}", "")
            if command[0:2] == ["docker", "inspect"]:
                name = command[-1]
                inspect_names.append(name)
                return CompletedProcess(command, 1, "", f"Error: No such object: {name}")
            self.fail(f"unexpected fake Docker command: {command[1]}")

        expected_names = [
            f"retention-{run_id}-{step['version']}-{step['trial_index']}-driver"
            for step in TRIAL_EXECUTION_ORDER
        ]
        with patch("measure.RUN_ID", run_id), patch("measure.run_command", side_effect=fake_docker):
            for step in TRIAL_EXECUTION_ORDER:
                version = step["version"]
                index = step["trial_index"]
                trial_dir = Path(tempfile.gettempdir()) / f"retention-test-{version}-{index}"
                cleanup: dict[str, object] = {}
                run_foreground(
                    "retention-application",
                    "sha256:" + "1" * 64,
                    started,
                    version,
                    index,
                    cleanup,
                    owned=owned,
                    root=trial_dir.parent,
                    recovery_path=trial_dir,
                )
                self.assertEqual(cleanup["container_name"], expected_names[len(run_names) - 1])
                self.assertEqual(cleanup["status"], "already_absent")

        self.assertEqual(run_names, expected_names)
        self.assertEqual(inspect_names, expected_names)
        records = owned.records()
        self.assertEqual(
            [record["container_name"] for record in records],
            sorted(expected_names),
        )
        self.assertEqual([record["status"] for record in records], ["already_absent"] * 6)

    def test_unconfirmed_foreground_cleanup_refuses_to_remove_trial_data(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "acceptance-root"
            trial_dir = root / "trial-baseline-1"
            trial_dir.mkdir(parents=True)
            sentinel = trial_dir / "database.sqlite"
            sentinel.write_bytes(b"preserve until driver absence is confirmed")
            owned = OwnedContainerLedger()
            cleanup: dict[str, object] = {}
            run_id = "123456abcdef"
            started = time.monotonic()

            def daemon_failure(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                if command[0:2] == ["docker", "run"]:
                    return CompletedProcess(command, 0, "{}", "")
                if command[0:2] == ["docker", "inspect"]:
                    return CompletedProcess(command, 1, "", "Cannot connect to the Docker daemon")
                self.fail(f"unexpected fake Docker command: {command[1]}")

            with patch("measure.RUN_ID", run_id), patch(
                "measure.run_command", side_effect=daemon_failure
            ), patch("measure.bounded_rmtree") as remove:
                with self.assertRaisesRegex(RuntimeError, "could not inspect named container"):
                    run_foreground(
                        "retention-application",
                        "sha256:" + "1" * 64,
                        started,
                        "baseline",
                        1,
                        cleanup,
                        owned=owned,
                        root=root,
                        recovery_path=trial_dir,
                    )
                result, errors = cleanup_path_if_safe(
                    root,
                    root,
                    owned,
                    started,
                    cleanup_reserve_seconds=5,
                )

            self.assertEqual(cleanup["status"], "failed")
            self.assertFalse(result["removed"])
            self.assertTrue(errors)
            self.assertEqual(len(result["recovery"]["unresolved_containers"]), 1)
            self.assertEqual(result["recovery"]["unresolved_containers"][0]["container_name"],
                             f"retention-{run_id}-baseline-1-driver")
            self.assertTrue(sentinel.exists())
            remove.assert_not_called()

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

    def test_daemon_failure_preserves_trial_and_root_across_outer_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "acceptance-root"
            trial = root / "trial-baseline-1"
            template = root / "template-baseline"
            trial.mkdir(parents=True)
            template.mkdir()
            (trial / "database.sqlite").write_bytes(b"recovery data")
            name = "retention-123456abcdef-baseline-1"
            container_id = "a" * 64
            owned = OwnedContainerLedger()
            owned.register(name, "application", root, trial, trial)
            owned.mark_started(name, container_id)
            init_name = "retention-123456abcdef-init-baseline"
            init_id = "d" * 64
            owned.register(init_name, "init", root, template, template)
            owned.mark_started(init_name, init_id)
            entered = threading.Event()
            release = threading.Event()
            result: dict[str, object] = {}

            def daemon_failure(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                entered.set()
                release.wait(timeout=2)
                return CompletedProcess(command, 1, "", "Cannot connect to the Docker daemon")

            def cleanup_trial() -> None:
                result["cleanup"] = cleanup_path_if_safe(
                    trial, root, owned, time.monotonic(), cleanup_reserve_seconds=5
                )

            with patch("measure.run_command", side_effect=daemon_failure), patch("measure.bounded_rmtree") as remove:
                worker = threading.Thread(target=cleanup_trial)
                worker.start()
                self.assertTrue(entered.wait(timeout=2), "fake Docker inspection did not reach its barrier")
                self.assertTrue(root.is_dir())
                self.assertTrue(trial.is_dir())
                release.set()
                worker.join(timeout=2)
                self.assertFalse(worker.is_alive(), "cleanup did not finish after the barrier was released")
                root_result, root_errors = cleanup_path_if_safe(
                    root, root, owned, time.monotonic(), cleanup_reserve_seconds=5
                )

            trial_result, trial_errors = result["cleanup"]
            self.assertFalse(trial_result["removed"])
            self.assertTrue(trial_errors)
            self.assertFalse(root_result["removed"])
            self.assertTrue(root_errors)
            unresolved_ids = {
                record["container_id"]
                for record in root_result["recovery"]["unresolved_containers"]
            }
            self.assertEqual(unresolved_ids, {container_id, init_id})
            self.assertEqual(
                set(root_result["recovery"]["preserved_paths"]),
                {str(root), str(trial), str(template)},
            )
            self.assertTrue(root.is_dir())
            self.assertTrue(trial.is_dir())
            remove.assert_not_called()

    def test_inspect_timeout_preserves_the_recovery_root(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "acceptance-root"
            root.mkdir()
            (root / "database.sqlite").write_bytes(b"recovery data")
            name = "retention-123456abcdef-driver"
            owned = OwnedContainerLedger()
            owned.register(name, "foreground", root, root, root / "foreground.py")
            owned.mark_started(name, "b" * 64)
            timeout = subprocess.TimeoutExpired(["docker", "inspect", name], 3)
            with patch("measure.run_command", side_effect=timeout), patch("measure.bounded_rmtree") as remove:
                cleanup, errors = cleanup_path_if_safe(
                    root, root, owned, time.monotonic(), cleanup_reserve_seconds=5
                )

            self.assertFalse(cleanup["removed"])
            self.assertTrue(errors)
            self.assertEqual(cleanup["recovery"]["unresolved_containers"][0]["container_id"], "b" * 64)
            self.assertTrue(root.is_dir())
            remove.assert_not_called()

    def test_confirmed_container_removal_allows_directory_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "acceptance-root"
            trial = root / "trial-baseline-1"
            trial.mkdir(parents=True)
            (trial / "database.sqlite").write_bytes(b"temporary data")
            name = "retention-123456abcdef-baseline-1"
            container_id = "c" * 64
            owned = OwnedContainerLedger()
            owned.register(name, "application", root, trial, trial)
            owned.mark_started(name, container_id)
            commands: list[list[str]] = []

            def docker_success(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                commands.append(command)
                if command[:3] == ["docker", "inspect", "--format"]:
                    if sum(row[:2] == ["docker", "inspect"] for row in commands) == 1:
                        return CompletedProcess(command, 0, f"{container_id} true", "")
                    return CompletedProcess(command, 1, "", f"Error: No such object: {name}")
                return CompletedProcess(command, 0, container_id, "")

            def remove_directory(path: Path, *_: object) -> None:
                shutil.rmtree(path)

            with patch("measure.run_command", side_effect=docker_success), patch(
                "measure.bounded_rmtree", side_effect=remove_directory
            ):
                cleanup, errors = cleanup_path_if_safe(
                    trial, root, owned, time.monotonic(), cleanup_reserve_seconds=5
                )

            self.assertTrue(cleanup["removed"])
            self.assertFalse(errors)
            self.assertFalse(trial.exists())
            self.assertEqual(owned.record(name)["status"], "removed")
            self.assertEqual([command[1] for command in commands], ["inspect", "stop", "rm", "inspect"])


if __name__ == "__main__":
    unittest.main()
