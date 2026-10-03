#!/usr/bin/env python3
"""Synthetic multi-conversation checkpoint replay in a real dual-database service."""

import concurrent.futures
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import time

spec = importlib.util.spec_from_file_location('control_workload', Path(__file__).with_name('prompt-cache-control-loadgen.py'))
control_workload = importlib.util.module_from_spec(spec)
spec.loader.exec_module(control_workload)
base = control_workload
COUNTS = [226, 7501, 58, 311, 465, 60431] + [7] * 394
assert sum(COUNTS) == 71750


def key(round_index, index):
    return f'acceptance-r{round_index}-key-{index:03d}'


def emit(phase, round_index, **values):
    print(json.dumps({'phase': phase, 'round': round_index, **values}, ensure_ascii=False), flush=True)


def accumulator(prompt_cache_key, conversation_id, count, occurred_at, invoke_id=None):
    result = dict.fromkeys([
        'request_count', 'success_count', 'failure_count', 'input_tokens', 'output_tokens',
        'cache_input_tokens', 'reported_cache_write_tokens', 'reasoning_tokens', 'total_tokens',
        'cost', 'cost_input', 'cost_cache_write', 'cost_cache_read', 'cost_output', 'cost_reasoning',
    ], 0)
    result.update(prompt_cache_key=prompt_cache_key, conversation_id=conversation_id,
                  max_invoke_id=invoke_id, request_count=count, success_count=count,
                  input_tokens=count, output_tokens=count, total_tokens=count * 2,
                  first_invocation_at=occurred_at if count else None,
                  last_invocation_at=occurred_at if count else None)
    return result


def seed_history(round_index):
    base.control(False)
    base.seed_account()
    with base.open_db(base.BUSINESS_DB, timeout=20) as db:
        db.execute('BEGIN IMMEDIATE')
        occurred_at = '2026-09-01T00:00:00.000Z'
        sequence = 0
        rows = []
        for index, count in enumerate(COUNTS):
            payload = json.dumps({'model': 'gpt-5', 'promptCacheKey': key(round_index, index), 'upstreamAccountId': 1})
            for item in range(count):
                rows.append((f'checkpoint-history-{round_index}-{sequence:06d}', occurred_at,
                             'proxy', 'success', payload, '{}', 'full', 'gpt-5', 1, 1, 2))
                sequence += 1
        db.executemany(
            'INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,payload,raw_response,detail_level,model,input_tokens,output_tokens,total_tokens) VALUES (?,?,?,?,?,?,?,?,?,?,?)', rows)
        db.execute('INSERT OR IGNORE INTO startup_backfill_progress (task_name,enabled) VALUES (?,0)', (base.TASK_NAME,))
        db.execute('UPDATE startup_backfill_progress SET enabled=0 WHERE task_name=?', (base.TASK_NAME,))
        db.execute('DELETE FROM schema_refresh_migrations WHERE migration_name IN (?,?)', ('prompt_cache_conversations_v1', base.STATS_MARKER))
        snapshot_max = db.execute('SELECT MAX(id) FROM codex_invocations').fetchone()[0]
        phase = 'identity_backfill'
        cursor = None
        completed = 0
        if round_index in (2, 3):
            # This is the existing durable shape from 2.85.1, including an empty
            # outer rebuild cursor and a committed generation-bound partial page.
            alphabet = 'ABCDEFGHJKMNPQRSTUVWXYZ23456789'
            for index in range(400):
                conversation_id = 'TEST' + alphabet[index // 31] + alphabet[index % 31]
                db.execute('INSERT INTO prompt_cache_conversations (prompt_cache_key,conversation_id) VALUES (?,?)', (key(round_index, index), conversation_id))
            db.execute('INSERT OR IGNORE INTO schema_refresh_migrations (migration_name) VALUES (?)', ('prompt_cache_conversations_v1',))
            completed = 400
            phase = 'stats_rebuild' if round_index == 2 else 'queue_drain'
            if round_index == 2:
                first = key(round_index, 0)
                db.execute('UPDATE prompt_cache_conversations SET request_count=226,success_count=226,input_tokens=226,output_tokens=226,total_tokens=452,first_invocation_at=?,last_invocation_at=? WHERE prompt_cache_key=?', (occurred_at, occurred_at, first))
                db.execute('DELETE FROM prompt_cache_conversation_stats_refresh_queue WHERE prompt_cache_key=?', (first,))
                staged_index, staged_count = 1, 512
            else:
                staged_index, staged_count = 1, 256
                cursor = 'legacy-queue-cursor'
            staged_key = key(round_index, staged_index)
            generation = db.execute('SELECT generation FROM prompt_cache_conversation_stats_refresh_queue WHERE prompt_cache_key=?', (staged_key,)).fetchone()[0]
            conversation_id = db.execute('SELECT conversation_id FROM prompt_cache_conversations WHERE prompt_cache_key=?', (staged_key,)).fetchone()[0]
            last_id, last_invoke_id = db.execute('SELECT id,invoke_id FROM codex_invocations WHERE json_extract(payload,\'$.promptCacheKey\')=? ORDER BY occurred_at,id LIMIT 1 OFFSET ?', (staged_key, staged_count - 1)).fetchone()
            db.execute('INSERT INTO prompt_cache_conversation_stats_refresh_staging (prompt_cache_key,generation,source_max_invocation_id,cursor_occurred_at,cursor_id,accumulator_json,page_size,updated_at) VALUES (?,?,?,?,?,?,256,?)',
                       (staged_key, generation, snapshot_max, occurred_at, last_id,
                        json.dumps(accumulator(staged_key, conversation_id, staged_count, occurred_at, last_invoke_id)), occurred_at))
        db.execute('UPDATE prompt_cache_conversation_migration_progress SET phase=?,source_max_invocation_id=?,cursor_key=?,total_keys=400,completed_keys=? WHERE migration_name=?', (phase, snapshot_max if phase != 'identity_backfill' else 0, cursor, completed, base.MATERIALIZATION_NAME))
        # Test-only audit triggers observe production commits. They are fixture
        # instrumentation, never installed by the application or on production.
        db.executescript('''
CREATE TABLE checkpoint_events (id INTEGER PRIMARY KEY, prompt_cache_key TEXT, generation INTEGER, cursor_id INTEGER, request_count INTEGER, kind TEXT);
CREATE TRIGGER checkpoint_stage_created AFTER INSERT ON prompt_cache_conversation_stats_refresh_staging BEGIN
  INSERT INTO checkpoint_events (prompt_cache_key,generation,cursor_id,request_count,kind) VALUES (NEW.prompt_cache_key,NEW.generation,NEW.cursor_id,json_extract(NEW.accumulator_json,'$.request_count'),'stage_created'); END;
CREATE TRIGGER checkpoint_page_committed AFTER UPDATE ON prompt_cache_conversation_stats_refresh_staging BEGIN
  INSERT INTO checkpoint_events (prompt_cache_key,generation,cursor_id,request_count,kind) VALUES (NEW.prompt_cache_key,NEW.generation,NEW.cursor_id,json_extract(NEW.accumulator_json,'$.request_count'),'page_committed'); END;
CREATE TRIGGER checkpoint_aggregate_published AFTER UPDATE OF request_count ON prompt_cache_conversations BEGIN
  INSERT INTO checkpoint_events (prompt_cache_key,generation,cursor_id,request_count,kind) VALUES (NEW.prompt_cache_key,COALESCE((SELECT generation FROM prompt_cache_conversation_stats_generation_clock WHERE prompt_cache_key=NEW.prompt_cache_key),0),0,NEW.request_count,'published'); END;
CREATE TABLE checkpoint_queue_deletions (id INTEGER PRIMARY KEY, prompt_cache_key TEXT, generation INTEGER);
CREATE TRIGGER checkpoint_queue_deleted AFTER DELETE ON prompt_cache_conversation_stats_refresh_queue BEGIN
  INSERT INTO checkpoint_queue_deletions (prompt_cache_key,generation) VALUES (OLD.prompt_cache_key,OLD.generation); END;
CREATE TABLE checkpoint_cursor_events (id INTEGER PRIMARY KEY, phase TEXT, cursor_key TEXT);
CREATE TRIGGER checkpoint_cursor_advanced AFTER UPDATE OF cursor_key ON prompt_cache_conversation_migration_progress
WHEN OLD.cursor_key IS NOT NEW.cursor_key AND NEW.phase='stats_rebuild'
BEGIN INSERT INTO checkpoint_cursor_events (phase,cursor_key) VALUES (NEW.phase,NEW.cursor_key); END;
''')
        db.commit()
    emit('checkpoint-fixture', round_index, historical_keys=400, historical_invocations=sequence,
         first_six_counts=COUNTS[:6], remaining_key_count=394, remaining_count_each=7,
         initial_phase=phase, initial_cursor=cursor,
         recovery='cold' if round_index == 1 else '2.85.1-partial-staging' if round_index == 2 else 'queue-drain')


def snapshot(round_index):
    state = base.snapshot()
    status, body, _ = base.request('GET', '/api/system/prompt-cache/materialization')
    state['api_status'] = status
    state['api_eta'] = json.loads(body).get('estimatedRemainingMs') if status == 200 else None
    with base.open_db(base.BUSINESS_DB) as db:
        state['pages_committed'] = db.execute("SELECT COUNT(*) FROM checkpoint_events WHERE kind='page_committed'").fetchone()[0]
        state['events_max_id'] = db.execute('SELECT COALESCE(MAX(id),0) FROM checkpoint_events').fetchone()[0]
        state['first_six'] = [dict(zip(('key', 'published_count', 'staging_count', 'staging_cursor', 'generation'), row)) for row in db.execute(
            'SELECT c.prompt_cache_key,c.request_count,COALESCE(json_extract(s.accumulator_json,\'$.request_count\'),0),COALESCE(s.cursor_id,0),COALESCE(q.generation,g.generation,0) FROM prompt_cache_conversations c LEFT JOIN prompt_cache_conversation_stats_refresh_staging s USING(prompt_cache_key) LEFT JOIN prompt_cache_conversation_stats_refresh_queue q USING(prompt_cache_key) LEFT JOIN prompt_cache_conversation_stats_generation_clock g USING(prompt_cache_key) WHERE c.prompt_cache_key LIKE ? ORDER BY c.prompt_cache_key LIMIT 6', (f'acceptance-r{round_index}-key-%',))]
    return state


def audit(round_index):
    failures = []
    publications = {}
    page_counts = {}
    with base.open_db(base.BUSINESS_DB, timeout=10) as db:
        rows = db.execute('SELECT id,prompt_cache_key,generation,cursor_id,request_count,kind FROM checkpoint_events ORDER BY id').fetchall()
        for event_id, prompt_cache_key, generation, cursor_id, count, kind in rows:
            if not prompt_cache_key.startswith(f'acceptance-r{round_index}-key-'):
                continue
            identity = (prompt_cache_key, generation)
            if kind == 'published':
                publications[identity] = publications.get(identity, 0) + 1
            elif kind == 'stage_created' and publications.get(identity, 0):
                failures.append({'kind': 'completed_key_restarted', 'event_id': event_id, 'key': prompt_cache_key, 'generation': generation})
            elif kind == 'page_committed':
                previous = page_counts.get(identity, 0)
                if count <= previous:
                    failures.append({'kind': 'same_generation_cursor_reset', 'event_id': event_id, 'key': prompt_cache_key, 'generation': generation})
                page_counts[identity] = count
        repeated = [{'key': identity[0], 'generation': identity[1], 'publications': count} for identity, count in publications.items() if count > 1]
        actual = db.execute('SELECT prompt_cache_key,request_count FROM prompt_cache_conversations WHERE prompt_cache_key LIKE ? ORDER BY prompt_cache_key', (f'acceptance-r{round_index}-key-%',)).fetchall()
        prefix_cursors = [row[0] for row in db.execute(
            "SELECT cursor_key FROM checkpoint_cursor_events WHERE phase='stats_rebuild' ORDER BY id")]
        queue_deletions = [(row[0], row[1]) for row in db.execute(
            'SELECT prompt_cache_key,generation FROM checkpoint_queue_deletions ORDER BY id')
            if row[0].startswith(f'acceptance-r{round_index}-key-')]
        raw_evidence = {
            'page_and_publication_events': [dict(id=row[0], key=row[1], generation=row[2], cursor_id=row[3], request_count=row[4], kind=row[5]) for row in rows if row[1].startswith(f'acceptance-r{round_index}-key-')],
            'cursor_events': [{'phase': 'stats_rebuild', 'cursor_key': cursor} for cursor in prefix_cursors],
            'queue_deletions': [{'key': item[0], 'generation': item[1]} for item in queue_deletions],
        }
    evidence_path = Path(base.DATA_DIR, f'checkpoint-events-r{round_index}.json')
    evidence_path.write_text(json.dumps(raw_evidence, separators=(',', ':')))
    evidence_digest = hashlib.sha256(evidence_path.read_bytes()).hexdigest()
    expected_cursors = [key(round_index, index) for index in range(400)]
    return {'events': len(rows), 'repeat_publications': repeated, 'restart_failures': failures,
            'continuous_prefix_passed': prefix_cursors == expected_cursors,
            'prefix_cursor_count': len(prefix_cursors), 'prefix_cursor_first': prefix_cursors[0] if prefix_cursors else None,
            'prefix_cursor_last': prefix_cursors[-1] if prefix_cursors else None,
            'queue_deletions_exactly_once': len(queue_deletions) == 400 and len({item[0] for item in queue_deletions}) == 400,
            'queue_deletion_count': len(queue_deletions), 'checkpoint_evidence_file': evidence_path.name,
            'checkpoint_evidence_sha256': evidence_digest,
            'exact_history_counts': len(actual) == 400 and [row[1] for row in actual] == COUNTS}


def sample(round_index, phase, started):
    try:
        state = snapshot(round_index)
        state['eligibility'] = base.progress_eligibility(state)
    except (sqlite3.Error, OSError) as error:
        state = {'snapshot_error': str(error)}
    emit(phase, round_index, elapsed_seconds=round(time.monotonic() - started, 3), **state)
    return state


def run_input(round_index, duration_seconds, request_rate, mode):
    started = time.monotonic()
    started_epoch = time.time()
    deadline = started + duration_seconds
    base.control(True, 'managed')
    baseline = snapshot(round_index)
    next_request = started
    pending = set()
    results = []
    priority_path = Path(base.DATA_DIR, f'priority-probe-r{round_index}.json')
    priority = json.loads(priority_path.read_text()) if priority_path.exists() else {'samples': []}
    sequence = 0
    paused = resumed = False
    pause_elapsed = None
    pause_state = None
    eligible_start = None
    first_progress = first_staging = None
    last_signature = None
    progress_deadline_failures = []
    eta_failures = []
    observed_second = -1
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
        while time.monotonic() < deadline:
            now = time.monotonic()
            elapsed = now - started
            if not paused and elapsed >= 20:
                pause_candidate = sample(round_index, 'checkpoint-pause-probe', started)
                pause_ready = round_index != 3 or (
                    pause_candidate.get('phase') == 'queue_drain'
                    and pause_candidate.get('staging_max_cursor', 0) > 256
                )
                if pause_ready:
                    base.control(False)
                    paused = True
                    pause_elapsed = elapsed
                    pause_state = snapshot(round_index)
                    emit('checkpoint-pause', round_index, elapsed_seconds=round(elapsed, 3), state=pause_state)
            if paused and not resumed and elapsed >= pause_elapsed + 5:
                base.control(True, 'managed')
                resumed = True
                base.control(True, 'managed')
                emit('checkpoint-resume', round_index, repeated_same_value=True)
            for future in list(pending):
                if future.done():
                    result = future.result()
                    results.append(result)
                    emit('checkpoint-proxy-sample', round_index, elapsed_seconds=round(elapsed, 3), **result)
                    pending.remove(future)
            if sequence < duration_seconds * request_rate and len(pending) < 2 and now >= next_request:
                pending.add(executor.submit(base.proxy_once, f'checkpoint-r{round_index}-{sequence}', f'checkpoint-online-r{round_index}'))
                sequence += 1
                next_request += 1 / request_rate
            second = int(elapsed)
            if second != observed_second:
                observed_second = second
                state = sample(round_index, 'checkpoint-input', started)
                if state.get('phase') in ('stats_rebuild', 'queue_drain') and state.get('api_eta') is not None:
                    eta_failures.append(second)
                signature = (state.get('outer_cursor'), state.get('queue_count'), state.get('pages_committed'))
                progressed = signature != last_signature and last_signature is not None
                last_signature = signature
                if state.get('eligibility') == 'eligible' and state.get('phase') != 'complete':
                    if eligible_start is None:
                        eligible_start = elapsed
                    if progressed:
                        if first_progress is None:
                            first_progress = elapsed
                        eligible_start = elapsed
                    elif elapsed - eligible_start > 30:
                        progress_deadline_failures.append(round(elapsed, 3))
                else:
                    eligible_start = None
                if first_staging is None and state.get('pages_committed', 0) > baseline['pages_committed']:
                    first_staging = elapsed
            time.sleep(0.01)
        for future in concurrent.futures.as_completed(pending, timeout=20):
            result = future.result()
            results.append(result)
            emit('checkpoint-proxy-sample', round_index, elapsed_seconds=round(time.monotonic() - started, 3), **result)
    measured = results + priority['samples']
    parse = [r['terminal']['request_parse_ms'] for r in measured if r['terminal'] and r['terminal']['request_parse_ms'] is not None]
    terminal = [r['terminal_ms'] for r in measured if r['terminal_ms'] is not None]
    failures = []
    if len(results) != duration_seconds * request_rate or any(r['status'] != 200 or r['terminal'] is None for r in results):
        failures.append('request_or_terminal_loss')
    if base.percentile(parse, .99) is None or base.percentile(parse, .99) > 100:
        failures.append('allocation_p99')
    if base.percentile(terminal, .99) is None or base.percentile(terminal, .99) > 1000:
        failures.append('terminal_confirm_p99')
    if progress_deadline_failures:
        failures.append('eligible_progress_deadline')
    if round_index == 1 and not priority['samples']:
        failures.append('priority_yield_probe_missing')
    if mode == 'candidate' and eta_failures:
        failures.append('incomplete_eta_contract')
    if first_staging is None:
        failures.append('no_committed_stats_page')
    if round_index == 3 and (pause_state or {}).get('phase') != 'queue_drain':
        failures.append('pause_outside_queue_drain')
    if not paused or not resumed:
        failures.append('pause_resume')
    summary = {'mode': mode, 'submitted': sequence, 'completed': len(results), 'failures': failures,
               'allocation_p99_ms': base.percentile(parse, .99), 'terminal_confirm_p99_ms': base.percentile(terminal, .99),
               'first_progress_seconds': first_progress, 'first_staging_seconds': first_staging,
               'progress_deadline_failures': progress_deadline_failures, 'input_finished_epoch': time.time(),
               'round_started_epoch': started_epoch, 'priority_probe_calls': len(priority['samples']), 'eta_failures': eta_failures, 'paused': paused, 'resumed': resumed, 'pause_phase': (pause_state or {}).get('phase')}
    emit('checkpoint-input-summary', round_index, **summary)
    Path(base.DATA_DIR, f'checkpoint-input-r{round_index}.json').write_text(json.dumps(summary))


def observe(round_index, duration_seconds, mode):
    summary = json.loads(Path(base.DATA_DIR, f'checkpoint-input-r{round_index}.json').read_text())
    started = time.monotonic()
    observation_deadline = started + duration_seconds
    completion_epoch = None
    stable_runs = None
    stable_events = None
    stable_failure = False
    eligible_start = None
    progress_failures = []
    last_pages = None
    observed_second = -1
    states = []
    while time.monotonic() < observation_deadline:
        elapsed = time.monotonic() - started
        second = int(elapsed)
        if second != observed_second:
            observed_second = second
            state = sample(round_index, 'checkpoint-observe', started)
            states.append(state)
            complete = state.get('phase') == 'complete' and state.get('queue_count') == 0 and state.get('staging_count') == 0 and state.get('stats_marker') is True
            if complete:
                if completion_epoch is None:
                    completion_epoch = time.time()
                    stable_runs = state['materialization_run_count']
                    stable_events = state['events_max_id']
                elif state['materialization_run_count'] != stable_runs or state['events_max_id'] != stable_events:
                    stable_failure = True
            elif state.get('eligibility') == 'eligible':
                if eligible_start is None:
                    eligible_start = elapsed
                if last_pages is not None and state.get('pages_committed') != last_pages:
                    eligible_start = elapsed
                elif elapsed - eligible_start > 30:
                    progress_failures.append(second)
            else:
                eligible_start = None
            last_pages = state.get('pages_committed')
        time.sleep(.1)
    final = snapshot(round_index)
    result = audit(round_index)
    status, body, _ = base.request('GET', '/api/system/prompt-cache/materialization')
    api = json.loads(body) if status == 200 else {}
    failures = list(summary['failures'])
    completion_after_input = completion_epoch - summary['input_finished_epoch'] if completion_epoch else None
    if completion_after_input is None or completion_after_input > 180:
        failures.append('completion_after_input_180s')
    if result['repeat_publications'] or result['restart_failures']:
        failures.append('completed_prefix_repeated')
    if mode == 'candidate' and round_index < 3 and not result['continuous_prefix_passed']:
        failures.append('non_continuous_stats_cursor')
    if mode == 'candidate' and not result['queue_deletions_exactly_once']:
        failures.append('queue_checkpoint_missing_or_repeated')
    if not result['exact_history_counts']:
        failures.append('inexact_aggregate')
    if final['phase'] != 'complete' or final['queue_count'] or final['staging_count'] or not final['stats_marker']:
        failures.append('inconsistent_completion')
    if stable_failure:
        failures.append('repeated_runs_after_completion')
    if progress_failures:
        failures.append('eligible_progress_deadline')
    if status != 200 or api.get('estimatedRemainingMs') != 0:
        failures.append('complete_eta_contract')
    output = {'mode': mode, 'passed': not failures, 'failures': failures, 'completion_after_input_seconds': completion_after_input,
              'stable_observation_seconds': time.time() - completion_epoch if completion_epoch else 0,
              'final': final, 'audit': result, 'input': summary}
    if mode == 'baseline':
        output['baseline_defect_reproduced'] = bool(result['repeat_publications'] or result['restart_failures'])
    emit('checkpoint-completion', round_index, **output)
    Path(base.DATA_DIR, f'checkpoint-result-r{round_index}.json').write_text(json.dumps(output))
    if mode == 'candidate' and failures:
        raise SystemExit(f'checkpoint candidate failed: {failures}')
    if mode == 'baseline' and not output['baseline_defect_reproduced']:
        raise SystemExit('baseline did not reproduce repeated completed prefix')


def compatibility_read(round_index):
    with base.open_db(base.BUSINESS_DB) as db:
        invocation_count_before = db.execute('SELECT COUNT(*) FROM codex_invocations').fetchone()[0]
    health, health_body, _ = base.request('GET', '/health')
    status, body, _ = base.request('GET', '/api/system/prompt-cache/materialization')
    status_body = json.loads(body) if status == 200 else {}
    integrity = {}
    for name, path in (('business', base.BUSINESS_DB), ('maintenance', base.MAINTENANCE_DB)):
        with base.open_db(path, timeout=5) as db:
            integrity[name] = db.execute('PRAGMA integrity_check').fetchone()[0]
    with base.open_db(base.BUSINESS_DB) as db:
        invocation_count_after = db.execute('SELECT COUNT(*) FROM codex_invocations').fetchone()[0]
    result = {
        'phase': 'older-reader-compatibility',
        'round': round_index,
        'health_status': health,
        'health_body': health_body,
        'status_get_status': status,
        'status_phase': status_body.get('phase'),
        'status_queue_pending': status_body.get('queuePending'),
        'status_enabled': status_body.get('enabled'),
        'integrity': integrity,
        'invocation_count_before': invocation_count_before,
        'invocation_count_after': invocation_count_after,
        'invocation_count_preserved': invocation_count_before == invocation_count_after,
    }
    emit('older-reader-compatibility', round_index, **result)
    if (
        health != 200 or health_body != 'ok' or status != 200
        or status_body.get('phase') != 'complete'
        or status_body.get('queuePending') != 0
        or status_body.get('enabled') is not True
        or any(value != 'ok' for value in integrity.values())
        or not result['invocation_count_preserved']
    ):
        raise SystemExit('the earlier service could not read the completed candidate database')


COMMANDS = {
    'wait-health': base.wait_health,
    'seed-history': lambda: seed_history(int(sys.argv[2])),
    'priority-yield-probe': lambda: base.priority_yield_probe(int(sys.argv[2])),
    'maintenance-lock-probe': lambda: base.maintenance_lock_probe(int(sys.argv[2])),
    'compatibility-read': lambda: compatibility_read(int(sys.argv[2])),
    'input': lambda: run_input(int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), sys.argv[5]),
    'observe': lambda: observe(int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]),
}
if __name__ == '__main__':
    if len(sys.argv) < 2 or sys.argv[1] not in COMMANDS:
        raise SystemExit(f'usage: {sys.argv[0]} <{",".join(COMMANDS)}> [args]')
    COMMANDS[sys.argv[1]]()
