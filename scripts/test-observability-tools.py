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
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parent

def load(name, path):
    loader = importlib.machinery.SourceFileLoader(name, str(path))
    spec = importlib.util.spec_from_loader(name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module

migration = load("retire_performance", SCRIPTS / "retire-performance-db.py")
observe = load("cvm_observe", SCRIPTS / "cvm-observe")
cpu = load("cvm_cpu", SCRIPTS / "cvm-hotpath-cpu")
OLD_IMAGE = "example/cvm@sha256:" + "a" * 64

def fixture(path):
    connection = sqlite3.connect(path)
    connection.execute("PRAGMA journal_mode=WAL")
    connection.execute("PRAGMA wal_autocheckpoint=0")
    connection.execute("PRAGMA user_version=1")
    # The actual retired v1 DDL is independent of the migration validator's tuples.
    connection.executescript('''
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
    ''')
    connection.execute("INSERT INTO performance_meta VALUES ('schema_version','1')")
    connection.execute("INSERT INTO performance_epochs VALUES ('wal-only','now',NULL)")
    connection.commit()
    return connection

class RetirementTests(unittest.TestCase):
    def test_program_range_and_old_writer_image_are_verified_before_cutover(self):
        metadata = {"Id": "sha256:old-image", "Config": {"Labels": {"org.opencontainers.image.version": "2.60.0"}}}
        with patch.object(subprocess, "check_output", side_effect=[json.dumps([metadata]), "sha256:old-image\n"]):
            self.assertEqual(migration.compatible_image(OLD_IMAGE, "old-app"), "2.60.0")
        for version in ["1.99.0", "3.0.0", "unknown", ""]:
            metadata["Config"]["Labels"]["org.opencontainers.image.version"] = version
            with patch.object(subprocess, "check_output", return_value=json.dumps([metadata])):
                with self.assertRaisesRegex(ValueError, "major skips"):
                    migration.compatible_image(OLD_IMAGE, "old-app")
        metadata["Config"]["Labels"]["org.opencontainers.image.version"] = "2.60.0"
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
        return migration.archive(source or self.source, self.business, self.data, self.archive, "cutover", OLD_IMAGE, self.config)

    def test_wal_backup_idempotency_and_restore(self):
        original = fixture(self.source)
        self.assertTrue(Path(str(self.source) + "-wal").exists())
        manifest = self.run_archive()
        original.close()
        self.assertEqual(manifest["state"], "archived")
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

    def test_unknown_corrupt_business_and_absent_sources(self):
        self.assertEqual(self.run_archive()["state"], "absent")
        self.source.write_bytes(b"not sqlite")
        with self.assertRaises(ValueError): self.run_archive()
        self.assertEqual(self.source.read_bytes(), b"not sqlite")
        with self.assertRaisesRegex(ValueError, "business"): self.run_archive(self.business)
        other = self.root / "unknown.sqlite"
        with closing(sqlite3.connect(other)) as connection: connection.execute("CREATE TABLE application_state (value TEXT)")
        with self.assertRaises(ValueError):
            migration.archive(other, self.business, self.data, self.archive, "unknown", OLD_IMAGE, self.config)
        self.assertTrue(other.exists())

    def test_source_change_backup_failure_and_restore_conflict_preserve_source(self):
        original = fixture(self.source)
        with patch.object(migration, "valid_backup", side_effect=ValueError("backup validation failed")):
            with self.assertRaisesRegex(ValueError, "validation failed"): self.run_archive()
        self.assertTrue(self.source.exists())
        self.run_archive(); original.close()
        self.source.write_bytes(b"occupied")
        with self.assertRaises(ValueError):
            migration.restore(self.archive / "cutover" / "manifest.json", OLD_IMAGE, self.config)
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
                migration.archive(source, self.business, self.data, self.archive, f"unknown-{index}", OLD_IMAGE, self.config)
            self.assertEqual(migration.digest(source), before)
            self.assertFalse((self.archive / f"unknown-{index}" / "manifest.json").exists())

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

class DiagnosticsTests(unittest.TestCase):
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
