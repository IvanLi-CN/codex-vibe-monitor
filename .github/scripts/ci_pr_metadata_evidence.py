#!/usr/bin/env python3
"""Read-only, fail-closed proof of a complete CI PR run for metadata edits.

Execute this file from the event's base commit, never from the PR checkout.
Artifacts distinguish full runs from metadata runs; a newer unknown/failed full
run must never be hidden by an older successful run.
"""
from __future__ import annotations

import argparse
import datetime as dt
import io
import json
import os
from pathlib import Path
import re
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile

JOB_NAMES = frozenset(['Lint & Format Check', 'Repository Tooling Checks', 'Front-end Tests', 'Storybook Accessibility Tests', 'Docs & Web Demo Build', 'PR E2E Test Producer', 'Records Overlay E2E', 'Backend Test Archive Producer', 'Backend Tests (Lightweight)', 'Backend Tests (Stateful SQLite shard 1/2)', 'Backend Tests (Stateful SQLite shard 2/2)', 'Backend Tests (Stateful SQLite)', 'Backend Tests (Representative Scale)', 'Backend Tests (Archive / File I/O)', 'PR Smoke Artifact Producer', 'Build Artifacts', 'Retention Migration Validation'])
# Kept independent of candidate declarations. Tests bind this list to production.


class EvidenceError(RuntimeError):
    pass


class Pending(EvidenceError):
    pass


def require(condition, message):
    if not condition:
        raise EvidenceError(message)


def sha(value):
    require(isinstance(value, str) and re.fullmatch(r'[0-9a-f]{40}', value), 'Invalid commit identity')
    return value


def number(value):
    require(str(value).isdigit() and int(value) > 0, 'Invalid numeric identity')
    return int(value)


def proof_name(run_id, attempt):
    return f'ci-pr-event-proof-{number(run_id)}-{number(attempt)}'


def event_proof(event, env):
    kind = env['GITHUB_EVENT_NAME']
    require(kind in ('pull_request', 'merge_group'), 'Unsupported CI event')
    changes = event.get('changes') or {}
    # GitHub expressions consider empty objects truthy. Presence is deliberate.
    def truthy(value):
        return isinstance(value, (dict, list)) or bool(value)

    metadata = (kind == 'pull_request' and event.get('action') == 'edited'
                and not truthy(changes.get('base'))
                and any(truthy(changes.get(key)) for key in ('title', 'body')))
    repository = env['GITHUB_REPOSITORY']
    require(re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository), 'Invalid repository')
    pr = event.get('pull_request', {})
    group = event.get('merge_group', {})
    return {
        'schema_version': 1, 'mode': 'metadata' if metadata else 'full',
        'event_name': kind, 'action': event.get('action'), 'repository': repository,
        'pr_number': number(pr['number']) if kind == 'pull_request' else None,
        'head_repository': pr['head']['repo']['full_name'] if kind == 'pull_request' else repository,
        'head_sha': sha(pr['head']['sha'] if kind == 'pull_request' else group['head_sha']),
        'base_sha': sha(pr['base']['sha'] if kind == 'pull_request' else group['base_sha']),
        'base_ref': pr['base']['ref'] if kind == 'pull_request' else group['base_ref'],
        'merge_sha': sha(env['GITHUB_SHA']),
        'run_id': number(env['GITHUB_RUN_ID']), 'run_attempt': number(env['GITHUB_RUN_ATTEMPT']),
    }


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class GitHub:
    def __init__(self, repository, token, api='https://api.github.com'):
        require(api == 'https://api.github.com', 'Unsupported GitHub API origin')
        self.root = f'{api}/repos/{repository}'
        self.token = token

    def request(self, path, *, artifact=False):
        require(path.startswith('/') and not path.startswith('//'), 'Invalid API path')
        req = urllib.request.Request(self.root + path, headers={
            'Authorization': f'Bearer {self.token}', 'Accept': 'application/vnd.github+json',
            'X-GitHub-Api-Version': '2022-11-28',
        })
        try:
            response = urllib.request.build_opener(NoRedirect).open(req, timeout=20)
        except urllib.error.HTTPError as exc:
            if artifact and exc.code in (301, 302, 303, 307, 308):
                location = exc.headers.get('Location', '')
                require(urllib.parse.urlsplit(location).scheme == 'https', 'Unsafe artifact redirect')
                # Never forward the API credential to the signed artifact URL.
                response = urllib.request.urlopen(location, timeout=20)
            else:
                raise EvidenceError(f'GitHub evidence request failed (HTTP {exc.code})') from None
        with response:
            payload = response.read(1024 * 1024 + 1)
        require(len(payload) <= 1024 * 1024, 'Oversized evidence response')
        return payload if artifact else json.loads(payload)

    def pages(self, path, key):
        result = []
        separator = '&' if '?' in path else '?'
        for page in range(1, 11):
            data = self.request(f'{path}{separator}per_page=100&page={page}')
            items = data[key]
            result.extend(items)
            if len(items) < 100:
                return result
        raise EvidenceError('Evidence pagination limit reached')

    def proof(self, run):
        artifacts = self.pages(f'/actions/runs/{run["id"]}/artifacts', 'artifacts')
        matching = [a for a in artifacts if a['name'] == proof_name(run['id'], run['run_attempt'])]
        if not matching:
            if run['status'] != 'completed':
                raise Pending(f'Run {run["id"]} has not published event provenance')
            raise EvidenceError(f'Run {run["id"]} lacks event provenance for its current attempt')
        require(len(matching) == 1 and not matching[0]['expired'], 'Ambiguous or expired event provenance')
        payload = self.request(f'/actions/artifacts/{number(matching[0]["id"])}/zip', artifact=True)
        return decode_proof(payload)


def decode_proof(payload):
    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
        entries = archive.infolist()
        require(len(entries) == 1 and entries[0].filename == 'event.json'
                and entries[0].file_size <= 65536, 'Invalid event provenance archive')
        return json.loads(archive.read(entries[0]))


def bind_proof(proof, expected, run):
    for key in ('schema_version', 'event_name', 'repository', 'pr_number', 'head_repository', 'head_sha', 'base_sha', 'base_ref'):
        require(proof.get(key) == expected[key], f'Event provenance mismatch: {key}')
    require(proof.get('run_id') == run['id'] and proof.get('run_attempt') == run['run_attempt'], 'Event provenance attempt mismatch')
    sha(proof.get('merge_sha'))
    require(proof.get('mode') in ('full', 'metadata'), 'Invalid event provenance mode')


def check_live_pr(api, expected):
    pr = api.request(f'/pulls/{expected["pr_number"]}')
    require(pr['state'] == 'open' and pr['head']['sha'] == expected['head_sha']
            and pr['base']['sha'] == expected['base_sha']
            and pr['base']['ref'] == expected['base_ref']
            and pr['head']['repo']['full_name'] == expected['head_repository']
            and pr['base']['repo']['full_name'] == expected['repository'], 'PR identity changed; full CI is required')


def jobs_for_attempt(api, run):
    jobs = api.pages(f'/actions/runs/{run["id"]}/attempts/{run["run_attempt"]}/jobs', 'jobs')
    require(len(jobs) == len(JOB_NAMES) and {j['name'] for j in jobs} == JOB_NAMES, 'Incomplete or unexpected CI job set')
    require(all(j.get('run_attempt') == run['run_attempt'] for j in jobs), 'CI attempt identity mismatch')
    # GitHub clones carried-forward jobs with new IDs AND the new run_attempt.
    # Their original timestamps remain, so the attempt start is the real boundary.
    started = dt.datetime.fromisoformat(run['run_started_at'].replace('Z', '+00:00'))
    require(all(dt.datetime.fromisoformat(j['started_at'].replace('Z', '+00:00')) >= started
                for j in jobs), 'Partial reruns cannot renew full CI evidence')
    return jobs


def evidence_runs(api, expected):
    runs = api.pages('/actions/workflows/ci-pr.yml/runs?event=pull_request&head_sha=' + expected['head_sha'], 'workflow_runs')
    runs = [run for run in runs if run['id'] != expected['run_id']]
    # A later attempt of an older run is newer validation. run_number alone
    # would let an earlier pass hide that attempt's failure or pending state.
    return sorted(runs, key=lambda run: (
        dt.datetime.fromisoformat(run['run_started_at'].replace('Z', '+00:00')),
        run['run_number'], run['run_attempt'],
    ), reverse=True)


def evidence_snapshot(runs):
    # Freeze values, not references: provider state can change during downloads.
    keys = ('id', 'run_number', 'run_attempt', 'run_started_at', 'status', 'conclusion', 'head_sha', 'event')
    return tuple(tuple(run[key] for key in keys) for run in runs)


def validate_once(api, expected, now=None):
    check_live_pr(api, expected)
    runs = evidence_runs(api, expected)
    snapshot = evidence_snapshot(runs)
    require(runs, 'No prior complete CI run exists')
    for index, run in enumerate(runs):
        # Do not reuse a run from another PR or fork sharing a commit.
        require(run['event'] == 'pull_request' and run['head_sha'] == expected['head_sha']
                and run['head_repository']['full_name'] == expected['head_repository'], 'CI run identity mismatch')
        proof = api.proof(run)
        bind_proof(proof, expected, run)
        if proof['mode'] == 'metadata':
            # A provenance marker alone cannot hide a newer full/partial execution.
            jobs = api.pages(f'/actions/runs/{run["id"]}/attempts/{run["run_attempt"]}/jobs', 'jobs')
            heavy = [j for j in jobs if j['name'] != 'Repository Tooling Checks']
            require(len(heavy) == len(JOB_NAMES) - 1 and {j['name'] for j in heavy} == JOB_NAMES - {'Repository Tooling Checks'}
                    and all(j['conclusion'] == 'skipped' for j in heavy), 'Metadata run contains executed or unresolved heavy jobs')
            continue
        if run['status'] != 'completed':
            raise Pending(f'Full CI run {run["id"]} is still running')
        require(run['conclusion'] == 'success', f'Latest full CI run {run["id"]} did not succeed')
        # Provider timestamps have one-second resolution. A lower-numbered
        # rerun in the same second cannot safely be assumed older than this pass.
        require(not any(other['run_started_at'] == run['run_started_at']
                        for other in runs[index + 1:]), 'Ambiguous CI attempt chronology')
        jobs = jobs_for_attempt(api, run)
        require(all(j['status'] == 'completed' and j['conclusion'] == 'success' for j in jobs), 'Full CI contains a failed or skipped job')
        current_time = now or dt.datetime.now(dt.timezone.utc)
        for job in jobs:
            completed = dt.datetime.fromisoformat(job['completed_at'].replace('Z', '+00:00'))
            require(dt.timedelta(0) <= current_time - completed <= dt.timedelta(days=7), 'Full CI evidence is outside the seven-day validity window')
        commit = api.request('/git/commits/' + proof['merge_sha'])
        require({p['sha'] for p in commit['parents']} == {expected['head_sha'], expected['base_sha']}
                and len(commit['parents']) == 2, 'Full CI merge commit does not bind the requested head/base')
        check_live_pr(api, expected)
        # PR identity alone cannot detect a same-head rerun or newly started CI.
        # Retry the entire reduction if its input set changed while validating.
        if evidence_snapshot(evidence_runs(api, expected)) != snapshot:
            raise Pending('CI evidence changed during validation; checking the latest attempt again')
        return run['id']
    raise EvidenceError('No prior full CI run exists')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('write-proof', 'validate'))
    parser.add_argument('--proof-dir', required=True)
    parser.add_argument('--wait-seconds', type=int, default=3600)
    args = parser.parse_args()
    event = json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text())
    expected = event_proof(event, os.environ)
    if args.command == 'write-proof':
        root = Path(args.proof_dir)
        root.mkdir(parents=True, exist_ok=True)
        (root / 'event.json').write_text(json.dumps(expected, sort_keys=True) + '\n')
        return
    require(expected['mode'] == 'metadata', 'Evidence reuse is only allowed for pure metadata edits')
    api = GitHub(expected['repository'], os.environ['GITHUB_TOKEN'], os.environ.get('GITHUB_API_URL', 'https://api.github.com'))
    deadline = time.monotonic() + min(max(args.wait_seconds, 0), 3600)
    while True:
        try:
            source = validate_once(api, expected)
            print(f'Complete CI evidence verified: run {source}, exact head/base, all {len(JOB_NAMES)} jobs succeeded')
            return
        except Pending as exc:
            require(time.monotonic() < deadline, 'Timed out waiting for complete CI evidence')
            print(str(exc), flush=True)
            time.sleep(min(30, max(0, deadline - time.monotonic())))


if __name__ == '__main__':
    try:
        main()
    except (EvidenceError, KeyError, ValueError, OSError, zipfile.BadZipFile):
        # Avoid printing credential-bearing URLs or raw remote responses.
        print('::error::Complete CI evidence unavailable or invalid; run full CI for the current head/base.')
        raise SystemExit(1)
