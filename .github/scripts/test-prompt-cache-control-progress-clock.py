#!/usr/bin/env python3
"""Check that current pressure evidence preserves the strict progress clock."""
import datetime
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / 'scripts/prompt-cache-control-loadgen.py'
spec = importlib.util.spec_from_file_location('prompt_cache_control', SOURCE)
loadgen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(loadgen)


class ProgressClockTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.log = Path(self.directory.name) / 'pressure.log'
        self.log.write_text('')
        self.state = dict(maintenance_enabled=True, scheduler_status='running',
                          scheduler_next_run_after=None,
                          latest_defer_reason='coordinator_priority',
                          scheduler_defer_reason='coordinator_priority')
        self.now = datetime.datetime(2026, 10, 3, tzinfo=datetime.timezone.utc)

    def test_stale_priority_reason_does_not_hide_45_second_stall(self):
        probe = dict(eligible_since=None, durable_seen=False, staging_seen=False,
                     baseline_cursor=list(loadgen.progress_cursor(self.state)),
                     baseline_staging_cursor=0, max_durable_eligible_wait_seconds=0,
                     max_staging_eligible_wait_seconds=0, deadline_failures=[])
        with patch.dict(os.environ, PROMPT_CACHE_PRESSURE_LOG=str(self.log)):
            loadgen.observe_progress(probe, dict(self.state), 0)
            loadgen.observe_progress(probe, dict(self.state), 45)
        self.assertEqual(probe['deadline_failures'], ['durable', 'staging'])

    def test_fresh_denial_excludes_only_its_unexpired_window(self):
        self.log.write_text(
            '2026-10-03T00:00:00Z INFO task="prompt-cache conversation materialization" '
            'next_eligibility=2026-10-03 00:00:20 UTC '
            'startup backfill task deferred before SQLite access\n')
        state = {**self.state, 'latest_defer_reason': None, 'scheduler_defer_reason': None}
        self.assertEqual(loadgen.progress_eligibility(
            state, self.now + datetime.timedelta(seconds=5), self.log), 'pressure_deadline')
        self.assertEqual(loadgen.progress_eligibility(
            state, self.now + datetime.timedelta(seconds=21), self.log), 'eligible')

    def test_future_statistics_retry_is_not_eligible_work(self):
        state = {**self.state, 'scheduler_status': 'idle',
                 'scheduler_defer_reason': 'stats_page_pending',
                 'scheduler_next_run_after': '2026-10-03T00:00:15Z'}
        self.assertEqual(loadgen.progress_eligibility(state, self.now, self.log), 'pressure_deadline')
        self.assertEqual(loadgen.progress_eligibility(
            state, self.now + datetime.timedelta(seconds=16), self.log), 'eligible')

    def test_fractional_pressure_deadline_preserves_exact_expiry(self):
        self.log.write_text(
            '2026-10-03T00:00:20.001Z INFO task="prompt-cache conversation materialization" '
            'next_eligibility=2026-10-03 00:00:20.077 UTC '
            'startup backfill task deferred before SQLite access\n')
        before_expiry = self.now + datetime.timedelta(seconds=20, milliseconds=50)
        expiry = self.now + datetime.timedelta(seconds=20, milliseconds=77)
        self.assertEqual(loadgen.progress_eligibility(
            self.state, before_expiry, self.log), 'pressure_deadline')
        self.assertEqual(loadgen.fresh_pressure_deadline(before_expiry, self.log), expiry)
        self.assertEqual(loadgen.progress_eligibility(self.state, expiry, self.log), 'eligible')

    def test_missing_pressure_evidence_fails_instead_of_exempting_a_stall(self):
        self.log.unlink()
        with self.assertRaises(FileNotFoundError):
            loadgen.progress_eligibility(self.state, self.now, self.log)

    def test_zero_work_priority_yield_is_observed_from_fresh_process_evidence(self):
        self.log.write_text(
            '2026-10-03T00:00:05Z INFO task="prompt-cache conversation materialization" '
            'defer_reason="coordinator_priority" '
            'startup backfill task yielded at a prompt-cache micro-batch boundary\n')
        self.assertEqual(loadgen.fresh_priority_yields(self.now.timestamp(), self.log),
                         ['2026-10-03T00:00:05Z'])
        self.assertEqual(loadgen.fresh_priority_yields(
            self.now.timestamp() + 6, self.log), [])

    def test_unrelated_task_pressure_cannot_supply_priority_coverage(self):
        self.log.write_text(
            '2026-10-03T00:00:05Z INFO task="other task" '
            'defer_reason="coordinator_priority" '
            'startup backfill task yielded at a prompt-cache micro-batch boundary\n')
        self.assertEqual(loadgen.fresh_priority_yields(self.now.timestamp(), self.log), [])


if __name__ == '__main__':
    unittest.main()
