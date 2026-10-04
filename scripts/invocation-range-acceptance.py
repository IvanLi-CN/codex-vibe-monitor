#!/usr/bin/env python3
"""Observe committed range writes in isolated Linux service acceptance databases."""

import importlib.util
import json
import os
import sqlite3
import sys
import time
from pathlib import Path

module_spec = importlib.util.spec_from_file_location(
    'control_workload', Path(__file__).with_name('prompt-cache-control-loadgen.py'))
base = importlib.util.module_from_spec(module_spec)
module_spec.loader.exec_module(base)

ALPHABET = 'ABCDEFGHJKMNPQRSTUVWXYZ23456789'
KEYS = ['range-acceptance-a', 'range-acceptance-b']
STATE = os.path.join(base.DATA_DIR, 'range-acceptance-recovery.json')


def emit(phase, **fields):
    print(json.dumps({'phase': phase, **fields}), flush=True)


def sequence(invoke_id):
    if len(invoke_id) != 10:
        raise RuntimeError(f'invalid compact invocation identity: {invoke_id}')
    result = 0
    for character in invoke_id[6:]:
        result = result * 31 + ALPHABET.index(character)
    return result


def call(key):
    payload = {'model': 'gpt-5', 'input': 'range acceptance', 'testScenario': 'fast'}
    if key:
        payload['promptCacheKey'] = key
    status, body, headers = base.request('POST', '/v1/responses', payload)
    if status != 200:
        raise RuntimeError(f'range probe failed: status={status}, body={body[:200]}')
    invoke_id = headers.get('x-cvm-invoke-id')
    sequence(invoke_id or '')
    return invoke_id


def install():
    with base.open_db(base.BUSINESS_DB, timeout=10) as db:
        db.executescript('''
CREATE TABLE IF NOT EXISTS acceptance_range_updates (
 id INTEGER PRIMARY KEY, owner_type TEXT NOT NULL, prefix TEXT NOT NULL,
 old_ceiling INTEGER NOT NULL, new_ceiling INTEGER NOT NULL);
CREATE TRIGGER IF NOT EXISTS acceptance_conversation_range_update
AFTER UPDATE OF last_invoke_sequence ON prompt_cache_conversations
WHEN OLD.last_invoke_sequence != NEW.last_invoke_sequence
BEGIN INSERT INTO acceptance_range_updates(owner_type,prefix,old_ceiling,new_ceiling)
 VALUES('conversation',NEW.conversation_id,OLD.last_invoke_sequence,NEW.last_invoke_sequence); END;
CREATE TRIGGER IF NOT EXISTS acceptance_hour_range_update
AFTER UPDATE OF last_invoke_sequence ON hourly_invoke_prefixes
WHEN OLD.last_invoke_sequence != NEW.last_invoke_sequence
BEGIN INSERT INTO acceptance_range_updates(owner_type,prefix,old_ceiling,new_ceiling)
 VALUES('hour',NEW.prefix,OLD.last_invoke_sequence,NEW.last_invoke_sequence); END;
''')
    emit('range-observation-installed')


def probe():
    # All three owners have 44 remaining before the trigger. They must join
    # one refill, rather than reserve a sequence for each successful call.
    prefixes = []
    for key in [*KEYS, None]:
        identities = [call(key) for _ in range(20)]
        prefix = identities[0][:6]
        if any(identity[:6] != prefix for identity in identities):
            raise RuntimeError('owner changed during the controlled range probe')
        prefixes.append(prefix)
    for _ in range(13):
        call(KEYS[0])
    deadline = time.monotonic() + 3
    while True:
        with base.open_db(base.BUSINESS_DB) as db:
            rows = db.execute(
                'SELECT prefix,COUNT(*),MAX(new_ceiling) FROM acceptance_range_updates '
                'WHERE prefix IN (?,?,?) GROUP BY prefix', prefixes,
            ).fetchall()
        if len(rows) == 3 and all(count == 2 and ceiling == 127 for _, count, ceiling in rows):
            break
        if time.monotonic() >= deadline:
            raise RuntimeError(f'controlled owners did not share a refill: {rows}')
        time.sleep(0.02)
    emit('range-controlled-probe', requests=73, owners=3, ceiling_updates=6,
         expected_shared_refill_owners=3, prefixes=prefixes)


def snapshot():
    with base.open_db(base.BUSINESS_DB, timeout=10) as db:
        rows = db.execute(
            'SELECT prompt_cache_key,conversation_id,last_invoke_sequence '
            'FROM prompt_cache_conversations WHERE prompt_cache_key IN (?,?)', KEYS,
        ).fetchall()
        hour = int(time.time()) // 3600
        hourly = db.execute(
            'SELECT prefix,last_invoke_sequence FROM hourly_invoke_prefixes WHERE utc_hour=?',
            (hour,),
        ).fetchone()
    if len(rows) != 2 or hourly is None:
        raise RuntimeError('recovery probe owners are missing')
    state = {'conversations': rows, 'utc_hour': hour, 'hourly': hourly}
    with open(STATE, 'w', encoding='utf-8') as output:
        json.dump(state, output)
    emit('range-before-crash', **state)


def recover():
    with open(STATE, encoding='utf-8') as source:
        state = json.load(source)
    results = []
    for key, prefix, ceiling in state['conversations']:
        invoke_id = call(key)
        if invoke_id[:6] != prefix or sequence(invoke_id) <= ceiling:
            raise RuntimeError('restart reused a committed conversation reservation')
        results.append({'prefix': prefix, 'old_ceiling': ceiling, 'sequence': sequence(invoke_id)})
    invoke_id = call(None)
    same_hour = int(time.time()) // 3600 == state['utc_hour']
    if same_hour and (invoke_id[:6] != state['hourly'][0] or sequence(invoke_id) <= state['hourly'][1]):
        raise RuntimeError('same-hour restart lost its prefix or reused a committed range')
    emit('range-crash-recovery', conversations=results, hourly_id=invoke_id, same_hour=same_hour)


def verify():
    with base.open_db(base.BUSINESS_DB, timeout=10) as db:
        updates = db.execute('SELECT owner_type,old_ceiling,new_ceiling FROM acceptance_range_updates').fetchall()
        issued = db.execute('SELECT invoke_id FROM codex_invocations WHERE source=\'proxy\' AND LENGTH(invoke_id)=10').fetchall()
        duplicates = db.execute('SELECT invoke_id,COUNT(*) FROM codex_invocations '
                                'WHERE LENGTH(invoke_id)=10 GROUP BY invoke_id HAVING COUNT(*)>1 LIMIT 1').fetchall()
    reservations = [row for row in updates if row[2] > row[1]]
    returns = [row for row in updates if row[2] < row[1]]
    if duplicates or not issued or not reservations:
        raise RuntimeError('missing range evidence or duplicate invocation identity')
    if any(end - start > 64 or end > 923520 for _, start, end in reservations):
        raise RuntimeError('reservation exceeded its bounded range')
    # Cold owners need one initial reservation. The controlled probe separately
    # proves hot owners, while this comparison covers the full mixed workload.
    if len(reservations) >= len(issued):
        raise RuntimeError('range writes remained proportional to each invocation')
    emit('range-write-observation', issued=len(issued), reservations=len(reservations),
         returns=len(returns), duplicate_ids=0, largest_range=max(end-start for _, start, end in reservations))


def reader_ownership(compare=False):
    with base.open_db(base.BUSINESS_DB, timeout=10) as db:
        ownership = {
            'conversations': db.execute('SELECT conversation_id,last_invoke_sequence FROM prompt_cache_conversations ORDER BY conversation_id').fetchall(),
            'hours': db.execute('SELECT utc_hour,prefix,last_invoke_sequence FROM hourly_invoke_prefixes ORDER BY utc_hour').fetchall(),
        }
    # JSON normalization makes tuple/list shapes identical for the persisted snapshot.
    ownership = json.loads(json.dumps(ownership))
    if compare:
        with open(STATE, encoding='utf-8') as source:
            if json.load(source) != ownership:
                raise RuntimeError('older-reader smoke changed durable reservation ownership')
        emit('range-older-reader-ownership-preserved', conversations=len(ownership['conversations']), hours=len(ownership['hours']))
    else:
        with open(STATE, 'w', encoding='utf-8') as output:
            json.dump(ownership, output)


if __name__ == '__main__':
    {'install': install, 'probe': probe, 'snapshot': snapshot, 'recover': recover, 'verify': verify,
     'reader-snapshot': reader_ownership, 'reader-compare': lambda: reader_ownership(True)}[sys.argv[1]]()
