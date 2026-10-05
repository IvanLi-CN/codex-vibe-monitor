#!/usr/bin/env python3
"""Business proxy allocation/terminal acceptance without legacy telemetry."""
import base64

import http.client

import json

import math

import os

import queue

import socket

import sqlite3

import subprocess
import sys

import threading

import time

from concurrent.futures import ThreadPoolExecutor

import urllib.error

import urllib.parse

import urllib.request

BASE_URL = 'http://app:8080'

TOKEN = 'pool-shared-testbox-key'

ACCOUNT_KEY = 'upstream-primary-key'

DIRECT_PROXY_KEY = '__direct__'

DATA_DIR = '/work/data'

CAPTURE_P1_ACK = os.environ.get('CAPTURE_P1_ACK') == '1'

def request(method, path, payload=None, headers=None, timeout=30):
    data = None if payload is None else json.dumps(payload).encode()
    request_headers = {'Authorization': f'Bearer {TOKEN}'}
    if payload is not None:
        request_headers.update({'Content-Type': 'application/json', 'Content-Length': str(len(data))})
    request_headers.update(headers or {})
    req = urllib.request.Request(BASE_URL + path, data=data, method=method, headers=request_headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            body = response.read().decode(errors='replace')
            return response.status, body
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode(errors='replace')


def wait_health():
    deadline = time.time() + 120
    last_error = None
    while time.time() < deadline:
        try:
            status, body = request('GET', '/health', timeout=5)
            if status == 200 and body == 'ok':
                print(json.dumps({'phase': 'wait-health', 'status': 'ok'}), flush=True)
                return
            last_error = f'status={status} body={body[:200]}'
        except Exception as exc:
            last_error = str(exc)
        time.sleep(1)
    raise SystemExit(f'healthcheck timed out: {last_error}')


def seed():
    status, body = request('PUT', '/api/pool/routing-settings', {'apiKey': TOKEN})
    if status != 200:
        raise SystemExit(f'failed to seed pool routing settings: status={status} body={body[:500]}')
    status, body = request('POST', '/api/pool/upstream-accounts/api-keys', {
        'displayName': 'Shared Performance Primary',
        'apiKey': ACCOUNT_KEY,
        'boundProxyKeys': [DIRECT_PROXY_KEY],
        'upstreamBaseUrl': 'http://mock-upstream:18080/',
    })
    if status != 200:
        raise SystemExit(f'failed to create api-key account: status={status} body={body[:500]}')
    print(json.dumps({'phase': 'seed', 'status': 'ok'}), flush=True)


def percentile(values, fraction):
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, math.ceil(len(ordered) * fraction) - 1))
    return round(ordered[index], 2)


def long_wait_terminal_row(invoke_id):
    try:
        connection = sqlite3.connect(os.path.join(DATA_DIR, 'codex_vibe_monitor.db'), timeout=0.25)
        connection.execute('PRAGMA busy_timeout=250')
        row = connection.execute(
            'SELECT status, t_req_parse_ms, t_persist_ms, t_total_ms, t_upstream_ttfb_ms '
            'FROM codex_invocations WHERE invoke_id = ? ORDER BY id DESC LIMIT 1',
            (invoke_id,),
        ).fetchone()
        connection.close()
        return row
    except sqlite3.OperationalError:
        return None


def long_wait_terminal(invoke_id, response_finished):
    deadline = time.perf_counter() + 10.0
    while time.perf_counter() < deadline:
        row = long_wait_terminal_row(invoke_id)
        if row and str(row[0] or '').lower() not in ('running', 'pending', 'in_flight'):
            return {
                'status': row[0],
                'request_parse_ms': row[1],
                'persist_ms': row[2],
                'total_ms': row[3],
                'upstream_ttfb_ms': row[4],
                'confirm_ms': (time.perf_counter() - response_finished) * 1000,
            }
        time.sleep(0.01)
    return None


def cancel_attempt_by_key(prompt_cache_key):
    try:
        connection = sqlite3.connect(os.path.join(DATA_DIR, 'codex_vibe_monitor.db'), timeout=0.25)
        connection.execute('PRAGMA busy_timeout=250')
        row = connection.execute(
            'SELECT invoke_id, status, phase, failure_kind, error_message, downstream_error_message '
            'FROM pool_upstream_request_attempts WHERE sticky_key = ? ORDER BY id DESC LIMIT 1',
            (prompt_cache_key,),
        ).fetchone()
        connection.close()
        return row
    except sqlite3.OperationalError:
        return None


def cancel_attempt_by_id(invoke_id):
    try:
        connection = sqlite3.connect(os.path.join(DATA_DIR, 'codex_vibe_monitor.db'), timeout=0.25)
        connection.execute('PRAGMA busy_timeout=250')
        row = connection.execute(
            'SELECT status, phase, failure_kind, error_message, downstream_error_message '
            'FROM pool_upstream_request_attempts WHERE invoke_id = ? ORDER BY id DESC LIMIT 1',
            (invoke_id,),
        ).fetchone()
        connection.close()
        return row
    except sqlite3.OperationalError:
        return None


def cancel_invocation_by_id(invoke_id):
    try:
        connection = sqlite3.connect(os.path.join(DATA_DIR, 'codex_vibe_monitor.db'), timeout=0.25)
        connection.execute('PRAGMA busy_timeout=250')
        row = connection.execute(
            'SELECT status, failure_kind, failure_class, error_message '
            'FROM codex_invocations WHERE invoke_id = ? ORDER BY id DESC LIMIT 1',
            (invoke_id,),
        ).fetchone()
        connection.close()
        return row
    except sqlite3.OperationalError:
        return None


def wait_for_cancel_invocation(prompt_cache_key, timeout_seconds):
    deadline = time.perf_counter() + timeout_seconds
    while time.perf_counter() < deadline:
        row = cancel_attempt_by_key(prompt_cache_key)
        if row:
            return row
        time.sleep(0.01)
    return None


def wait_for_cancel_terminal(invoke_id, timeout_seconds):
    deadline = time.perf_counter() + timeout_seconds
    while time.perf_counter() < deadline:
        attempt = cancel_attempt_by_id(invoke_id)
        if attempt and (
            str(attempt[0] or '').lower() not in ('running', 'pending', 'in_flight')
            and str(attempt[1] or '').lower()
            not in (
                'running',
                'pending',
                'started',
                'in_flight',
                'active',
                'waiting',
                'streaming_response',
                'response_headers',
                'body_streaming',
            )
        ):
            invocation = cancel_invocation_by_id(invoke_id)
            invocation_status = str(invocation[0] or '').lower() if invocation else ''
            invocation_error = str(invocation[3] or '') if invocation else ''
            invocation_has_terminal_marker = invocation and (
                invocation[1] == 'downstream_closed'
                or invocation[2] == 'client_abort'
                or 'downstream_closed' in invocation_error
            )
            if (
                invocation
                and invocation_status not in ('', 'running', 'pending', 'in_flight')
                and invocation_has_terminal_marker
            ):
                return {
                    'attempt_status': attempt[0],
                    'attempt_phase': attempt[1],
                    'attempt_failure_kind': attempt[2],
                    'attempt_error_message': attempt[3],
                    'attempt_downstream_error_message': attempt[4],
                    'invocation_status': invocation[0],
                    'invocation_failure_kind': invocation[1],
                    'invocation_failure_class': invocation[2],
                    'invocation_error_message': invocation[3],
                }
        time.sleep(0.01)
    return None


def long_wait_proxy_once(spec, timeout_seconds):
    payload = {
        'model': 'gpt-5',
        'input': f"long-wait-{spec['round']}-{spec['sequence']}",
        'testScenario': spec['scenario'],
    }
    request_headers = {
        'Authorization': f'Bearer {TOKEN}',
        'Content-Type': 'application/json',
    }
    prompt_cache_key = spec.get('prompt_cache_key')
    if prompt_cache_key:
        payload['promptCacheKey'] = prompt_cache_key
        request_headers['x-prompt-cache-key'] = prompt_cache_key
    encoded = json.dumps(payload).encode()
    request_headers['Content-Length'] = str(len(encoded))
    started = time.perf_counter()
    status = 'exception'
    body = ''
    invoke_id = None
    response_finished = None
    try:
        req = urllib.request.Request(
            BASE_URL + '/v1/responses', data=encoded, method='POST', headers=request_headers
        )
        with urllib.request.urlopen(req, timeout=timeout_seconds) as response:
            body = response.read().decode(errors='replace')[:200]
            status = response.status
            invoke_id = response.headers.get('x-cvm-invoke-id')
            response_finished = time.perf_counter()
    except urllib.error.HTTPError as error:
        status = error.code
        body = error.read().decode(errors='replace')[:200]
        invoke_id = error.headers.get('x-cvm-invoke-id') if error.headers else None
        response_finished = time.perf_counter()
    except Exception as exc:
        body = str(exc)[:200]
        response_finished = time.perf_counter()
    terminal = (
        long_wait_terminal(invoke_id, response_finished)
        if invoke_id and spec.get('track_terminal', True)
        else None
    )
    return {
        'sequence': spec['sequence'],
        'scenario': spec['scenario'],
        'key_mode': spec['key_mode'],
        'status': status,
        'body': body,
        'invoke_id': invoke_id,
        'response_finished': response_finished,
        'response_ms': (response_finished - started) * 1000,
        'terminal': terminal,
    }


def long_wait_cancel(round_index):
    prompt_cache_key = f'cancel-{round_index}'
    payload = json.dumps({
        'model': 'gpt-5',
        'input': f'long-wait-cancel-{round_index}',
        'testScenario': 'cancel',
        'promptCacheKey': prompt_cache_key,
    }).encode()
    headers = (
        f'POST /v1/responses HTTP/1.1\r\n'
        f'Host: app:8080\r\n'
        f'Authorization: Bearer {TOKEN}\r\n'
        'Content-Type: application/json\r\n'
        f'Content-Length: {len(payload)}\r\n\r\n'
    ).encode()
    started = time.perf_counter()
    sock = None
    try:
        sock = socket.create_connection(('app', 8080), timeout=5)
        sock.sendall(headers + payload)
        accepted = wait_for_cancel_invocation(prompt_cache_key, 5)
        if accepted is None:
            sock.close()
            return {
                'status': 'client_closed',
                'duration_ms': (time.perf_counter() - started) * 1000,
                'server_accepted': False,
                'invoke_id': None,
                'terminal': None,
            }
        close_started = time.perf_counter()
        time.sleep(1.0)
        sock.close()
        close_duration_ms = (time.perf_counter() - close_started) * 1000
        terminal = wait_for_cancel_terminal(accepted[0], 15)
        return {
            'status': 'client_closed',
            'duration_ms': close_duration_ms,
            'server_accepted': True,
            'invoke_id': accepted[0],
            'terminal': terminal,
        }
    except Exception as exc:
        if sock is not None:
            sock.close()
        return {
            'status': 'exception',
            'error': str(exc),
            'duration_ms': (time.perf_counter() - started) * 1000,
            'server_accepted': False,
            'invoke_id': None,
            'terminal': None,
        }


def run_long_wait_round(round_index, duration_seconds, request_rate):
    slow_modes = [
        ('slow-success', 'same', 'slow-success-shared'),
        ('slow-success', 'same', 'slow-success-shared'),
        ('slow-success', 'different', f'slow-success-{round_index}-different'),
        ('slow-success', 'none', None),
        ('slow-timeout', 'same', 'slow-timeout-shared'),
        ('slow-timeout', 'same', 'slow-timeout-shared'),
        ('slow-timeout', 'different', f'slow-timeout-{round_index}-different'),
        ('slow-timeout', 'none', None),
    ]
    slow_specs = [
        {
            'round': round_index,
            'sequence': sequence,
            'scenario': scenario,
            'key_mode': key_mode,
            'prompt_cache_key': prompt_cache_key,
        }
        for sequence, (scenario, key_mode, prompt_cache_key) in enumerate(slow_modes)
    ]
    # Measure long upstream waits on admitted identities. Fresh-key pressure
    # remains in every third fast request below; a permitted cold refusal must
    # not prevent the long-wait fixture from reaching the mock upstream.
    warmed_keys = set()
    warmup_refusals = 0
    for spec in slow_specs:
        key = spec['prompt_cache_key']
        if key in warmed_keys:
            continue
        for attempt in range(1, 6):
            result = long_wait_proxy_once({
                **spec,
                'scenario': 'fast',
                'key_mode': 'different' if key is not None else 'none',
            }, 15)
            if result['status'] == 200 and result['terminal'] is not None:
                warmed_keys.add(key)
                break
            if (
                result['status'] == 503
                and result['invoke_id'] is None
                and result['body'] == '{"error":"failed to allocate proxy invoke id: invocation range allocation timed out after 100ms"}'
                and attempt < 5
            ):
                warmup_refusals += 1
                continue
            raise SystemExit(f'long-wait fixture admission failed: {result}')
    print(json.dumps({
        'phase': 'long-wait-fixture-ready',
        'round': round_index,
        'owners': len(warmed_keys),
        'cold_refusals': warmup_refusals,
    }), flush=True)
    started_at = time.perf_counter()
    slow_pool = ThreadPoolExecutor(max_workers=8)
    slow_futures = [slow_pool.submit(long_wait_proxy_once, spec, 195) for spec in slow_specs]
    cancel_result = {}
    cancel_thread = threading.Thread(
        target=lambda: cancel_result.update(long_wait_cancel(round_index)), daemon=True
    )
    cancel_thread.start()

    fast_futures = []
    with ThreadPoolExecutor(max_workers=2) as fast_pool:
        next_request_at = started_at
        sequence = 0
        deadline = started_at + duration_seconds
        while time.perf_counter() < deadline:
            key_mode = ('same', 'different', 'none')[sequence % 3]
            prompt_cache_key = (
                'fast-shared'
                if key_mode == 'same'
                else f'fast-{round_index}-{sequence}'
                if key_mode == 'different'
                else None
            )
            fast_futures.append(fast_pool.submit(long_wait_proxy_once, {
                'round': round_index,
                'sequence': 1000 + sequence,
                'scenario': 'fast',
                'key_mode': key_mode,
                'prompt_cache_key': prompt_cache_key,
            }, 15))
            sequence += 1
            next_request_at += 1.0 / request_rate
            delay = next_request_at - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
    fast_results = [future.result() for future in fast_futures]
    slow_results = [future.result() for future in slow_futures]
    slow_pool.shutdown(wait=True)
    cancel_thread.join(timeout=10)

    # C3 permits a cold owner's empty-range/admission wait to fail closed at
    # 100 ms. Keep these visible and include their full response latency in
    # the allocation percentile; cached owners must still succeed.
    cold_rejections = [
        item for item in fast_results
        if item['key_mode'] == 'different'
        and item['status'] == 503
        and item['invoke_id'] is None
        and item['body'] in (
            '{"error":"failed to allocate proxy invoke id: invocation range allocation timed out after 100ms"}',
            '{"error":"failed to allocate proxy invoke id: invocation cache admission timed out after 100ms"}',
        )
    ]
    cold_rejected_sequences = {item['sequence'] for item in cold_rejections}
    fast_bad = [
        item for item in fast_results
        if item['status'] != 200 or item['terminal'] is None
        if item['sequence'] not in cold_rejected_sequences
    ]
    fast_parse_ms = [
        item['terminal']['request_parse_ms']
        for item in fast_results
        if item['terminal'] and isinstance(item['terminal']['request_parse_ms'], (int, float))
    ]
    fast_parse_ms.extend(item['response_ms'] for item in cold_rejections)
    fast_confirm_ms = [
        item['terminal']['confirm_ms']
        for item in fast_results
        if item['terminal'] and isinstance(item['terminal']['confirm_ms'], (int, float))
    ]
    slow_success = [item for item in slow_results if item['scenario'] == 'slow-success']
    slow_timeout = [item for item in slow_results if item['scenario'] == 'slow-timeout']
    cancel_terminal = cancel_result.get('terminal') or {}
    cancel_ok = (
        cancel_result.get('status') == 'client_closed'
        and isinstance(cancel_result.get('duration_ms'), (int, float))
        and cancel_result['duration_ms'] <= 5000
        and cancel_result.get('server_accepted') is True
        and str(cancel_terminal.get('attempt_status') or '').lower()
        not in ('', 'running', 'pending', 'in_flight')
        and str(cancel_terminal.get('attempt_phase') or '').lower()
        not in (
            '',
            'running',
            'pending',
            'started',
            'in_flight',
            'active',
            'waiting',
            'streaming_response',
            'response_headers',
            'body_streaming',
        )
        and str(cancel_terminal.get('invocation_status') or '').lower()
        not in ('', 'running', 'pending', 'in_flight')
        and (
            cancel_terminal.get('invocation_failure_kind') == 'downstream_closed'
            or cancel_terminal.get('invocation_failure_class') == 'client_abort'
            or 'downstream_closed' in str(cancel_terminal.get('invocation_error_message') or '')
        )
        and (
            cancel_terminal.get('attempt_failure_kind') == 'downstream_closed'
            or cancel_terminal.get('invocation_failure_kind') == 'downstream_closed'
            or cancel_terminal.get('invocation_failure_class') == 'client_abort'
            or 'downstream_closed' in str(cancel_terminal.get('attempt_downstream_error_message') or '')
        )
    )
    success_ok = all(
        item['status'] == 200
        and item['terminal'] is not None
        and 170000 <= item['response_ms'] <= 185000
        for item in slow_success
    )
    timeout_ok = all(
        item['status'] != 200
        and item['terminal'] is not None
        and 175000 <= item['response_ms'] <= 195000
        for item in slow_timeout
    )
    summary = {
        'phase': 'long-wait-round',
        'round': round_index,
        'duration_seconds': duration_seconds,
        'request_rate': request_rate,
        'fast_submitted': len(fast_results),
        'fast_bad_count': len(fast_bad),
        'fast_bad_examples': fast_bad[:3],
        'cold_rejection_count': len(cold_rejections),
        'cold_rejection_examples': cold_rejections[:3],
        'allocation_parse_p99_ms': percentile(fast_parse_ms, 0.99),
        'terminal_confirm_p99_ms': percentile(fast_confirm_ms, 0.99),
        'slow_success_ok': success_ok,
        'slow_timeout_ok': timeout_ok,
        'cancel_ok': cancel_ok,
        'slow_results': slow_results,
        'cancel_result': cancel_result,
    }
    print(json.dumps(summary, ensure_ascii=False), flush=True)
    if (
        fast_bad
        or not fast_parse_ms
        or not fast_confirm_ms
        or not success_ok
        or not timeout_ok
        or not cancel_ok
        or summary['allocation_parse_p99_ms'] > 100
        or summary['terminal_confirm_p99_ms'] > 1000
    ):
        raise SystemExit(f'long-wait acceptance failed in round {round_index}')


def run_long_wait(duration_seconds, rounds, request_rate):
    seed()
    subprocess.run(['python', '/work/invocation-range-acceptance.py', 'install'], check=True)
    subprocess.run(['python', '/work/invocation-range-acceptance.py', 'probe'], check=True)
    for round_index in range(1, rounds + 1):
        run_long_wait_round(round_index, duration_seconds, request_rate)
    print(json.dumps({
        'phase': 'long-wait-complete',
        'rounds': rounds,
        'duration_seconds': duration_seconds,
        'request_rate': request_rate,
    }), flush=True)


COMMANDS = {'wait-health': wait_health, 'seed': seed, 'long-wait': lambda: run_long_wait(int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]))}
if len(sys.argv) < 2 or sys.argv[1] not in COMMANDS:
    raise SystemExit('expected wait-health, seed or long-wait command')
COMMANDS[sys.argv[1]]()
