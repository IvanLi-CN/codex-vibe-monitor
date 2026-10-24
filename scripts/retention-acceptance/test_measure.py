from __future__ import annotations

import json
import shutil
import subprocess
from pathlib import Path
from subprocess import CompletedProcess
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from contract import (
    AcceptanceError,
    CLEANUP_RESERVE_SECONDS,
    CONTAINER_KIND_LABEL,
    CONTAINER_OWNER_LABEL,
    CONTAINER_RUN_ID_LABEL,
    CONTAINER_TRIAL_LABEL,
    TRIAL_EXECUTION_ORDER,
)
from measure import (
    OwnedContainerLedger,
    cleanup_budget,
    cleanup_named_container,
    cleanup_path_if_safe,
    remaining_budget,
    run_command,
    run_foreground,
    start_container,
)


def docker_labels(command: list[str]) -> dict[str, str]:
    labels: dict[str, str] = {}
    for index, argument in enumerate(command[:-1]):
        if argument == "--label":
            key, value = command[index + 1].split("=", 1)
            labels[key] = value
    return labels


class MeasurementBudgetTests(unittest.TestCase):
    def test_foreground_driver_lifecycle_uses_unique_trial_names_on_one_ledger(self) -> None:
        run_id = "123456abcdef"
        owned = OwnedContainerLedger()
        started = time.monotonic()
        run_names: list[str] = []
        inspect_names: list[str] = []
        labels_by_name: dict[str, dict[str, str]] = {}
        ids_by_name: dict[str, str] = {}

        def fake_docker(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
            if command[0:2] == ["docker", "run"]:
                name = command[command.index("--name") + 1]
                run_names.append(name)
                labels = {
                    command[index + 1].split("=", 1)[0]: command[index + 1].split("=", 1)[1]
                    for index, argument in enumerate(command[:-1])
                    if argument == "--label"
                }
                labels_by_name[name] = labels
                ids_by_name[name] = f"{len(run_names):064x}"
                return CompletedProcess(command, 0, "{}", "")
            if command[0:2] == ["docker", "rm"]:
                container_id = command[-1]
                name = next(name for name, item in ids_by_name.items() if item == container_id)
                del ids_by_name[name]
                del labels_by_name[name]
                return CompletedProcess(command, 0, container_id, "")
            if command[0:2] == ["docker", "inspect"]:
                target = command[-1]
                name = target if target in ids_by_name else next(
                    (name for name, item in ids_by_name.items() if item == target), None
                )
                if name is None:
                    return CompletedProcess(command, 1, "", f"Error: No such object: {target}")
                if target == name:
                    inspect_names.append(target)
                return CompletedProcess(
                    command,
                    0,
                    f"{ids_by_name[name]} false {json.dumps(labels_by_name[name])}",
                    "",
                )
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
                self.assertEqual(cleanup["status"], "removed")
                self.assertEqual(cleanup["container_id"], f"{len(run_names):064x}")
                self.assertEqual(cleanup["identity_labels"], owned.record(expected_names[len(run_names) - 1])["identity_labels"])

        self.assertEqual(run_names, expected_names)
        self.assertEqual(inspect_names, expected_names)
        records = owned.records()
        self.assertEqual(
            [record["container_name"] for record in records],
            sorted(expected_names),
        )
        self.assertEqual([record["status"] for record in records], ["removed"] * 6)

    def test_init_and_application_creation_labels_bind_run_and_trial_identity(self) -> None:
        run_id = "123456abcdef"
        with tempfile.TemporaryDirectory() as temporary, patch("measure.RUN_ID", run_id):
            for kind, version, trial_index in (
                ("init", "baseline", None),
                ("application", "candidate", 2),
            ):
                owned = OwnedContainerLedger()
                data_dir = Path(temporary) / f"{kind}-{version}"
                name = (
                    f"retention-{run_id}-init-{version}"
                    if trial_index is None
                    else f"retention-{run_id}-{version}-{trial_index}"
                )
                command_seen: list[list[str]] = []

                def docker_run(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                    command_seen.append(command)
                    return CompletedProcess(command, 0, "a" * 64, "")

                with patch("measure.run_command", side_effect=docker_run):
                    result = start_container(
                        "test-image",
                        data_dir,
                        name,
                        time.monotonic(),
                        owned=owned,
                        kind=kind,
                        root=Path(temporary),
                        version=version,
                        trial_index=trial_index,
                    )

                self.assertEqual(result, "a" * 64)
                self.assertEqual(docker_labels(command_seen[0]), owned.record(name)["identity_labels"])
                identity = owned.record(name)["identity_labels"]
                self.assertEqual(identity[CONTAINER_RUN_ID_LABEL], run_id)
                self.assertEqual(identity[CONTAINER_KIND_LABEL], kind)
                self.assertEqual(
                    identity[CONTAINER_TRIAL_LABEL],
                    "init" if trial_index is None else str(trial_index),
                )

    def test_unconfirmed_name_collisions_preserve_foreign_init_app_and_driver_containers(self) -> None:
        run_id = "123456abcdef"
        foreign_id = "f" * 64
        with tempfile.TemporaryDirectory() as temporary, patch("measure.RUN_ID", run_id):
            for kind in ("init", "application", "foreground"):
                with self.subTest(kind=kind):
                    root = Path(temporary) / kind
                    recovery_path = root / "trial"
                    recovery_path.mkdir(parents=True)
                    sentinel = recovery_path / "database.sqlite"
                    sentinel.write_bytes(b"preserve while container ownership is unknown")
                    owned = OwnedContainerLedger()
                    trial_index = None if kind == "init" else 1
                    version = "baseline"
                    name = (
                        f"retention-{run_id}-init-{version}"
                        if kind == "init"
                        else f"retention-{run_id}-{version}-{trial_index}"
                        + ("-driver" if kind == "foreground" else "")
                    )
                    commands: list[list[str]] = []

                    def docker_conflict(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                        commands.append(command)
                        if command[0:2] == ["docker", "run"]:
                            raise RuntimeError("container name already exists")
                        if command[0:2] == ["docker", "inspect"]:
                            labels = dict(owned.record(name)["identity_labels"])
                            labels[CONTAINER_RUN_ID_LABEL] = "fedcba987654"
                            labels[CONTAINER_OWNER_LABEL] = "e" * 32
                            return CompletedProcess(
                                command,
                                0,
                                f"{foreign_id} true {json.dumps(labels)}",
                                "",
                            )
                        self.fail(f"foreign container must not be stopped or removed: {command}")

                    with patch("measure.run_command", side_effect=docker_conflict), patch(
                        "measure.bounded_rmtree"
                    ) as remove:
                        if kind == "foreground":
                            cleanup: dict[str, object] = {}
                            with self.assertRaisesRegex(AcceptanceError, "unexpected run identity"):
                                run_foreground(
                                    "retention-application",
                                    "sha256:" + "1" * 64,
                                    time.monotonic(),
                                    version,
                                    1,
                                    cleanup,
                                    owned=owned,
                                    root=root,
                                    recovery_path=recovery_path,
                                )
                        else:
                            with self.assertRaisesRegex(RuntimeError, "container name already exists"):
                                start_container(
                                    "test-image",
                                    recovery_path,
                                    name,
                                    time.monotonic(),
                                    owned=owned,
                                    kind=kind,
                                    root=root,
                                    version=version,
                                    trial_index=trial_index,
                                )
                            result, errors = cleanup_path_if_safe(
                                recovery_path,
                                root,
                                owned,
                                time.monotonic(),
                                cleanup_reserve_seconds=5,
                            )
                            self.assertFalse(result["removed"])
                            self.assertTrue(errors)

                    self.assertTrue(sentinel.exists())
                    self.assertEqual([command[1] for command in commands if command[0] == "docker"], ["run", "inspect"])
                    remove.assert_not_called()

    def test_creation_timeouts_recover_owned_init_application_and_driver_containers(self) -> None:
        run_id = "123456abcdef"
        container_id = "a" * 64
        with tempfile.TemporaryDirectory() as temporary, patch("measure.RUN_ID", run_id):
            for kind in ("init", "application", "foreground"):
                with self.subTest(kind=kind):
                    root = Path(temporary) / kind
                    recovery_path = root / "recovery"
                    recovery_path.mkdir(parents=True)
                    sentinel = recovery_path / "database.sqlite"
                    sentinel.write_bytes(b"recover until confirmed cleanup")
                    version = "baseline"
                    trial_index = None if kind == "init" else 1
                    name = (
                        f"retention-{run_id}-init-{version}"
                        if kind == "init"
                        else f"retention-{run_id}-{version}-{trial_index}"
                        + ("-driver" if kind == "foreground" else "")
                    )
                    owned = OwnedContainerLedger()
                    commands: list[list[str]] = []

                    def docker_timeout(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                        commands.append(command)
                        if command[0:2] == ["docker", "run"]:
                            raise subprocess.TimeoutExpired(command, 60)
                        if command[0:2] == ["docker", "inspect"] and command[-1] == name:
                            labels = owned.record(name)["identity_labels"]
                            return CompletedProcess(command, 0, f"{container_id} true {json.dumps(labels)}", "")
                        if command[0:2] in (["docker", "stop"], ["docker", "rm"]):
                            return CompletedProcess(command, 0, container_id, "")
                        if command[0:2] == ["docker", "inspect"]:
                            return CompletedProcess(command, 1, "", f"Error: No such object: {command[-1]}")
                        self.fail(f"unexpected fake Docker command: {command[1]}")

                    with patch("measure.run_command", side_effect=docker_timeout), patch(
                        "measure.bounded_rmtree", side_effect=lambda path, *_: shutil.rmtree(path)
                    ):
                        if kind == "foreground":
                            cleanup_evidence: dict[str, object] = {}
                            with self.assertRaises(subprocess.TimeoutExpired):
                                run_foreground(
                                    "retention-application",
                                    "sha256:" + "1" * 64,
                                    time.monotonic(),
                                    version,
                                    1,
                                    cleanup_evidence,
                                    owned=owned,
                                    root=root,
                                    recovery_path=recovery_path,
                                )
                            self.assertEqual(cleanup_evidence["status"], "removed")
                            self.assertEqual(cleanup_evidence["container_id"], container_id)
                            self.assertTrue(sentinel.exists())
                        else:
                            with self.assertRaises(subprocess.TimeoutExpired):
                                start_container(
                                    "test-image",
                                    recovery_path,
                                    name,
                                    time.monotonic(),
                                    owned=owned,
                                    kind=kind,
                                    root=root,
                                    version=version,
                                    trial_index=trial_index,
                                )
                            result, errors = cleanup_path_if_safe(
                                recovery_path,
                                root,
                                owned,
                                time.monotonic(),
                                cleanup_reserve_seconds=5,
                            )
                            self.assertTrue(result["removed"])
                            self.assertFalse(errors)
                            self.assertFalse(recovery_path.exists())

                    self.assertEqual(owned.record(name)["container_id"], container_id)
                    self.assertEqual(owned.record(name)["status"], "removed")
                    self.assertEqual(
                        owned.record(name)["verified_identity_labels"],
                        owned.record(name)["identity_labels"],
                    )
                    self.assertEqual(
                        [command[1] for command in commands],
                        ["run", "inspect", "stop", "rm", "inspect"],
                    )

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
            owned.register(name, "application", root, trial, trial, version="baseline", trial_index=1)
            owned.mark_started(name, container_id)
            init_name = "retention-123456abcdef-init-baseline"
            init_id = "d" * 64
            owned.register(init_name, "init", root, template, template, version="baseline", trial_index=None)
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
            owned.register(
                name,
                "foreground",
                root,
                root,
                root / "foreground.py",
                version="candidate",
                trial_index=1,
            )
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
            owned.register(name, "application", root, trial, trial, version="baseline", trial_index=1)
            owned.mark_started(name, container_id)
            commands: list[list[str]] = []

            def docker_success(command: list[str], *_: object, **__: object) -> CompletedProcess[str]:
                commands.append(command)
                if command[:3] == ["docker", "inspect", "--format"]:
                    if command[-1] == name:
                        labels = owned.record(name)["identity_labels"]
                        return CompletedProcess(command, 0, f"{container_id} true {json.dumps(labels)}", "")
                    return CompletedProcess(command, 1, "", f"Error: No such object: {command[-1]}")
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
