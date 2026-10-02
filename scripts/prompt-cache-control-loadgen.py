#!/usr/bin/env python3
"""Linux service-process acceptance workload for prompt-cache task control."""

import concurrent.futures
import datetime
import json
import math
import os
import sqlite3
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

BASE_URL = 'http://app:8080'
TOKEN = 'pool-shared-testbox-key'
ACCOUNT_KEY = 'upstream-primary-key'
DIRECT_PROXY_KEY = '__direct__'
TASK_KEY = 'startup_backfill.prompt_cache_conversations_materialization'
TASK_NAME = 'prompt_cache_conversations_materialization_v1'
MATERIALIZATION_NAME = 'prompt_cache_conversations_materialization_v1'
STATS_MARKER = 'prompt_cache_conversations_stats_v2'
DATA_DIR = '/work/data'
BUSINESS_DB = os.path.join(DATA_DIR, 'codex_vibe_monitor.db')
MAINTENANCE_DB = os.path.join(DATA_DIR, 'codex_vibe_monitor.maintenance.sqlite')


def request(method, path, payload=None, timeout=30):
    data = None if payload is None else json.dumps(payload).encode()
    headers = {'Authorization': f'Bearer {TOKEN}'}
    if payload is not None:
        headers['Content-Type'] = 'application/json'
        headers['Content-Length'] = str(len(data))
    req = urllib.request.Request(BASE_URL + path, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            return response.status, response.read().decode(errors='replace'), response.headers
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode(errors='replace'), error.headers


def open_db(path, timeout=0.5):
    connection = sqlite3.connect(path, timeout=timeout)
    connection.execute(f'PRAGMA busy_timeout={int(timeout * 1000)}')
    return connection


def control(enabled, route='dedicated'):
    if route == 'dedicated':
        path = '/api/system/prompt-cache/materialization'
        payload = {'enabled': enabled}
    else:
        path = '/api/system/managed-tasks/' + urllib.parse.quote(TASK_KEY, safe='')
        payload = {'enabled': enabled}
    status, body, _ = request('PATCH', path, payload)
    if status != 200:
        raise RuntimeError(f'{route} control PATCH failed: status={status} body={body[:300]}')
    status, body, _ = request('GET', '/api/system/prompt-cache/materialization')
    if status != 200:
        raise RuntimeError(f'control status GET failed: status={status} body={body[:300]}')
    response = json.loads(body)
    if response.get('enabled') is not enabled:
        raise RuntimeError(f'control GET disagrees with PATCH: {response}')
    return response


def set_maintenance_control_without_legacy_sync(enabled):
    connection = open_db(MAINTENANCE_DB)
    try:
        connection.execute('BEGIN IMMEDIATE')
        now = datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='milliseconds')
        connection.execute(
            'UPDATE managed_tasks SET enabled=?,next_trigger_at=NULL,updated_at=? WHERE task_key=?',
            (int(enabled), now, TASK_KEY),
        )
        connection.execute(
            'UPDATE startup_backfill_progress SET enabled=?,next_run_after=?,suspension_reason=?, '
            'next_probe_at=NULL,wake_generation=wake_generation+1 WHERE task_name=?',
            (int(enabled), None if enabled else '2036-01-01T00:00:00.000Z',
             None if enabled else 'operator_disabled', TASK_NAME),
        )
        connection.commit()
    finally:
        connection.close()


def seed_account():
    status, body, _ = request('PUT', '/api/pool/routing-settings', {'apiKey': TOKEN})
    if status != 200:
        raise RuntimeError(f'failed to seed routing settings: status={status} body={body[:300]}')
    status, body, _ = request('POST', '/api/pool/upstream-accounts/api-keys', {
        'displayName': 'Prompt Cache Control Acceptance',
        'apiKey': ACCOUNT_KEY,
        'boundProxyKeys': [DIRECT_PROXY_KEY],
        'upstreamBaseUrl': 'http://mock-upstream:18080/',
    })
    if status != 200:
        raise RuntimeError(f'failed to seed mock account: status={status} body={body[:300]}')


def seed_history(round_index, legacy_enabled):
    control(False, 'dedicated')
    seed_account()
    connection = open_db(BUSINESS_DB, timeout=10)
    connection.execute('BEGIN IMMEDIATE')
    now = datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='milliseconds')
    rows = []
    sequence = 0
    for key_index in range(400):
        prompt_cache_key = f'acceptance-r{round_index}-key-{key_index:03d}'
        count = 1024 if key_index == 0 else 8 if key_index <= 279 else 7
        for _ in range(count):
            payload = json.dumps({
                'model': 'gpt-5',
                'promptCacheKey': prompt_cache_key,
                'upstreamAccountId': 1,
            }, separators=(',', ':'))
            rows.append((
                f'prompt-cache-control-{round_index}-{sequence}',
                now,
                'proxy',
                'success',
                payload,
                '{}',
                'full',
                'gpt-5',
                1,
                1,
                2,
            ))
            sequence += 1
    if sequence != 4096:
        raise RuntimeError(f'history fixture has unexpected invocation count: {sequence}')
    connection.executemany(
        'INSERT INTO codex_invocations '
        '(invoke_id,occurred_at,source,status,payload,raw_response,detail_level,model,input_tokens,output_tokens,total_tokens) '
        'VALUES (?,?,?,?,?,?,?,?,?,?,?)',
        rows,
    )
    columns = {
        row[1] for row in connection.execute('PRAGMA table_info(startup_backfill_progress)')
    }
    if 'enabled' not in columns:
        raise RuntimeError('business legacy startup_backfill_progress.enabled column is missing')
    changed = connection.execute(
        'UPDATE startup_backfill_progress SET enabled=? WHERE task_name=?',
        (int(legacy_enabled), TASK_NAME),
    ).rowcount
    if changed == 0:
        connection.execute(
            'INSERT INTO startup_backfill_progress (task_name,enabled) VALUES (?,?)',
            (TASK_NAME, int(legacy_enabled)),
        )
    connection.commit()
    connection.close()
    print(json.dumps({
        'phase': 'fixture-seeded',
        'round': round_index,
        'historical_keys': 400,
        'historical_invocations': sequence,
        'large_key_invocations': 1024,
        'business_legacy_enabled': legacy_enabled,
        'maintenance_enabled': False,
    }), flush=True)


def snapshot():
    business = open_db(BUSINESS_DB)
    try:
        progress = business.execute(
            'SELECT phase,total_keys,completed_keys,cursor_key FROM prompt_cache_conversation_migration_progress '
            'WHERE migration_name=?',
            (MATERIALIZATION_NAME,),
        ).fetchone()
        queue_count = business.execute(
            'SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue'
        ).fetchone()[0]
        staging = business.execute(
            'SELECT COUNT(*),COALESCE(MAX(cursor_id),0) '
            'FROM prompt_cache_conversation_stats_refresh_staging'
        ).fetchone()
        key_count = business.execute(
            'SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key LIKE ?',
            (f'acceptance-r%-key-%',),
        ).fetchone()[0]
        target = business.execute(
            'SELECT request_count FROM prompt_cache_conversations WHERE prompt_cache_key LIKE ? ORDER BY prompt_cache_key LIMIT 1',
            ('acceptance-r%-key-000',),
        ).fetchone()
        latest = business.execute(
            'SELECT status,defer_reason,scanned,updated FROM prompt_cache_conversation_materialization_runs '
            'ORDER BY id DESC LIMIT 1'
        ).fetchone()
        run_count = business.execute(
            'SELECT COUNT(*) FROM prompt_cache_conversation_materialization_runs'
        ).fetchone()[0]
        marker = business.execute(
            'SELECT EXISTS(SELECT 1 FROM schema_refresh_migrations WHERE migration_name=?)',
            (STATS_MARKER,),
        ).fetchone()[0]
        legacy_enabled = business.execute(
            'SELECT enabled FROM startup_backfill_progress WHERE task_name=?',
            (TASK_NAME,),
        ).fetchone()[0]
    finally:
        business.close()

    maintenance = open_db(MAINTENANCE_DB)
    try:
        managed_enabled = maintenance.execute(
            'SELECT enabled FROM managed_tasks WHERE task_key=?', (TASK_KEY,)
        ).fetchone()[0]
        scheduler = maintenance.execute(
            'SELECT enabled,last_status,suspension_reason,next_run_after,wake_generation '
            'FROM startup_backfill_progress WHERE task_name=?',
            (TASK_NAME,),
        ).fetchone()
        managed_run_count = maintenance.execute(
            'SELECT COUNT(*) FROM managed_task_runs WHERE task_key=?', (TASK_KEY,)
        ).fetchone()[0]
        latest_managed_run = maintenance.execute(
            'SELECT trigger_kind,status FROM managed_task_runs WHERE task_key=? ORDER BY id DESC LIMIT 1',
            (TASK_KEY,),
        ).fetchone()
    finally:
        maintenance.close()
    return {
        'phase': progress[0] if progress else None,
        'total_keys': progress[1] if progress else None,
        'completed_keys': progress[2] if progress else 0,
        'outer_cursor': progress[3] if progress else None,
        'queue_count': queue_count,
        'staging_count': staging[0],
        'staging_max_cursor': staging[1],
        'history_key_count': key_count,
        'large_key_request_count': target[0] if target else 0,
        'latest_run_status': latest[0] if latest else None,
        'latest_defer_reason': latest[1] if latest else None,
        'latest_run_scanned': latest[2] if latest else 0,
        'latest_run_updated': latest[3] if latest else 0,
        'materialization_run_count': run_count,
        'stats_marker': bool(marker),
        'business_legacy_enabled': bool(legacy_enabled),
        'maintenance_enabled': bool(managed_enabled),
        'scheduler_enabled': bool(scheduler[0]) if scheduler else None,
        'scheduler_status': scheduler[1] if scheduler else None,
        'scheduler_defer_reason': scheduler[2] if scheduler else None,
        'scheduler_next_run_after': scheduler[3] if scheduler else None,
        'scheduler_wake_generation': scheduler[4] if scheduler else None,
        'managed_task_run_count': managed_run_count,
        'latest_managed_run_trigger': latest_managed_run[0] if latest_managed_run else None,
        'latest_managed_run_status': latest_managed_run[1] if latest_managed_run else None,
    }


def terminal_row(invoke_id):
    deadline = time.perf_counter() + 10
    while time.perf_counter() < deadline:
        try:
            connection = open_db(BUSINESS_DB, timeout=0.25)
            row = connection.execute(
                'SELECT status,t_req_parse_ms,t_persist_ms FROM codex_invocations '
                'WHERE invoke_id=? ORDER BY id DESC LIMIT 1',
                (invoke_id,),
            ).fetchone()
            connection.close()
        except sqlite3.OperationalError:
            row = None
        if row and str(row[0] or '').lower() not in ('running', 'pending', 'in_flight'):
            return {
                'status': row[0],
                'request_parse_ms': row[1],
                'persist_ms': row[2],
            }
        time.sleep(0.01)
    return None


def proxy_once(sequence, prompt_cache_key):
    payload = json.dumps({
        'model': 'gpt-5',
        'input': f'prompt-cache-control-{sequence}',
        'testScenario': 'fast',
        'promptCacheKey': prompt_cache_key,
    }).encode()
    started = time.perf_counter()
    status = 'exception'
    body = ''
    invoke_id = None
    try:
        req = urllib.request.Request(
            BASE_URL + '/v1/responses',
            data=payload,
            method='POST',
            headers={
                'Authorization': f'Bearer {TOKEN}',
                'Content-Type': 'application/json',
                'Content-Length': str(len(payload)),
                'x-prompt-cache-key': prompt_cache_key,
            },
        )
        with urllib.request.urlopen(req, timeout=20) as response:
            status = response.status
            body = response.read().decode(errors='replace')[:200]
            invoke_id = response.headers.get('x-cvm-invoke-id')
    except urllib.error.HTTPError as error:
        status = error.code
        body = error.read().decode(errors='replace')[:200]
        invoke_id = error.headers.get('x-cvm-invoke-id') if error.headers else None
    except Exception as error:
        body = str(error)[:200]
    response_finished = time.perf_counter()
    terminal = terminal_row(invoke_id) if invoke_id else None
    return {
        'status': status,
        'body': body,
        'invoke_id': invoke_id,
        'response_ms': (response_finished - started) * 1000,
        'terminal_ms': (time.perf_counter() - response_finished) * 1000 if terminal else None,
        'terminal': terminal,
    }


def percentile(values, fraction):
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, math.ceil(len(ordered) * fraction) - 1))
    return round(ordered[index], 2)


def log_snapshot(phase, round_index, started_at):
    try:
        state = snapshot()
    except (sqlite3.Error, OSError, IndexError) as error:
        state = {'snapshot_error': str(error)[:200]}
    state.update({
        'phase': phase,
        'round': round_index,
        'elapsed_seconds': round(time.monotonic() - started_at, 2),
        'timestamp': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
    })
    print(json.dumps(state, ensure_ascii=False), flush=True)
    return state


def candidate_input(round_index, duration_seconds, request_rate):
    start = time.monotonic()
    deadline = start + duration_seconds
    pause_at = start + min(30, max(5, duration_seconds // 4))
    resume_at = pause_at + 5
    duplicate_at = resume_at + 1
    next_request_at = start
    sequence = 0
    pending = set()
    results = []
    paused = False
    resumed = False
    duplicate_sent = False
    run_count_at_resume = None
    first_progress = None
    first_staging_progress = None
    sampled_seconds = set()
    unexpected_disabled_after_resume = False
    first_state = log_snapshot('input-start', round_index, start)
    baseline_cursor = (
        first_state.get('phase'),
        first_state.get('outer_cursor'),
        first_state.get('completed_keys'),
        first_state.get('queue_count'),
        first_state.get('staging_count'),
        first_state.get('staging_max_cursor'),
    )
    baseline_staging_cursor = first_state.get('staging_max_cursor', 0)
    control(True, 'managed')
    enabled_state = snapshot()
    if not enabled_state['maintenance_enabled'] or enabled_state['business_legacy_enabled']:
        raise SystemExit(f'dual-database control fixture is inconsistent: {enabled_state}')

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
        while time.monotonic() < deadline:
            now = time.monotonic()
            if not paused and now >= pause_at:
                control(False, 'dedicated')
                paused = True
                print(json.dumps({'phase': 'control-pause', 'round': round_index, 'route': 'dedicated'}), flush=True)
            if paused and not resumed and now >= resume_at:
                control(True, 'managed')
                resumed = True
                run_count_at_resume = snapshot()['materialization_run_count']
                print(json.dumps({'phase': 'control-resume', 'round': round_index, 'route': 'managed'}), flush=True)
            if resumed and not duplicate_sent and now >= duplicate_at:
                control(True, 'managed')
                duplicate_sent = True
                print(json.dumps({'phase': 'control-repeat-same-value', 'round': round_index}), flush=True)
            done = [future for future in pending if future.done()]
            for future in done:
                results.append(future.result())
                pending.remove(future)
            if sequence < (duration_seconds * request_rate) and len(pending) < 2:
                if now >= next_request_at:
                    pending.add(executor.submit(
                        proxy_once,
                        f'r{round_index}-{sequence}',
                        f'acceptance-r{round_index}-key-000',
                    ))
                    sequence += 1
                    next_request_at += 1.0 / request_rate
            second = int(now - start)
            if second not in sampled_seconds:
                sampled_seconds.add(second)
                state = log_snapshot('input', round_index, start)
                cursor = (
                    state.get('phase'),
                    state.get('outer_cursor'),
                    state.get('completed_keys'),
                    state.get('queue_count'),
                    state.get('staging_count'),
                    state.get('staging_max_cursor'),
                )
                if first_progress is None and cursor != baseline_cursor and state.get('snapshot_error') is None:
                    first_progress = second
                if (
                    first_staging_progress is None
                    and state.get('staging_max_cursor', 0) > baseline_staging_cursor
                ):
                    first_staging_progress = second
                if (
                    resumed
                    and state.get('maintenance_enabled') is True
                    and state.get('materialization_run_count', 0) > (run_count_at_resume or 0)
                    and state.get('latest_defer_reason') == 'operator_disabled'
                ):
                    unexpected_disabled_after_resume = True
            time.sleep(0.01)
        for future in concurrent.futures.as_completed(pending, timeout=20):
            results.append(future.result())

    bad = [item for item in results if item['status'] != 200 or item['terminal'] is None]
    parse_values = [
        item['terminal']['request_parse_ms'] for item in results
        if item['terminal'] and isinstance(item['terminal']['request_parse_ms'], (int, float))
    ]
    persist_values = [
        item['terminal']['persist_ms'] for item in results
        if item['terminal'] and isinstance(item['terminal']['persist_ms'], (int, float))
    ]
    confirm_values = [
        item['terminal_ms'] for item in results
        if isinstance(item['terminal_ms'], (int, float))
    ]
    response_values = [
        item['response_ms'] for item in results
        if isinstance(item['response_ms'], (int, float))
    ]
    gate_failures = []
    if bad:
        gate_failures.append('request_or_terminal_failure')
    if sequence != duration_seconds * request_rate:
        gate_failures.append('submitted_request_count')
    if first_progress is None or first_progress > 30:
        gate_failures.append('first_durable_progress')
    if first_staging_progress is None or first_staging_progress > 30:
        gate_failures.append('first_staging_progress')
    if percentile(parse_values, 0.99) is None or percentile(parse_values, 0.99) > 100:
        gate_failures.append('request_parse_and_id_allocation_p99')
    if percentile(confirm_values, 0.99) is None or percentile(confirm_values, 0.99) > 1000:
        gate_failures.append('terminal_confirm_p99')
    if not paused or not resumed or not duplicate_sent:
        gate_failures.append('control_pause_resume')
    if unexpected_disabled_after_resume:
        gate_failures.append('unexpected_operator_disabled')
    summary = {
        'phase': 'input-summary',
        'round': round_index,
        'submitted': sequence,
        'achieved_request_rate_per_second': round(sequence / duration_seconds, 3),
        'completed': len(results),
        'bad_count': len(bad),
        'bad_examples': bad[:3],
        'online_workload_passed': not gate_failures,
        'online_gate_failures': gate_failures,
        'response_p99_ms': percentile(response_values, 0.99),
        'response_max_ms': round(max(response_values), 2) if response_values else None,
        'request_parse_and_id_allocation_p99_ms': percentile(parse_values, 0.99),
        'request_parse_and_id_allocation_max_ms': round(max(parse_values), 2) if parse_values else None,
        'terminal_persist_p99_ms': percentile(persist_values, 0.99),
        'terminal_persist_max_ms': round(max(persist_values), 2) if persist_values else None,
        'terminal_confirm_p99_ms': percentile(confirm_values, 0.99),
        'terminal_confirm_max_ms': round(max(confirm_values), 2) if confirm_values else None,
        'first_durable_progress_seconds': first_progress,
        'first_staging_progress_seconds': first_staging_progress,
        'pause_route': 'dedicated',
        'resume_route': 'managed-task',
        'same_value_control_repeated': duplicate_sent,
        'run_count_at_resume': run_count_at_resume,
        'unexpected_operator_disabled_after_resume': unexpected_disabled_after_resume,
    }
    print(json.dumps(summary, ensure_ascii=False), flush=True)
    with open(os.path.join(DATA_DIR, f'prompt-cache-control-r{round_index}.json'), 'w', encoding='utf-8') as output:
        json.dump(summary, output)


def baseline_probe(round_index, duration_seconds):
    start = time.monotonic()
    before_enable = log_snapshot('baseline-before-enable', round_index, start)
    managed_runs_before = before_enable.get('managed_task_run_count', 0)
    set_maintenance_control_without_legacy_sync(True)
    status, body, _ = request(
        'POST',
        '/api/system/managed-tasks/' + urllib.parse.quote(TASK_KEY, safe='') + '/run',
    )
    if status != 200:
        raise SystemExit(f'baseline managed run request failed: status={status} body={body[:300]}')
    start = time.monotonic()
    deadline = start + duration_seconds
    reproduced = None
    reproduction_kind = None
    while time.monotonic() < deadline:
        state = log_snapshot('baseline-probe', round_index, start)
        if (
            state.get('maintenance_enabled') is True
            and state.get('latest_defer_reason') == 'operator_disabled'
        ):
            reproduced = state
            reproduction_kind = 'operator_disabled_misclassification'
            break
        time.sleep(1)
    if reproduced is None:
        state = snapshot()
        stalled = (
            state.get('maintenance_enabled') is True
            and state.get('business_legacy_enabled') is False
            and state.get('managed_task_run_count', 0) > managed_runs_before
            and state.get('history_key_count') == 0
            and state.get('staging_count') == 0
            and state.get('materialization_run_count') == 0
            and state.get('queue_count', 0) > 0
        )
        if stalled:
            reproduced = state
            reproduction_kind = 'no_progress_with_conflicting_business_enablement'
    summary = {
        'phase': 'baseline-reproduction',
        'round': round_index,
        'reproduced': reproduced is not None,
        'reproduction_kind': reproduction_kind,
        'trigger': 'maintenance control is enabled, legacy business control stays disabled, and the old service either reports operator_disabled or makes no durable progress',
        'checkpoint': reproduced,
    }
    print(json.dumps(summary, ensure_ascii=False), flush=True)
    if reproduced is None:
        raise SystemExit('baseline did not reproduce the prompt-cache page/disable misclassification')


def maintenance_lock_probe(round_index):
    control(True, 'managed')
    lock_connection = open_db(MAINTENANCE_DB, timeout=10)
    lock_connection.execute('BEGIN IMMEDIATE')
    lock_started = time.monotonic()
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
            patch_future = executor.submit(request, 'PATCH', '/api/system/prompt-cache/materialization', {'enabled': False})
            time.sleep(0.1)
            proxy_future = executor.submit(
                proxy_once,
                f'lock-probe-r{round_index}',
                f'lock-probe-r{round_index}',
            )
            proxy_result = proxy_future.result(timeout=3)
            patch_result = patch_future.result(timeout=4)
            lock_duration = time.monotonic() - lock_started
    finally:
        lock_connection.rollback()
        lock_connection.close()

    status, body = patch_result[:2]
    status_after_release, body_after_release, _ = request('GET', '/api/system/prompt-cache/materialization')
    status_ok = (
        status == 503
        and bool(body.strip())
        and status_after_release == 200
        and json.loads(body_after_release).get('enabled') is True
        and lock_duration >= 0.1
    )
    proxy_ok = (
        proxy_result['status'] == 200
        and proxy_result['terminal'] is not None
        and proxy_result['response_ms'] <= 1000
        and str(proxy_result['terminal']['status']).lower() not in ('running', 'pending', 'in_flight')
    )
    summary = {
        'phase': 'maintenance-file-lock-probe',
        'round': round_index,
        'maintenance_write_lock_seconds': round(lock_duration, 2),
        'control_patch_status': status,
        'control_patch_body': body[:200],
        'status_after_release': status_after_release,
        'enabled_after_release': json.loads(body_after_release).get('enabled') if status_after_release == 200 else None,
        'proxy_status': proxy_result['status'],
        'proxy_response_ms': round(proxy_result['response_ms'], 2),
        'proxy_terminal': proxy_result['terminal'],
        'control_unavailable_is_explicit': status_ok,
        'business_path_progressed': proxy_ok,
    }
    print(json.dumps(summary, ensure_ascii=False), flush=True)
    if not status_ok or not proxy_ok:
        raise SystemExit(f'maintenance file-lock isolation failed in round {round_index}')


def candidate_observe(round_index, duration_seconds):
    input_path = os.path.join(DATA_DIR, f'prompt-cache-control-r{round_index}.json')
    with open(input_path, encoding='utf-8') as source:
        input_summary = json.load(source)
    start = time.monotonic()
    deadline = start + duration_seconds
    first_progress = None
    complete_at = None
    completed_run_count = None
    no_work_run_count_changed = False
    unexpected_disabled = False
    states = []
    initial = log_snapshot('observe-start', round_index, start)
    start_cursor = (
        initial.get('phase'),
        initial.get('outer_cursor'),
        initial.get('completed_keys'),
        initial.get('queue_count'),
        initial.get('staging_count'),
        initial.get('staging_max_cursor'),
    )
    if (
        initial.get('phase') == 'complete'
        and initial.get('queue_count') == 0
        and initial.get('staging_count') == 0
        and initial.get('stats_marker') is True
    ):
        complete_at = 0
        completed_run_count = initial.get('materialization_run_count')
    observed_seconds = set()
    while time.monotonic() < deadline:
        elapsed = int(time.monotonic() - start)
        if elapsed not in observed_seconds:
            observed_seconds.add(elapsed)
            state = log_snapshot('observe', round_index, start)
            states.append(state)
            current_cursor = (
                state.get('phase'),
                state.get('outer_cursor'),
                state.get('completed_keys'),
                state.get('queue_count'),
                state.get('staging_count'),
                state.get('staging_max_cursor'),
            )
            if first_progress is None and current_cursor != start_cursor:
                first_progress = elapsed
            if (
                state.get('maintenance_enabled') is True
                and state.get('materialization_run_count', 0) > input_summary.get('run_count_at_resume', 0)
                and state.get('latest_defer_reason') == 'operator_disabled'
            ):
                unexpected_disabled = True
            if (
                state.get('phase') == 'complete'
                and state.get('queue_count') == 0
                and state.get('staging_count') == 0
                and state.get('stats_marker') is True
            ):
                if complete_at is None:
                    complete_at = elapsed
                    completed_run_count = state.get('materialization_run_count')
                elif elapsed - complete_at >= 30:
                    no_work_run_count_changed = state.get('materialization_run_count') != completed_run_count
        time.sleep(0.5)

    final = snapshot()
    status, body, _ = request('GET', '/api/system/prompt-cache/materialization')
    api_status = json.loads(body) if status == 200 else {}
    expected_target_count = 1024 + input_summary['completed']
    target_count_ok = final['large_key_request_count'] >= expected_target_count
    all_keys_ok = final['history_key_count'] == 400
    summary = {
        'phase': 'candidate-completion',
        'round': round_index,
        'observation_seconds': duration_seconds,
        'online_workload_passed': input_summary.get('online_workload_passed') is True,
        'online_gate_failures': input_summary.get('online_gate_failures', []),
        'complete_within_budget': complete_at is not None and complete_at <= duration_seconds,
        'complete_at_seconds': complete_at,
        'first_durable_progress_seconds': input_summary['first_durable_progress_seconds'],
        'first_staging_progress_seconds': input_summary['first_staging_progress_seconds'],
        'post_restart_progress_seconds': first_progress,
        'all_historical_keys': all_keys_ok,
        'history_keys': final['history_key_count'],
        'large_key_request_count': final['large_key_request_count'],
        'expected_large_key_request_count': expected_target_count,
        'large_key_count_ok': target_count_ok,
        'queue_count': final['queue_count'],
        'staging_count': final['staging_count'],
        'phase_final': final['phase'],
        'stats_marker': final['stats_marker'],
        'maintenance_enabled': final['maintenance_enabled'],
        'business_legacy_enabled': final['business_legacy_enabled'],
        'api_status': status,
        'api_enabled': api_status.get('enabled'),
        'unexpected_operator_disabled': unexpected_disabled,
        'no_work_run_count_changed': no_work_run_count_changed,
        'input_summary': input_summary,
        'defer_reasons': sorted({state.get('latest_defer_reason') for state in states if state.get('latest_defer_reason')}),
        'coordinator_priority_observed': any(state.get('latest_defer_reason') == 'coordinator_priority' for state in states),
    }
    print(json.dumps(summary, ensure_ascii=False), flush=True)
    if (
        not summary['online_workload_passed']
        or not summary['complete_within_budget']
        or input_summary['first_durable_progress_seconds'] is None
        or input_summary['first_durable_progress_seconds'] > 30
        or input_summary['first_staging_progress_seconds'] is None
        or input_summary['first_staging_progress_seconds'] > 30
        or not all_keys_ok
        or not target_count_ok
        or final['queue_count'] != 0
        or final['staging_count'] != 0
        or final['phase'] != 'complete'
        or not final['stats_marker']
        or not final['maintenance_enabled']
        or final['business_legacy_enabled']
        or status != 200
        or api_status.get('enabled') is not True
        or unexpected_disabled
        or no_work_run_count_changed
    ):
        raise SystemExit(f'candidate completion workload failed in round {round_index}')


def wait_health():
    deadline = time.time() + 120
    last_error = None
    while time.time() < deadline:
        try:
            status, body, _ = request('GET', '/health', timeout=5)
            if status == 200 and body == 'ok':
                print(json.dumps({'phase': 'health', 'status': 'ok'}), flush=True)
                return
            last_error = f'status={status} body={body[:200]}'
        except Exception as error:
            last_error = str(error)
        time.sleep(1)
    raise SystemExit(f'health check timed out: {last_error}')


COMMANDS = {
    'seed-history': lambda: seed_history(int(sys.argv[2]), sys.argv[3] == '1'),
    'baseline-probe': lambda: baseline_probe(int(sys.argv[2]), int(sys.argv[3])),
    'maintenance-lock-probe': lambda: maintenance_lock_probe(int(sys.argv[2])),
    'candidate-input': lambda: candidate_input(int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])),
    'candidate-observe': lambda: candidate_observe(int(sys.argv[2]), int(sys.argv[3])),
    'wait-health': wait_health,
}

if len(sys.argv) < 2 or sys.argv[1] not in COMMANDS:
    raise SystemExit(f'usage: {sys.argv[0]} <{",".join(sorted(COMMANDS))}> [args]')
COMMANDS[sys.argv[1]]()
