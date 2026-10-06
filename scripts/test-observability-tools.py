#!/usr/bin/env python3
"""Regression coverage for offline retirement and restricted diagnostics."""
import importlib.machinery
import importlib.util
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, call, patch

SCRIPTS = Path(__file__).resolve().parent
SOURCE = SCRIPTS.parent

def load(name, path):
    loader = importlib.machinery.SourceFileLoader(name, str(path))
    spec = importlib.util.spec_from_loader(name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module

migration = load("retire_performance", SCRIPTS / "retire-performance-db.py")
observe = load("cvm_observe", SCRIPTS / "cvm-observe")
cpu = load("cvm_cpu", SCRIPTS / "cvm-hotpath-cpu")
preview = load("grafana_preview_metrics", SOURCE / "ops/observability/preview/metrics_server.py")
OLD_IMAGE = "example/cvm@sha256:" + "a" * 64

def fixture(path, ddl_transform=None):
    connection = sqlite3.connect(path)
    connection.execute("PRAGMA journal_mode=WAL")
    connection.execute("PRAGMA wal_autocheckpoint=0")
    connection.execute("PRAGMA user_version=1")
    # The actual retired v1 DDL is independent of the migration validator's tuples.
    ddl = '''
        CREATE TABLE performance_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE performance_epochs (epoch TEXT PRIMARY KEY, started_at TEXT NOT NULL, ended_at TEXT);
        CREATE TABLE performance_buckets (
            bucket_start INTEGER NOT NULL, resolution_seconds INTEGER NOT NULL,
            metric_id TEXT NOT NULL, dimension_code TEXT NOT NULL,
            sample_count INTEGER NOT NULL, expected_count INTEGER NOT NULL, sum_value REAL NOT NULL,
            min_value REAL, max_value REAL, last_value REAL,
            weighted_sum REAL NOT NULL DEFAULT 0, weighted_seconds REAL NOT NULL DEFAULT 0,
            histogram_json TEXT NOT NULL, epoch TEXT NOT NULL,
            PRIMARY KEY(bucket_start, resolution_seconds, metric_id, dimension_code));
        CREATE INDEX idx_performance_buckets_range ON performance_buckets(resolution_seconds,bucket_start);
        CREATE TABLE performance_collector_health (
            id INTEGER PRIMARY KEY CHECK(id = 1), state TEXT NOT NULL, last_successful_flush TEXT,
            dropped_samples INTEGER NOT NULL, flush_failure_count INTEGER NOT NULL, last_error TEXT);
    '''
    connection.executescript(ddl_transform(ddl) if ddl_transform is not None else ddl)
    connection.execute("INSERT INTO performance_meta VALUES ('schema_version','1')")
    connection.execute("INSERT INTO performance_epochs VALUES ('wal-only','now',NULL)")
    connection.commit()
    return connection


class GrafanaPreviewTests(unittest.TestCase):
    def test_fixture_emits_unique_series_for_both_scrape_ports(self):
        for port in (9091, 6772):
            with self.subTest(port=port):
                payload = preview.Metrics().render(port)
                series = [
                    line.split(" ", 1)[0]
                    for line in payload.splitlines()
                    if line and not line.startswith("#")
                ]
                self.assertEqual(len(series), len(set(series)))
        self.assertIn('source="synthetic-preview"', preview.Metrics().render(9091))

    def test_preview_command_passes_shell_syntax_check(self):
        subprocess.run(["bash", "-n", str(SCRIPTS / "cvm-grafana-preview")], check=True)

class RetirementTests(unittest.TestCase):
    def test_program_range_and_old_writer_image_are_verified_before_cutover(self):
        metadata = {"Id": "sha256:old-image", "Config": {"Labels": {"org.opencontainers.image.version": "3.0.0"}}}
        with patch.object(subprocess, "check_output", side_effect=[json.dumps([metadata]), "sha256:old-image\n"]):
            self.assertEqual(migration.compatible_image(OLD_IMAGE, "old-app"), "3.0.0")
        for version in ["1.99.0", "2.86.2", "4.0.0", "unknown", ""]:
            metadata["Config"]["Labels"]["org.opencontainers.image.version"] = version
            with patch.object(subprocess, "check_output", return_value=json.dumps([metadata])):
                with self.assertRaisesRegex(ValueError, "major skips"):
                    migration.compatible_image(OLD_IMAGE, "old-app")
        metadata["Config"]["Labels"]["org.opencontainers.image.version"] = "3.0.0"
        with patch.object(subprocess, "check_output", side_effect=[json.dumps([metadata]), "sha256:unrelated\n"]):
            with self.assertRaisesRegex(ValueError, "does not match"):
                migration.compatible_image(OLD_IMAGE, "wrong-app")

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.data = self.root / "data"; self.data.mkdir()
        self.business = self.data / "business.sqlite"; self.business.write_bytes(b"business untouched")
        self.source = self.data / "custom.metrics.sqlite"
        self.config = self.root / "old.env"; self.config.write_text("compatible configuration")
        self.archive = self.root / "archive"
        self.business_hash = migration.digest(self.business)

    def tearDown(self):
        self.assertEqual(migration.digest(self.business), self.business_hash)
        self.temp.cleanup()

    def run_archive(self, source=None):
        return migration.archive(source or self.source, self.business, self.data, self.archive, "cutover", OLD_IMAGE, self.config, "3.0.0")

    def test_archive_refuses_nonpreceding_major_before_touching_source(self):
        original = fixture(self.source)
        original.close()
        before = migration.digest(self.source)
        for version in ["2.86.2", "4.0.0"]:
            with self.assertRaisesRegex(ValueError, "verified v3"):
                migration.archive(self.source, self.business, self.data, self.archive,
                                  "cutover", OLD_IMAGE, self.config, version)
            self.assertEqual(migration.digest(self.source), before)
            self.assertFalse(self.archive.exists())

    def test_wal_backup_idempotency_and_restore(self):
        original = fixture(self.source)
        self.assertTrue(Path(str(self.source) + "-wal").exists())
        manifest = self.run_archive()
        original.close()
        self.assertEqual(manifest["state"], "archived")
        self.assertEqual(manifest["schema"]["userVersion"], 1)
        self.assertEqual(manifest["previousProgramVersion"], "3.0.0")
        self.assertEqual(manifest["integrityCheck"], {"status": "passed", "result": "ok"})
        self.assertLessEqual(manifest["verifiedAt"], manifest["archivedAt"])
        self.assertEqual(manifest["cutoverScope"], "performance_file_family")
        self.assertEqual((migration.dt.datetime.fromisoformat(manifest["retainUntil"]) - migration.dt.datetime.fromisoformat(manifest["archivedAt"])).days, 90)
        self.assertFalse(self.source.exists())
        backup = self.archive / "cutover" / "performance.sqlite"
        with closing(sqlite3.connect(backup)) as connection:
            self.assertEqual(connection.execute("SELECT epoch FROM performance_epochs").fetchall(), [("wal-only",)])
        self.assertEqual(self.run_archive(), manifest)
        path = self.archive / "cutover" / "manifest.json"
        self.assertEqual(migration.restore(path, OLD_IMAGE, self.config)["state"], "restored")
        self.assertEqual(migration.restore(path, OLD_IMAGE, self.config)["state"], "restored")

    def test_custom_symlink_and_interrupted_verified_stage(self):
        actual = self.root / "custom.sqlite"
        original = fixture(actual)
        self.source.symlink_to(actual)
        original_unlink = Path.unlink
        failed = False
        def interrupt(path, *args, **kwargs):
            nonlocal failed
            if path == actual and not failed:
                failed = True
                raise OSError("simulated interruption after verification")
            return original_unlink(path, *args, **kwargs)
        with patch.object(Path, "unlink", interrupt):
            with self.assertRaisesRegex(OSError, "interruption"):
                self.run_archive()
        self.assertEqual(json.loads((self.archive / "cutover" / "manifest.json").read_text())["state"], "verified")
        self.assertEqual(self.run_archive()["state"], "archived")
        original.close()
        self.assertFalse(self.source.is_symlink())
        migration.restore(self.archive / "cutover" / "manifest.json", OLD_IMAGE, self.config)
        self.assertEqual(self.source.resolve(), actual)

    def test_new_sidecar_after_verification_preserves_source_family(self):
        original_valid_backup = migration.valid_backup
        for index, (suffix, symlink) in enumerate([(suffix, symlink) for suffix in ["-wal", "-shm"] for symlink in [False, True]]):
            with self.subTest(suffix=suffix, symlink=symlink):
                source = self.data / f"late-member-{index}.sqlite"
                fixture(source).close()
                with closing(sqlite3.connect(source)) as connection:
                    connection.execute("PRAGMA journal_mode=DELETE")
                source_hash = migration.digest(source)
                sidecar = Path(str(source) + suffix)
                operation = f"late-member-{index}"
                def introduce_member(path, expected):
                    original_valid_backup(path, expected)
                    if path.name == "performance.sqlite":
                        if symlink:
                            sidecar.symlink_to(self.root / "missing-sidecar-target")
                        else:
                            sidecar.write_bytes(b"new unverified member")
                with patch.object(migration, "valid_backup", introduce_member):
                    with self.assertRaisesRegex(ValueError, "source family changed"):
                        migration.archive(source, self.business, self.data, self.archive, operation, OLD_IMAGE, self.config, "3.0.0")
                self.assertEqual(migration.digest(source), source_hash)
                if symlink:
                    self.assertTrue(sidecar.is_symlink())
                else:
                    self.assertEqual(sidecar.read_bytes(), b"new unverified member")
                manifest = json.loads((self.archive / operation / "manifest.json").read_text())
                self.assertEqual(manifest["state"], "verified")
                self.assertEqual([row["suffix"] for row in manifest["family"]], [""])

    def test_partially_removed_family_resumes_after_unlink_interruption(self):
        original = fixture(self.source)
        original_unlink = Path.unlink
        wal = Path(str(self.source) + "-wal")
        def interrupt(path, *args, **kwargs):
            if path == wal:
                raise OSError("simulated interruption after main removal")
            return original_unlink(path, *args, **kwargs)
        try:
            with patch.object(Path, "unlink", interrupt):
                with self.assertRaisesRegex(OSError, "interruption after main removal"):
                    self.run_archive()
            self.assertFalse(self.source.exists())
            self.assertTrue(wal.exists())
            manifest = json.loads((self.archive / "cutover" / "manifest.json").read_text())
            self.assertEqual(manifest["state"], "verified")
            self.assertEqual(self.run_archive()["state"], "archived")
            self.assertFalse(wal.exists())
        finally:
            original.close()

    def test_unknown_corrupt_business_and_absent_sources(self):
        self.assertEqual(self.run_archive()["state"], "absent")
        self.source.write_bytes(b"not sqlite")
        with self.assertRaises(ValueError): self.run_archive()
        self.assertEqual(self.source.read_bytes(), b"not sqlite")
        with self.assertRaisesRegex(ValueError, "business"): self.run_archive(self.business)
        other = self.root / "unknown.sqlite"
        with closing(sqlite3.connect(other)) as connection: connection.execute("CREATE TABLE application_state (value TEXT)")
        with self.assertRaises(ValueError):
            migration.archive(other, self.business, self.data, self.archive, "unknown", OLD_IMAGE, self.config, "3.0.0")
        self.assertTrue(other.exists())

    def test_corruption_records_immutable_failure_without_moving_family(self):
        self.source.write_bytes(b"truncated SQLite database")
        wal = Path(str(self.source) + "-wal"); wal.write_bytes(b"preserved WAL bytes")
        before = {path: path.read_bytes() for path in [self.source, wal]}
        with self.assertRaises(sqlite3.DatabaseError): self.run_archive()
        path = self.archive / "cutover" / "manifest.json"
        manifest = json.loads(path.read_text())
        self.assertEqual(manifest["state"], "failed")
        self.assertEqual(manifest["failureClass"], "sqlite_integrity")
        self.assertEqual(manifest["sourceOwnership"], "unverified")
        self.assertTrue(manifest["failedAt"])
        self.assertTrue({"", "-wal"}.issubset({row["suffix"] for row in manifest["family"]}))
        with self.assertRaisesRegex(ValueError, "recorded retirement failure"): self.run_archive()
        self.assertEqual(json.loads(path.read_text()), manifest)
        self.assertEqual({path: path.read_bytes() for path in before}, before)

    def test_archive_reentry_preserves_recorded_previous_configuration(self):
        original = fixture(self.source)
        manifest = self.run_archive(); original.close()
        self.config.write_text("a different previous configuration")
        with self.assertRaisesRegex(ValueError, "different previous program/configuration"):
            self.run_archive()
        self.assertEqual(json.loads((self.archive / "cutover" / "manifest.json").read_text()), manifest)
        self.assertFalse(self.source.exists())

    def test_source_change_backup_failure_and_restore_conflict_preserve_source(self):
        original = fixture(self.source)
        with patch.object(migration, "valid_backup", side_effect=ValueError("backup validation failed")):
            with self.assertRaisesRegex(ValueError, "validation failed"): self.run_archive()
        self.assertTrue(self.source.exists())
        failed = json.loads((self.archive / "cutover" / "manifest.json").read_text())
        self.assertEqual(failed["state"], "failed")
        self.assertEqual(failed["failureClass"], "backup_integrity")
        with self.assertRaisesRegex(ValueError, "recorded retirement failure"):
            self.run_archive()
        self.assertEqual(json.loads((self.archive / "cutover" / "manifest.json").read_text()), failed)
        migration.archive(self.source, self.business, self.data, self.archive, "retry", OLD_IMAGE, self.config, "3.0.0")
        original.close()
        self.source.write_bytes(b"occupied")
        with self.assertRaises(ValueError):
            migration.restore(self.archive / "retry" / "manifest.json", OLD_IMAGE, self.config)
        self.assertEqual(self.source.read_bytes(), b"occupied")

    def test_running_old_writer_is_rejected(self):
        with patch.object(migration.subprocess, "check_output", return_value=b'{"Running":true,"Pid":123}'):
            with self.assertRaisesRegex(ValueError, "stopped"): migration.stopped("cvm")

    def test_archive_is_isolated_from_all_actual_application_mounts(self):
        custom_mount = self.root / "custom-mount"; custom_mount.mkdir()
        alias = self.root / "custom-alias"; alias.symlink_to(custom_mount, target_is_directory=True)
        mounts = [{"Type": "bind", "Source": str(self.data)}, {"Type": "volume", "Source": str(custom_mount)}]
        with patch.object(subprocess, "check_output", return_value=json.dumps(mounts)):
            migration.outside_application_mounts(self.archive, "old-app")
            for destination in [self.data / "archive", alias / "archive"]:
                with self.assertRaisesRegex(ValueError, "every application mount"):
                    migration.outside_application_mounts(destination, "old-app")

    def test_same_names_with_unknown_types_index_or_constraints_are_preserved(self):
        mutations = [
            "DROP INDEX idx_performance_buckets_range; CREATE INDEX idx_performance_buckets_range ON performance_buckets(bucket_start,resolution_seconds);",
            "DROP TABLE performance_epochs; CREATE TABLE performance_epochs(epoch TEXT PRIMARY KEY, started_at REAL NOT NULL, ended_at TEXT);",
            "DROP TABLE performance_collector_health; CREATE TABLE performance_collector_health(id INTEGER PRIMARY KEY, state TEXT NOT NULL, last_successful_flush TEXT, dropped_samples INTEGER NOT NULL, flush_failure_count INTEGER NOT NULL, last_error TEXT);",
        ]
        for index, mutation in enumerate(mutations):
            source = self.data / f"unknown-{index}.sqlite"
            original = fixture(source)
            original.executescript(mutation); original.close()
            before = migration.digest(source)
            with self.assertRaisesRegex(ValueError, "unexpected performance"):
                migration.archive(source, self.business, self.data, self.archive, f"unknown-{index}", OLD_IMAGE, self.config, "3.0.0")
            self.assertEqual(migration.digest(source), before)
            rejected = json.loads((self.archive / f"unknown-{index}" / "manifest.json").read_text())
            self.assertEqual(rejected["state"], "failed")
            self.assertEqual(rejected["failureClass"], "unrecognized_schema")
            self.assertEqual(rejected["sourceOwnership"], "unverified")

    def test_extra_table_check_and_changed_default_preserve_unknown_source(self):
        changes = [
            lambda ddl: ddl.replace("PRIMARY KEY(bucket_start", "CHECK(sample_count >= 0), PRIMARY KEY(bucket_start"),
            lambda ddl: ddl.replace("weighted_sum REAL NOT NULL DEFAULT 0", "weighted_sum REAL NOT NULL DEFAULT 9"),
        ]
        for index, change in enumerate(changes):
            with self.subTest(change=index):
                source = self.data / f"unknown-ddl-{index}.sqlite"
                fixture(source, change).close()
                before = migration.digest(source)
                with closing(sqlite3.connect(source)) as connection:
                    self.assertEqual(connection.execute("PRAGMA integrity_check").fetchone(), ("ok",))
                with self.assertRaisesRegex(ValueError, "schema DDL"):
                    migration.archive(source, self.business, self.data, self.archive, f"unknown-ddl-{index}", OLD_IMAGE, self.config, "3.0.0")
                self.assertEqual(migration.digest(source), before)
                manifest = json.loads((self.archive / f"unknown-ddl-{index}" / "manifest.json").read_text())
                self.assertEqual(manifest["state"], "failed")
                self.assertEqual(manifest["sourceOwnership"], "unverified")
                self.assertEqual(manifest["failureClass"], "unrecognized_schema")

    def test_known_schema_accepts_formatting_and_comments(self):
        original = fixture(self.source, lambda ddl: ddl.replace("CREATE TABLE", "create /* formatting only */ table"))
        try:
            self.assertEqual(self.run_archive()["state"], "archived")
        finally:
            original.close()

    def test_parent_symlink_restores_without_creating_a_leaf_alias(self):
        alias = self.root / "data-alias"
        alias.symlink_to(self.data, target_is_directory=True)
        original = fixture(self.source)
        manifest = self.run_archive(alias / self.source.name)
        original.close()
        self.assertFalse(manifest["sourceIsSymlink"])
        migration.restore(self.archive / "cutover" / "manifest.json", OLD_IMAGE, self.config)
        self.assertFalse(self.source.is_symlink())
        self.assertTrue((alias / self.source.name).exists())

    def test_changed_parent_alias_refuses_restore_before_creating_any_file(self):
        alias = self.root / "data-alias"; alias.symlink_to(self.data, target_is_directory=True)
        original = fixture(self.source)
        self.run_archive(alias / self.source.name); original.close()
        other = self.root / "other"; other.mkdir()
        alias.unlink(); alias.symlink_to(other, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "parent identity"):
            migration.restore(self.archive / "cutover" / "manifest.json", OLD_IMAGE, self.config)
        self.assertFalse(self.source.exists())
        self.assertEqual(list(other.iterdir()), [])

    def archived_restore(self, target=None):
        target = target or self.source
        fixture(target).close()
        if target != self.source: self.source.symlink_to(target)
        self.run_archive()
        return self.archive / "cutover" / "manifest.json"

    def test_restore_copy_failure_preserves_absent_target_and_retry(self):
        manifest = self.archived_restore()
        def interrupt(original, destination, *args, **kwargs):
            destination.write(original.read(4096))
            raise OSError("injected partial copy")
        with patch.object(migration.shutil, "copyfileobj", interrupt):
            with self.assertRaisesRegex(OSError, "partial copy"):
                migration.restore(manifest, OLD_IMAGE, self.config)
        self.assertFalse(self.source.exists())
        self.assertFalse(list(self.data.glob(".custom.metrics.sqlite.restore-*")))
        self.assertEqual(migration.restore(manifest, OLD_IMAGE, self.config)["state"], "restored")
        migration.valid_backup(self.source, json.loads(manifest.read_text())["backupSha256"])

    def test_restore_before_publication_failure_is_reentrant(self):
        manifest = self.archived_restore()
        with patch.object(migration.os, "link", side_effect=OSError("injected before publication")):
            with self.assertRaisesRegex(OSError, "before publication"):
                migration.restore(manifest, OLD_IMAGE, self.config)
        self.assertFalse(self.source.exists())
        self.assertEqual(migration.restore(manifest, OLD_IMAGE, self.config)["state"], "restored")

    def test_restore_temporary_validation_failure_preserves_absent_target(self):
        manifest = self.archived_restore()
        validate = migration.valid_backup
        def reject(path, expected):
            if path.name.startswith(".custom.metrics.sqlite.restore-"):
                raise ValueError("injected staged validation failure")
            return validate(path, expected)
        with patch.object(migration, "valid_backup", reject):
            with self.assertRaisesRegex(ValueError, "staged validation"):
                migration.restore(manifest, OLD_IMAGE, self.config)
        self.assertFalse(self.source.exists())
        self.assertEqual(migration.restore(manifest, OLD_IMAGE, self.config)["state"], "restored")

    def test_restore_racing_destination_is_never_overwritten(self):
        manifest = self.archived_restore()
        link = migration.os.link
        def occupy(original, destination, **kwargs):
            Path(destination).write_bytes(b"unrelated racing destination")
            return link(original, destination, **kwargs)
        with patch.object(migration.os, "link", occupy):
            with self.assertRaises(FileExistsError):
                migration.restore(manifest, OLD_IMAGE, self.config)
        self.assertEqual(self.source.read_bytes(), b"unrelated racing destination")

    def test_restore_existing_expected_dangling_alias_is_reused(self):
        target = self.root / "custom.sqlite"
        manifest = self.archived_restore(target)
        self.source.symlink_to(target)
        alias_identity = self.source.lstat()
        self.assertEqual(migration.restore(manifest, OLD_IMAGE, self.config)["state"], "restored")
        self.assertEqual(self.source.lstat().st_ino, alias_identity.st_ino)
        self.assertEqual(self.source.resolve(), target)
        self.assertEqual(migration.restore(manifest, OLD_IMAGE, self.config)["state"], "restored")

    def test_restore_alias_interruption_resumes_from_complete_target(self):
        target = self.root / "custom.sqlite"
        manifest = self.archived_restore(target)
        symlink = Path.symlink_to
        def interrupt(path, *args, **kwargs):
            if path == self.source: raise OSError("injected before alias publication")
            return symlink(path, *args, **kwargs)
        with patch.object(Path, "symlink_to", interrupt):
            with self.assertRaisesRegex(OSError, "before alias"):
                migration.restore(manifest, OLD_IMAGE, self.config)
        migration.valid_backup(target, json.loads(manifest.read_text())["backupSha256"])
        self.assertFalse(self.source.is_symlink())
        self.assertEqual(migration.restore(manifest, OLD_IMAGE, self.config)["state"], "restored")
        self.assertEqual(self.source.resolve(), target)

    def test_restore_process_death_leaves_only_unpublished_private_temp(self):
        manifest = self.archived_restore()
        code = '''import importlib.util,os,sys
from pathlib import Path
spec=importlib.util.spec_from_file_location("interrupted_restore",sys.argv[1]);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
def fail(original,destination,*args,**kwargs):
 destination.write(original.read(4096));destination.flush();os.fsync(destination.fileno());os._exit(73)
m.shutil.copyfileobj=fail
m.restore(Path(sys.argv[2]),sys.argv[3],Path(sys.argv[4]))
'''
        result = subprocess.run([sys.executable, "-B", "-c", code, str(SCRIPTS / "retire-performance-db.py"), str(manifest), OLD_IMAGE, str(self.config)], check=False)
        self.assertEqual(result.returncode, 73)
        self.assertFalse(self.source.exists())
        orphan = list(self.data.glob(".custom.metrics.sqlite.restore-*"))
        self.assertEqual(len(orphan), 1)
        self.assertEqual(orphan[0].stat().st_size, 4096)
        before = orphan[0].read_bytes()
        self.assertEqual(migration.restore(manifest, OLD_IMAGE, self.config)["state"], "restored")
        self.assertEqual(orphan[0].read_bytes(), before)
        migration.valid_backup(self.source, json.loads(manifest.read_text())["backupSha256"])

    def test_restore_late_sidecar_preserved_before_publication(self):
        manifest = self.archived_restore()
        wal = Path(str(self.source) + "-wal")
        validate = migration.valid_backup
        def occupy(path, expected):
            validate(path, expected)
            if path.name.startswith(".custom.metrics.sqlite.restore-"):
                wal.write_bytes(b"unknown sidecar")
        with patch.object(migration, "valid_backup", occupy):
            with self.assertRaisesRegex(ValueError, "destination is occupied"):
                migration.restore(manifest, OLD_IMAGE, self.config)
        self.assertFalse(self.source.exists())
        self.assertEqual(wal.read_bytes(), b"unknown sidecar")


class DiagnosticsTests(unittest.TestCase):
    def test_container_deadline_signals_and_removes_only_the_owned_sampler(self):
        process = Mock()
        process.wait.side_effect = [subprocess.TimeoutExpired("docker", 30), 0]
        process.poll.return_value = 0
        with patch.object(cpu.subprocess, "Popen", return_value=process), patch.object(cpu, "command") as command, patch.object(cpu.os, "killpg") as killpg:
            cpu.record_bounded(["docker", "run"], 30, {}, lambda: None, "cvm-cpu-owned")
            self.assertEqual(command.call_args_list, [call(["docker", "kill", "--signal", "INT", "cvm-cpu-owned"]), call(["docker", "rm", "-f", "cvm-cpu-owned"])])
            killpg.assert_not_called()

    def test_runtime_snapshot_rejects_unrelated_paths_before_copying(self):
        for path in ["/srv/app/data/private.db", "/tmp/jit.so", "/usr/lib/x86_64-linux-gnu/libc.so.6 (deleted)"]:
            maps = "1-2 r-xp 00000000 00:01 123 " + path
            with patch.object(Path, "read_text", return_value=maps), patch.object(cpu, "command") as command:
                with self.assertRaisesRegex(ValueError, "unsupported runtime"):
                    cpu.snapshot_libraries("bound-app", 123, Path("/unused"))
                command.assert_not_called()
        with patch.object(Path, "read_text", return_value="1-2 rw-s 00000000 00:01 123 /srv/app/data/business.db-shm"), patch.object(cpu, "command") as command:
            self.assertEqual(cpu.snapshot_libraries("bound-app", 123, Path("/unused")), [])
            command.assert_not_called()

    def test_container_sampler_has_only_bound_mounts_and_no_control_socket(self):
        root = Path("/srv/cvm/profiles")
        binary = Path("/srv/cvm/symbols/build-id/codex-vibe-monitor")
        image = "sha256:" + "a" * 64
        libraries = [(root / "capture-runtime-123/0", "/usr/lib/x86_64-linux-gnu/libc.so.6")]
        args = cpu.profiler_arguments({"profilerImage": image}, root, binary, libraries, 1000, 123, "cvm-cpu-test", root / "capture.pending.json.gz", 1024**2, 30)
        self.assertIn("type=bind,src=/srv/cvm/symbols/build-id/codex-vibe-monitor,dst=/usr/local/bin/codex-vibe-monitor,readonly", args)
        self.assertIn("type=bind,src=/srv/cvm/profiles/capture-runtime-123/0,dst=/usr/lib/x86_64-linux-gnu/libc.so.6,readonly", args)
        self.assertIn("--network=none", args)
        self.assertIn("--pull=never", args)
        self.assertEqual(args[args.index("--user") + 1], "0:1000")
        self.assertNotIn("/var/run/docker.sock", " ".join(args))
        self.assertNotIn("--privileged", args)
        self.assertFalse(any(argument.startswith("--cpus=") for argument in args))
        self.assertEqual(args[-2:], ["--output", str(root / "capture.pending.json.gz")])

    def test_cpu_deadline_signals_only_the_profiler_and_saves_output(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "saved.txt"
            application = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(10)"])
            try:
                code = "import signal,time,sys; from pathlib import Path; signal.signal(signal.SIGINT, lambda *_: (Path(sys.argv[1]).write_text('saved'), sys.exit(0))); time.sleep(10)"
                cpu.record_bounded([sys.executable, "-c", code, str(output)], 0.5, cpu.os.environ.copy(), lambda: None)
                self.assertEqual(output.read_text(), "saved")
                self.assertIsNone(application.poll())
            finally:
                application.terminate(); application.wait(timeout=5)

    def test_cpu_retention_validates_symbols_before_removing_any_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            profile = root / "capture-123-456.json.gz"
            symbols = profile.with_suffix(".syms.json")
            profile.write_bytes(b"profile"); symbols.write_bytes(b"symbols")
            path = root / "capture-123-456.manifest.json"
            path.write_text(json.dumps({"kind":"cvm-cpu-profile-v1","profile":profile.name,"symbols":symbols.name,"sha256":cpu.checksum(profile),"symbolsSha256":cpu.checksum(symbols),"finishedEpoch":0}))
            symbols.write_bytes(b"modified")
            with self.assertRaisesRegex(ValueError, "identity changed"):
                cpu.prune(root, cpu.RETENTION_SECONDS)
            self.assertTrue(profile.exists())
            symbols.write_bytes(b"symbols")
            # Reenter safely after interrupted deletion of the first artifact.
            profile.unlink()
            self.assertEqual(cpu.prune(root, cpu.RETENTION_SECONDS), 0)
            self.assertFalse(symbols.exists())
            self.assertFalse(path.exists())

    def test_cpu_retention_preserves_unknown_files_and_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            unknown = root / "unrelated.json"
            unknown.write_bytes(b"preserve")
            with self.assertRaises(ValueError): cpu.prune(root, 0)
            self.assertEqual(unknown.read_bytes(), b"preserve")
            link = root / "capture-123-456.json.gz"
            link.symlink_to(unknown)
            with self.assertRaises(ValueError): cpu.prune(root, 0)
            self.assertTrue(link.is_symlink())

    def test_https_url_rejects_credential_and_query_injection(self):
        for value in ["http://grafana.invalid", "https://u:p@grafana.invalid", "https://grafana.invalid?x=y", "https://grafana.invalid#secret"]:
            with self.assertRaises(ValueError): observe.base_url(value)
        self.assertEqual(observe.base_url("https://grafana.invalid/grafana/"), "https://grafana.invalid/grafana")

    def test_ssh_command_cannot_supply_pid_shell_or_output_path(self):
        for command in ["capture 61", "capture 0", "capture 30; id", "capture --pid 1", "capture 30 /tmp/out", "sh -c id"]:
            with patch.dict(cpu.os.environ, {"SSH_ORIGINAL_COMMAND": command}), patch.object(cpu, "capture") as capture:
                with self.assertRaises(ValueError): cpu.main()
                capture.assert_not_called()

if __name__ == "__main__": unittest.main()
