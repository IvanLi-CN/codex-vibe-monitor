#!/usr/bin/env python3
"""Exercise snapshot identity, isolation, corruption and generation failures without Docker."""
import contextlib
import copy
import importlib.util
import sys
import io
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

import retention_fixture_cache as CACHE


class FixtureCacheTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.cache = self.root / "cache"
        self.output = self.root / "cold"
        self.sources = {"v-test": {"tag": "v-test", "image": "example:v-test", "digest": "example@sha256:123", "generator": "released maintenance verify-archive-storage --dry-run", "exitCode": 0}}
        self.generated = 0

    def generate(self, output):
        self.generated += 1
        version = output / "v-test"
        version.mkdir(parents=True)
        for name in ("business.sqlite", "maintenance.sqlite"):
            with contextlib.closing(sqlite3.connect(version / name)) as connection, connection:
                connection.execute("CREATE TABLE original (value TEXT)")
                connection.execute("INSERT INTO original VALUES ('released')")
        (version / "archives").mkdir()
        (version / "source.json").write_text(json.dumps({**self.sources['v-test'], "imageRef": self.sources['v-test']['digest']}))

    def prepare(self, output=None):
        with contextlib.redirect_stdout(io.StringIO()) as log:
            CACHE.prepare_fixtures(output or self.output, self.cache, self.sources, self.generate)
        return log.getvalue()

    def test_cold_then_warm_avoids_generation_and_copies_identical_state(self):
        self.assertIn("cache: miss", self.prepare())
        self.assertIn("cache: hit", self.prepare(self.root / "warm"))
        self.assertEqual(self.generated, 1)
        self.assertEqual(CACHE.inventory(self.output), CACHE.inventory(self.root / "warm"))

    def test_candidate_mutation_does_not_touch_cached_source_or_next_run(self):
        self.prepare()
        before = CACHE.inventory(self.cache)
        with contextlib.closing(sqlite3.connect(self.output / "v-test/business.sqlite")) as connection, connection:
            connection.execute("DELETE FROM original")
        self.assertEqual(before, CACHE.inventory(self.cache))
        self.prepare(self.root / "warm")
        with contextlib.closing(sqlite3.connect(self.root / "warm/v-test/business.sqlite")) as connection:
            self.assertEqual(connection.execute("SELECT value FROM original").fetchall(), [('released',)])

    def test_each_input_invalidates_fingerprint(self):
        for name in CACHE.INPUTS:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)
        original = CACHE.input_fingerprint(self.root)
        for name in CACHE.INPUTS:
            path = self.root / name
            path.write_text(name + "changed")
            self.assertNotEqual(original, CACHE.input_fingerprint(self.root))
            path.write_text(name)

    def test_stale_snapshot_fails_before_copy_or_generation(self):
        self.prepare()
        with patch.object(CACHE, 'input_fingerprint', return_value='changed'):
            with self.assertRaises(ValueError): self.prepare(self.root / "warm")
        self.assertFalse((self.root / "warm").exists())
        self.assertEqual(self.generated, 1)

    def test_missing_extra_and_changed_files_fail_before_copy(self):
        for mutation in ("missing", "extra", "changed", "missing-directory"):
            with self.subTest(mutation=mutation):
                cache = self.root / mutation
                with patch.object(self, 'cache', cache):
                    self.prepare(self.root / f"cold-{mutation}")
                    database = cache / "v-test/business.sqlite"
                    if mutation == 'missing': database.unlink()
                    elif mutation == 'extra': (cache / "extra").write_text('unexpected')
                    elif mutation == 'missing-directory': (cache / "v-test/archives").rmdir()
                    else: database.write_bytes(b'corrupted')
                    with self.assertRaises((ValueError, FileNotFoundError)): self.prepare(self.root / f"warm-{mutation}")
                    self.assertFalse((self.root / f"warm-{mutation}").exists())

    def test_links_and_metadata_links_rejected(self):
        self.prepare()
        for name in ('v-test/business.sqlite', CACHE.MANIFEST):
            path = self.cache / name
            original = path.read_bytes()
            path.unlink()
            path.symlink_to(self.root / 'external')
            with self.assertRaises(ValueError): self.prepare(self.root / "warm")
            path.unlink()
            path.write_bytes(original)

    def test_cache_root_link_and_overlapping_paths_rejected(self):
        self.cache.symlink_to(self.root / 'external', target_is_directory=True)
        with self.assertRaises(ValueError): self.prepare()
        self.cache.unlink()
        for output in (self.cache, self.cache / 'nested', self.root):
            with self.assertRaises(ValueError): self.prepare(output)

    def test_wrong_release_identity_rejected_even_with_matching_inventory(self):
        self.prepare()
        self.sources['v-test']['digest'] = 'different@sha256:456'
        with self.assertRaises(ValueError): self.prepare(self.root / 'warm')

    def test_size_and_entry_limits_rejected(self):
        self.prepare()
        for name in ('MAX_BYTES', 'MAX_ENTRIES'):
            with patch.object(CACHE, name, 1):
                with self.assertRaises(ValueError): self.prepare(self.root / 'warm')

    def test_partial_generation_never_publishes_snapshot(self):
        def incomplete(output):
            output.mkdir()
            raise RuntimeError('docker failed')
        with patch.object(self, 'generate', side_effect=incomplete):
            with self.assertRaises(RuntimeError): self.prepare()
        self.assertFalse(self.cache.exists())

    def test_existing_working_copy_is_preserved(self):
        self.output.mkdir()
        (self.output / 'keep').write_text('existing')
        with self.assertRaises(ValueError): self.prepare()
        self.assertEqual((self.output / 'keep').read_text(), 'existing')


class WorkflowCacheContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = CACHE.ROOT / '.github/scripts/check_quality_gates_contract.py'
        spec = importlib.util.spec_from_file_location('fixture_cache_contract', path)
        cls.contract = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = cls.contract
        spec.loader.exec_module(cls.contract)
        workflow = cls.contract.load_yaml(CACHE.ROOT / '.github/workflows/ci-pr.yml')
        cls.job = workflow['jobs']['retention-migration-validation']

    def check(self, job):
        step = self.contract.step_config(job, 'Generate released-source fixtures', 'test')
        command = self.contract.require_command(step, ['python3', 'scripts/generate-retention-maintenance-release-fixtures.py'], 'test', 'generation required')
        self.contract.require_job_and_steps_fail_closed(job, 'test')
        self.contract.require_retention_fixture_cache_contract(job, command, 'test')

    def test_actual_workflow_and_original_rollback_format_pass(self):
        self.check(self.job)
        job = copy.deepcopy(self.job)
        job['steps'] = [step for step in job['steps'] if step['name'] not in ('Restore released-source fixtures', 'Save released-source fixtures')]
        step = self.contract.step_config(job, 'Generate released-source fixtures', 'test')
        step['run'] = 'python3 scripts/generate-retention-maintenance-release-fixtures.py --output-dir "$RUNNER_TEMP/retention-migration-fixtures"'
        self.check(job)

    def test_cache_contract_rejects_partial_or_unsafe_formats(self):
        for mutation in ('fallback', 'key', 'path', 'save-order', 'save-if', 'ignore-failure', 'missing-restore', 'same-working-copy'):
            with self.subTest(mutation=mutation):
                job = copy.deepcopy(self.job)
                restore = self.contract.step_config(job, 'Restore released-source fixtures', 'test')
                save = self.contract.step_config(job, 'Save released-source fixtures', 'test')
                if mutation == 'fallback': restore['with']['restore-keys'] = 'old-'
                elif mutation == 'key': restore['with']['key'] = 'constant'
                elif mutation == 'path': save['with']['path'] = '${{ runner.temp }}/retention-migration-results'
                elif mutation == 'save-order': job['steps'].remove(save); job['steps'].append(save)
                elif mutation == 'save-if': save['if'] = '${{ always() }}'
                elif mutation == 'ignore-failure': restore['continue-on-error'] = True
                elif mutation == 'missing-restore': job['steps'].remove(restore)
                else:
                    step = self.contract.step_config(job, 'Generate released-source fixtures', 'test')
                    step['run'] = step['run'].replace('--cache-dir "$RUNNER_TEMP/retention-release-fixture-cache"', '--cache-dir "$RUNNER_TEMP/retention-migration-fixtures"')
                with self.assertRaises(self.contract.ContractError): self.check(job)


if __name__ == '__main__':
    unittest.main()
