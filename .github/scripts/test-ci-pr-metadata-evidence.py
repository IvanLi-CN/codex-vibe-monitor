#!/usr/bin/env python3
"""Adversarial, offline tests for the trusted metadata evidence gate."""
import copy
import datetime as dt
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location('evidence', Path(__file__).with_name('ci_pr_metadata_evidence.py'))
evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evidence)
HEAD, BASE, MERGE = 'a' * 40, 'b' * 40, 'c' * 40
NOW = dt.datetime(2026, 11, 8, tzinfo=dt.timezone.utc)
ENV = {'GITHUB_EVENT_NAME': 'pull_request', 'GITHUB_REPOSITORY': 'owner/repo', 'GITHUB_SHA': MERGE,
       'GITHUB_RUN_ID': '200', 'GITHUB_RUN_ATTEMPT': '1'}
EVENT = {'action': 'edited', 'changes': {'body': {}}, 'pull_request': {
    'number': 1, 'head': {'sha': HEAD, 'repo': {'full_name': 'owner/repo'}},
    'base': {'sha': BASE, 'ref': 'main', 'repo': {'full_name': 'owner/repo'}}}}
EXPECTED = evidence.event_proof(EVENT, ENV)


def run(identity=100, mode='full', conclusion='success', attempt=1):
    source = {'id': identity, 'run_number': identity, 'run_attempt': attempt,
              'event': 'pull_request', 'head_sha': HEAD, 'head_repository': {'full_name': 'owner/repo'},
              'status': 'completed', 'conclusion': conclusion, 'run_started_at': NOW.isoformat()}
    proof = dict(EXPECTED, run_id=identity, run_attempt=attempt, mode=mode)
    jobs = [{'name': name, 'run_attempt': attempt, 'status': 'completed', 'conclusion': 'success',
             'started_at': NOW.isoformat(), 'completed_at': NOW.isoformat()} for name in evidence.JOB_NAMES]
    if mode == 'metadata':
        for job in jobs:
            if job['name'] != 'Repository Tooling Checks':
                job['conclusion'] = 'skipped'
    return source, proof, jobs


class FakeAPI:
    def __init__(self, sources=None):
        self.sources = sources or [run()]
        self.pr = dict(EVENT['pull_request'], state='open')
        self.parents = [HEAD, BASE]
        self.live_reads = 0
        self.change_on_final_read = False

    def request(self, path):
        if path.startswith('/pulls/'):
            self.live_reads += 1
            if self.change_on_final_read and self.live_reads == 2:
                return dict(self.pr, head={'sha': 'd' * 40})
            return self.pr
        if path.startswith('/git/commits/'):
            return {'parents': [{'sha': sha} for sha in self.parents]}
        raise AssertionError(path)

    def pages(self, path, key):
        if key == 'workflow_runs':
            return [source[0] for source in self.sources]
        if key == 'jobs':
            return next(jobs for source, _, jobs in self.sources if f'/runs/{source["id"]}/' in path)
        raise AssertionError(path)

    def proof(self, source):
        proof = next(proof for run_, proof, _ in self.sources if run_['id'] == source['id'])
        if isinstance(proof, Exception):
            raise proof
        return proof


class EvidenceTests(unittest.TestCase):
    def validate(self, api=None):
        return evidence.validate_once(api or FakeAPI(), EXPECTED, now=NOW)

    def test_success_and_current_run_exclusion(self):
        self.assertEqual(self.validate(), 100)
        self.assertEqual(self.validate(FakeAPI([run(), run(200, 'metadata')])), 100)

    def test_metadata_chain_and_failed_metadata_do_not_hide_full_run(self):
        api = FakeAPI([run(), run(101, 'metadata'), run(102, 'metadata', 'failure')])
        self.assertEqual(self.validate(api), 100)

    def test_newest_full_failure_cannot_fall_back(self):
        for conclusion in ('failure', 'cancelled', 'timed_out', 'skipped', 'neutral'):
            with self.subTest(conclusion=conclusion), self.assertRaises(evidence.EvidenceError):
                self.validate(FakeAPI([run(), run(101, conclusion=conclusion)]))

    def test_newest_full_pending_waits(self):
        newer = run(101)
        newer[0]['status'] = 'in_progress'
        with self.assertRaises(evidence.Pending):
            self.validate(FakeAPI([run(), newer]))

    def test_older_run_later_attempt_outranks_newer_run_number(self):
        earlier_pass = run(101)
        earlier_pass[0]['run_started_at'] = (NOW - dt.timedelta(hours=1)).isoformat()
        for conclusion in ('failure', 'cancelled', 'timed_out'):
            later_attempt = run(100, conclusion=conclusion, attempt=2)
            with self.subTest(conclusion=conclusion), self.assertRaises(evidence.EvidenceError):
                self.validate(FakeAPI([earlier_pass, later_attempt]))
        later_attempt = run(100, attempt=2)
        later_attempt[0]['status'] = 'in_progress'
        with self.assertRaises(evidence.Pending):
            self.validate(FakeAPI([earlier_pass, later_attempt]))
        later_attempt = run(100, attempt=2)
        later_attempt[2][0]['started_at'] = (NOW - dt.timedelta(hours=2)).isoformat()
        with self.assertRaises(evidence.EvidenceError):
            self.validate(FakeAPI([earlier_pass, later_attempt]))
        # A genuinely complete, later rerun remains reusable.
        self.assertEqual(self.validate(FakeAPI([earlier_pass, run(100, attempt=2)])), 100)

    def test_same_second_attempt_order_is_not_assumed_safe(self):
        # The lower run number could have been rerun after the higher run passed.
        with self.assertRaises(evidence.EvidenceError):
            self.validate(FakeAPI([run(101), run(100, conclusion='failure', attempt=2)]))

    def test_ci_snapshot_change_retries_instead_of_accepting_stale_success(self):
        for change in ('new-full', 'source-attempt', 'source-pending', 'new-metadata'):
            class ChangingAPI(FakeAPI):
                def request(self, path):
                    if path.startswith('/git/commits/'):
                        if change == 'new-full':
                            self.sources.append(run(101, conclusion='failure'))
                        elif change == 'source-attempt':
                            self.sources[0] = run(100, conclusion='failure', attempt=2)
                        elif change == 'source-pending':
                            self.sources[0][0]['status'] = 'in_progress'
                        else:
                            self.sources.append(run(101, 'metadata'))
                    return super().request(path)
            with self.subTest(change=change), self.assertRaises(evidence.Pending):
                self.validate(ChangingAPI())

    def test_missing_newer_marker_never_falls_back(self):
        newer = list(run(101))
        newer[1] = evidence.EvidenceError('No provenance')
        with self.assertRaises(evidence.EvidenceError):
            self.validate(FakeAPI([run(), newer]))

    def test_each_provenance_identity_is_bound(self):
        for key in ('schema_version', 'event_name', 'repository', 'pr_number', 'head_repository', 'head_sha', 'base_sha', 'base_ref', 'run_id', 'run_attempt', 'mode', 'merge_sha'):
            source = run()
            source[1][key] = 'wrong'
            with self.subTest(key=key), self.assertRaises(evidence.EvidenceError):
                self.validate(FakeAPI([source]))

    def test_all_jobs_must_succeed_on_same_attempt(self):
        for state in ('failure', 'skipped', 'cancelled', None):
            source = run()
            source[2][0]['conclusion'] = state
            with self.subTest(state=state), self.assertRaises(evidence.EvidenceError):
                self.validate(FakeAPI([source]))
        source = run(attempt=2)
        source[2][0]['run_attempt'] = 1
        with self.assertRaises(evidence.EvidenceError):
            self.validate(FakeAPI([source]))

    def test_github_cloned_partial_rerun_has_new_attempt_but_old_timestamp(self):
        source = run(attempt=2)
        source[2][0]['started_at'] = (NOW - dt.timedelta(minutes=10)).isoformat()
        with self.assertRaises(evidence.EvidenceError):
            self.validate(FakeAPI([source]))

    def test_incomplete_duplicate_and_unknown_job_sets(self):
        for mutation in ('missing', 'duplicate', 'unknown'):
            source = run()
            if mutation == 'missing':
                source[2].pop()
            elif mutation == 'duplicate':
                source[2].append(source[2][0])
            else:
                source[2][0]['name'] = 'Unexpected'
            with self.subTest(mutation=mutation), self.assertRaises(evidence.EvidenceError):
                self.validate(FakeAPI([source]))

    def test_metadata_marker_cannot_hide_executed_heavy_job(self):
        source = run(101, 'metadata')
        next(j for j in source[2] if j['name'] != 'Repository Tooling Checks')['conclusion'] = 'success'
        with self.assertRaises(evidence.EvidenceError):
            self.validate(FakeAPI([run(), source]))

    def test_freshness_every_job_and_future_time(self):
        for timestamp in (NOW - dt.timedelta(days=7, seconds=1), NOW + dt.timedelta(seconds=1)):
            source = run()
            source[2][0]['completed_at'] = timestamp.isoformat()
            with self.assertRaises(evidence.EvidenceError):
                self.validate(FakeAPI([source]))

    def test_merge_parents_and_live_pr_rechecked(self):
        api = FakeAPI()
        api.parents = [HEAD, 'd' * 40]
        with self.assertRaises(evidence.EvidenceError):
            self.validate(api)
        api = FakeAPI()
        api.change_on_final_read = True
        with self.assertRaises(evidence.EvidenceError):
            self.validate(api)
        self.assertEqual(api.live_reads, 2)

    def test_event_matrix_empty_objects_and_base_changes(self):
        for action, changes, mode in [
            ('edited', {'body': {}}, 'metadata'), ('edited', {'title': {}}, 'metadata'),
            ('edited', {'base': {}, 'body': {}}, 'full'), ('edited', {}, 'full'),
            ('edited', None, 'full'), ('synchronize', {'body': {}}, 'full'),
            ('edited', {'body': None}, 'full'),
        ]:
            event = dict(EVENT, action=action, changes=changes)
            self.assertEqual(evidence.event_proof(event, ENV)['mode'], mode)

    def test_archive_limits_and_exact_member(self):
        for name, body in [('event.json', json.dumps(EXPECTED)), ('../event.json', '{}'), ('event.json', ' ' * 65537)]:
            stream = io.BytesIO()
            with zipfile.ZipFile(stream, 'w') as archive:
                archive.writestr(name, body)
            if name == 'event.json' and len(body) < 65536:
                self.assertEqual(evidence.decode_proof(stream.getvalue()), EXPECTED)
            else:
                with self.assertRaises(evidence.EvidenceError):
                    evidence.decode_proof(stream.getvalue())

    def test_isolated_interpreter_ignores_candidate_startup_hook(self):
        # Run the real interpreter and actual helper, not a simulated validator.
        with tempfile.TemporaryDirectory(prefix='ci-event-startup-') as root:
            Path(root, 'sitecustomize.py').write_text('import os; os._exit(42)\n')
            environment = dict(os.environ, PYTHONPATH=root)
            helper = str(Path(evidence.__file__).resolve())
            vulnerable = subprocess.run([sys.executable, helper, '--help'], cwd=root,
                                        env=environment, capture_output=True, check=False)
            self.assertEqual(vulnerable.returncode, 42)
            isolated = subprocess.run([sys.executable, '-I', helper, '--help'], cwd=root,
                                      env=environment, capture_output=True, check=False)
            self.assertEqual(isolated.returncode, 0, isolated.stderr)
            self.assertIn(b'write-proof', isolated.stdout)

    def test_artifact_redirect_strips_token(self):
        import urllib.error
        redirect = urllib.error.HTTPError('https://api.github.com/', 302, 'Found', {'Location': 'https://artifact.example/proof'}, None)
        response = io.BytesIO(b'zip')
        with patch.object(evidence.urllib.request, 'build_opener') as opener, patch.object(evidence.urllib.request, 'urlopen', return_value=response) as download:
            opener.return_value.open.side_effect = redirect
            api = evidence.GitHub('owner/repo', 'secret')
            self.assertEqual(api.request('/actions/artifacts/1/zip', artifact=True), b'zip')
            download.assert_called_once_with('https://artifact.example/proof', timeout=20)
            request = opener.return_value.open.call_args.args[0]
            self.assertEqual(request.get_header('Authorization'), 'Bearer secret')

    def test_artifact_selection_expired_ambiguous_attempt(self):
        api = evidence.GitHub('owner/repo', 'secret')
        source = run()[0]
        marker = {'id': 1, 'name': evidence.proof_name(100, 1), 'expired': False}
        for artifacts in ([], [dict(marker, expired=True)], [marker, marker], [dict(marker, name=evidence.proof_name(100, 2))]):
            with patch.object(api, 'pages', return_value=artifacts), self.assertRaises(evidence.EvidenceError):
                api.proof(source)


if __name__ == '__main__':
    unittest.main()
