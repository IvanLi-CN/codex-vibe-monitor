#!/usr/bin/env python3
"""Validate each released ownership source with an already-built candidate on Linux."""
import argparse
from datetime import datetime, timedelta, timezone
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile

SHANGHAI = timezone(timedelta(hours=8))
VERSIONS = [*[f"v4.0.{index}" for index in range(7)], "v4.1.0", "v4.1.1", "v4.1.2", "v4.1.3", "v4.2.0", "v4.2.1", "v4.2.2"]
RETENTION_OWNERS = [
    "retention_archive",
    "pool_upstream_attempt_archive",
    "forward_proxy_attempt_archive",
    "invocation_detail_prune",
    "quota_snapshot_compaction",
    "model_route_expiry",
    "task_run_history_cleanup",
]
RETENTION_COMMANDS = {
    "pool_upstream_attempt_archive": "pool-upstream-attempt-archive",
    "forward_proxy_attempt_archive": "forward-proxy-attempt-archive",
    "invocation_detail_prune": "invocation-detail-prune",
    "quota_snapshot_compaction": "quota-snapshot-compaction",
    "model_route_expiry": "model-route-expiry",
    "task_run_history_cleanup": "task-run-history-cleanup",
}
LEGACY_OWNERS = ["invocation_identity_cleanup", "raw_orphan_sweep", "prompt_cache_materialization"]
OWNERS = RETENTION_OWNERS + LEGACY_OWNERS
OWNER_VERSIONS = {key: 2 for key in RETENTION_OWNERS}
OWNER_VERSIONS.update({key: 1 for key in LEGACY_OWNERS})

OWNER_FIXTURE_PREFIX = "retention-owner-fixture"
OWNER_RAW_PATHS = {
    "retention_archive": (
        "raw/retention-owner-fixture-archive-request.json",
        "raw/retention-owner-fixture-shared-response.json",
    ),
    "pool_upstream_attempt_archive": ("raw/retention-owner-fixture-shared-response.json",),
    "invocation_detail_prune": ("raw/retention-owner-fixture-detail-response.json",),
}
OWNER_RAW_RELEASED_PATHS = {
    "retention_archive": ("raw/retention-owner-fixture-archive-request.json",),
    "pool_upstream_attempt_archive": ("raw/retention-owner-fixture-shared-response.json",),
    "invocation_detail_prune": ("raw/retention-owner-fixture-detail-response.json",),
}
OWNER_ARCHIVE_DATASETS = {
    "retention_archive": "codex_invocations",
    "pool_upstream_attempt_archive": "pool_upstream_request_attempts",
    "forward_proxy_attempt_archive": "forward_proxy_attempts",
    "invocation_detail_prune": "codex_invocations",
    "quota_snapshot_compaction": "codex_quota_snapshots",
}
OWNER_ARCHIVE_FIXTURES = {
    "retention_archive": ("codex_invocations", "invoke_id", f"{OWNER_FIXTURE_PREFIX}-archive"),
    "pool_upstream_attempt_archive": ("pool_upstream_request_attempts", "invoke_id", f"{OWNER_FIXTURE_PREFIX}-pool"),
    "forward_proxy_attempt_archive": ("forward_proxy_attempts", "proxy_key", f"{OWNER_FIXTURE_PREFIX}-forward"),
    "invocation_detail_prune": ("codex_invocations", "invoke_id", f"{OWNER_FIXTURE_PREFIX}-detail"),
    "quota_snapshot_compaction": ("codex_quota_snapshots", "captured_at", "2020-01-01 00:00:00"),
}
OWNER_RAW_REFERENCES = {
    "retention_archive": (("codex_invocations", "request_raw_path"), ("codex_invocations", "response_raw_path")),
    "pool_upstream_attempt_archive": (("pool_upstream_request_attempts", "response_raw_path"),),
    "invocation_detail_prune": (("codex_invocations", "response_raw_path"),),
}


def seed_owner_candidates(root):
    now_local = datetime.now(timezone.utc).astimezone(SHANGHAI).replace(tzinfo=None)
    old_archive = (now_local - timedelta(days=120)).strftime("%Y-%m-%d %H:%M:%S")
    old_retained = (now_local - timedelta(days=60)).strftime("%Y-%m-%d %H:%M:%S")
    raw_dir = root / "raw"
    raw_dir.mkdir(exist_ok=True)
    for relative_path in {path for paths in OWNER_RAW_PATHS.values() for path in paths}:
        (root / relative_path).write_text(json.dumps({"fixture": relative_path}))
    with sqlite3.connect(root / "business.sqlite") as connection:
        connection.execute(
            "INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,payload,raw_response,request_raw_path,response_raw_path,detail_level,created_at) VALUES (?,?,?,?,?,?,?,?,?,?)",
            (f"{OWNER_FIXTURE_PREFIX}-archive", old_archive, "proxy", "completed", "{}", "{}", OWNER_RAW_PATHS["retention_archive"][0], OWNER_RAW_PATHS["retention_archive"][1], "full", old_archive),
        )
        connection.execute(
            "INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,payload,raw_response,response_raw_path,detail_level,created_at) VALUES (?,?,?,?,?,?,?,?,?)",
            (f"{OWNER_FIXTURE_PREFIX}-detail", old_retained, "proxy", "completed", "{}", "{}", OWNER_RAW_PATHS["invocation_detail_prune"][0], "full", old_retained),
        )
        connection.execute(
            "INSERT INTO pool_upstream_request_attempts (invoke_id,occurred_at,endpoint,route_mode,attempt_index,distinct_account_index,same_account_retry_index,status,response_raw_path) VALUES (?,?,?,?,?,?,?,?,?)",
            (f"{OWNER_FIXTURE_PREFIX}-pool", old_retained, "https://fixture.invalid", "fixture", 1, 1, 1, "success", OWNER_RAW_PATHS["pool_upstream_attempt_archive"][0]),
        )
        connection.execute(
            "INSERT INTO forward_proxy_attempts (proxy_key,occurred_at,is_success) VALUES (?,?,?)",
            (f"{OWNER_FIXTURE_PREFIX}-forward", old_retained, 1),
        )
        connection.execute(
            "INSERT INTO codex_quota_snapshots (captured_at,amount_limit,used_amount,remaining_amount) VALUES (?,?,?,?)",
            ("2020-01-01 00:00:00", 100.0, 1.0, 99.0),
        )
        connection.execute(
            "INSERT INTO codex_quota_snapshots (captured_at,amount_limit,used_amount,remaining_amount) VALUES (?,?,?,?)",
            ("2020-01-01 00:01:00", 100.0, 2.0, 98.0),
        )
        connection.execute(
            "INSERT INTO pool_upstream_accounts (kind,display_name,status,created_at,updated_at) VALUES (?,?,?,?,?)",
            ("api_key_codex", f"{OWNER_FIXTURE_PREFIX}-account", "active", old_retained, old_retained),
        )
        account_id = connection.execute("SELECT last_insert_rowid()").fetchone()[0]
        connection.execute(
            "INSERT INTO pool_upstream_account_model_routes (account_id,model,last_seen_at) VALUES (?,?,?)",
            (account_id, f"{OWNER_FIXTURE_PREFIX}-model", old_retained),
        )
        connection.executemany(
            "INSERT INTO system_task_runs (task_kind,trigger_kind,status,summary,started_at,finished_at,duration_ms) VALUES (?,?,?,?,?,?,?)",
            [
                (
                    f"{OWNER_FIXTURE_PREFIX}-history",
                    "manual",
                    "success",
                    "fixture",
                    f"2020-01-01T00:{index // 60:02d}:{index % 60:02d}.000Z",
                    f"2020-01-01T00:{index // 60:02d}:{index % 60:02d}.000Z",
                    1,
                )
                for index in range(201)
            ],
        )


def owner_candidate_state(root, task_key):
    with sqlite3.connect(root / "business.sqlite") as connection:
        queries = {
            "retention_archive": ("SELECT COUNT(*) FROM codex_invocations WHERE invoke_id=?", f"{OWNER_FIXTURE_PREFIX}-archive"),
            "pool_upstream_attempt_archive": ("SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id=?", f"{OWNER_FIXTURE_PREFIX}-pool"),
            "forward_proxy_attempt_archive": ("SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key=?", f"{OWNER_FIXTURE_PREFIX}-forward"),
            "invocation_detail_prune": ("SELECT COUNT(*) FROM codex_invocations WHERE invoke_id=? AND detail_level='full'", f"{OWNER_FIXTURE_PREFIX}-detail"),
            "model_route_expiry": ("SELECT COUNT(*) FROM pool_upstream_account_model_routes WHERE model=?", f"{OWNER_FIXTURE_PREFIX}-model"),
        }
        if task_key == "quota_snapshot_compaction":
            return connection.execute("SELECT COUNT(*) FROM codex_quota_snapshots WHERE captured_at LIKE '2020-01-01 00:%'").fetchone()[0]
        if task_key == "task_run_history_cleanup":
            return connection.execute("SELECT COUNT(*) FROM system_task_runs WHERE task_kind=?", (f"{OWNER_FIXTURE_PREFIX}-history",)).fetchone()[0]
        query, value = queries[task_key]
        return connection.execute(query, (value,)).fetchone()[0]


def owner_for_arguments(arguments):
    if arguments in {("--retention-dry-run",), ("--retention-run-once",)}:
        return "retention_archive"
    if len(arguments) >= 2 and arguments[:1] == ("maintenance",):
        return {
            "pool-upstream-attempt-archive": "pool_upstream_attempt_archive",
            "forward-proxy-attempt-archive": "forward_proxy_attempt_archive",
            "invocation-detail-prune": "invocation_detail_prune",
            "quota-snapshot-compaction": "quota_snapshot_compaction",
            "model-route-expiry": "model_route_expiry",
            "task-run-history-cleanup": "task_run_history_cleanup",
            "invocation-identity-cleanup": "invocation_identity_cleanup",
            "raw-orphan-sweep": "raw_orphan_sweep",
        }.get(arguments[1])
    return None


def output_text(value):
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode(errors="replace")
    return value


def execute(candidate, root, *arguments, expect_success=True):
    environment = dict(os.environ, DATABASE_PATH=str(root / "business.sqlite"), MAINTENANCE_DATABASE_PATH=str(root / "maintenance.sqlite"), ARCHIVE_DIR=str(root / "archives"), PROXY_RAW_DIR=str(root / "raw"), INVOCATION_SUCCESS_FULL_DAYS="30", INVOCATION_MAX_DAYS="90", FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS="30", POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS="7", QUOTA_SNAPSHOT_FULL_DAYS="30", RETENTION_ENABLED="invalid-retired-value", XY_RETENTION_ENABLED="false", RETENTION_DRY_RUN="true", RUST_LOG="warn")
    version = root.name
    owner = owner_for_arguments(arguments)
    try:
        result = subprocess.run([str(candidate), *arguments], env=environment, capture_output=True, text=True, timeout=90)
        result_record = {
            "version": version,
            "owner": owner,
            "arguments": arguments,
            "exitCode": result.returncode,
            "stdout": output_text(result.stdout),
            "stderr": output_text(result.stderr),
        }
    except subprocess.TimeoutExpired as error:
        result_record = {
            "version": version,
            "owner": owner,
            "arguments": arguments,
            "exitCode": None,
            "timeoutSeconds": 90,
            "stdout": output_text(error.stdout),
            "stderr": output_text(error.stderr),
        }
        with (root / "candidate.log").open("a") as log:
            log.write(json.dumps(result_record, ensure_ascii=False) + "\n")
        with (root / "candidate-diagnostics.jsonl").open("a") as diagnostics:
            diagnostics.write(json.dumps(result_record, ensure_ascii=False) + "\n")
        raise
    with (root / "candidate.log").open("a") as log:
        log.write(
            json.dumps(
                {key: value for key, value in result_record.items() if key not in {"stdout", "stderr"}},
                ensure_ascii=False,
            )
            + "\n"
            + result_record["stdout"]
            + result_record["stderr"]
        )
    recorded_task_result = None
    try:
        with sqlite3.connect(root / "maintenance.sqlite") as connection:
            row = connection.execute(
                "SELECT task_key,status,summary,details FROM managed_task_runs ORDER BY id DESC LIMIT 1"
            ).fetchone()
        if row is not None:
            recorded_task_result = {
                "taskKey": row[0],
                "status": row[1],
                "summary": row[2],
                "details": row[3],
            }
    except sqlite3.Error as error:
        recorded_task_result = {"readError": str(error)}
    result_record["recordedTaskResult"] = recorded_task_result
    with (root / "candidate.log").open("a") as log:
        log.write("recordedTaskResult=" + json.dumps(recorded_task_result, ensure_ascii=False) + "\n")
    with (root / "candidate-diagnostics.jsonl").open("a") as diagnostics:
        diagnostics.write(json.dumps(result_record, ensure_ascii=False) + "\n")
    assert (result.returncode == 0) == expect_success, (
        result.stdout[-4000:]
        + result.stderr[-2000:]
        + "\nrecordedTaskResult="
        + json.dumps(recorded_task_result, ensure_ascii=False)
    )
    return result


def business_facts(root):
    with sqlite3.connect(root / "business.sqlite") as connection:
        facts = {}
        for table in ["codex_invocations", "prompt_cache_conversations", "hourly_invoke_prefixes", "prompt_cache_conversation_orphan_cleanup_state", "retention_recovery_cursors", "retention_raw_reconciliation", "retention_prepared_archives", "proxy_raw_payload_blob_links", "archive_batches"]:
            exists = connection.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", (table,)).fetchone()
            if exists:
                # Structure initialization may add a scope; subsequent previews must be strictly read-only.
                facts[table] = connection.execute(f'SELECT * FROM "{table}" ORDER BY rowid').fetchall()
        return facts


def file_facts(root):
    facts = []
    for directory_name in ["raw", "archives"]:
        directory = root / directory_name
        if not directory.exists():
            continue
        for path in sorted(path for path in directory.rglob("*") if path.is_file()):
            facts.append(
                (
                    str(path.relative_to(root)),
                    path.stat().st_size,
                    hashlib.sha256(path.read_bytes()).hexdigest(),
                )
            )
    return facts


def archive_manifest_facts(root, dataset):
    with sqlite3.connect(root / "business.sqlite") as connection:
        return connection.execute(
            "SELECT file_path,sha256,status,row_count FROM archive_batches WHERE dataset=? ORDER BY id",
            (dataset,),
        ).fetchall()


def assert_new_archive_manifests(root, dataset, before):
    before_by_path = {row[0]: row[1:] for row in before}
    changed = [row for row in archive_manifest_facts(root, dataset) if before_by_path.get(row[0]) != row[1:]]
    assert changed, (dataset, before)
    for file_path, sha256, status, row_count in changed:
        archive_path = Path(file_path)
        if not archive_path.is_absolute():
            archive_path = root / archive_path
        assert status == "completed" and row_count > 0 and archive_path.is_file(), (dataset, file_path, status, row_count)
        assert sha256 == hashlib.sha256(archive_path.read_bytes()).hexdigest(), (dataset, file_path, sha256)
    return changed


def assert_archive_contains_fixture(root, dataset, changed, task_key):
    table, column, value = OWNER_ARCHIVE_FIXTURES[task_key]
    found = False
    for file_path, _, _, _ in changed:
        archive_path = Path(file_path)
        if not archive_path.is_absolute():
            archive_path = root / archive_path
        with tempfile.TemporaryDirectory() as directory:
            sqlite_path = Path(directory) / "archive.sqlite"
            if archive_path.suffix == ".gz":
                with gzip.open(archive_path, "rb") as source, sqlite_path.open("wb") as target:
                    shutil.copyfileobj(source, target)
            else:
                shutil.copyfile(archive_path, sqlite_path)
            with sqlite3.connect(sqlite_path) as connection:
                fixture_count = connection.execute(
                    f"SELECT COUNT(*) FROM {table} WHERE {column}=?",
                    (value,),
                ).fetchone()[0]
                found = fixture_count == 1
                if found and task_key == "retention_archive":
                    assert connection.execute(
                        "SELECT request_raw_path,response_raw_path FROM codex_invocations WHERE invoke_id=?",
                        (value,),
                    ).fetchone() == OWNER_RAW_PATHS["retention_archive"], (task_key, file_path)
                if found and task_key == "pool_upstream_attempt_archive":
                    assert connection.execute(
                        "SELECT response_raw_path FROM pool_upstream_request_attempts WHERE invoke_id=?",
                        (value,),
                    ).fetchone() == (OWNER_RAW_PATHS["pool_upstream_attempt_archive"][0],), (task_key, file_path)
        if found:
            break
    assert found, (task_key, dataset, changed)


def assert_fixture_raw_references_released(root, task_key):
    released_paths = OWNER_RAW_RELEASED_PATHS.get(task_key, ())
    for relative_path in released_paths:
        assert not (root / relative_path).exists(), (task_key, relative_path)
    with sqlite3.connect(root / "business.sqlite") as connection:
        for table, column in OWNER_RAW_REFERENCES.get(task_key, ()):
            assert connection.execute(
                f"SELECT COUNT(*) FROM {table} WHERE {column} IN ({','.join('?' for _ in released_paths)})",
                released_paths,
            ).fetchone()[0] == 0, (task_key, table, column)
    if task_key == "retention_archive":
        shared_path = OWNER_RAW_PATHS["pool_upstream_attempt_archive"][0]
        assert (root / shared_path).is_file(), (task_key, shared_path)
        with sqlite3.connect(root / "business.sqlite") as connection:
            assert connection.execute(
                "SELECT COUNT(*) FROM proxy_raw_payload_blob_links WHERE raw_path=?",
                (shared_path,),
            ).fetchone()[0] > 0, (task_key, shared_path)


def retention_command(task_key, dry_run):
    if task_key == "retention_archive":
        return ("--retention-dry-run" if dry_run else "--retention-run-once",)
    command = ("maintenance", RETENTION_COMMANDS[task_key])
    return command + (("--dry-run",) if dry_run else ())


def assert_real_owner_result(details, task_key):
    result = details.get("result")
    assert result in {"complete", "partial"}, (task_key, details)
    return result


def run_scoped_command(candidate, root, task_key, dry_run):
    with sqlite3.connect(root / "maintenance.sqlite") as connection:
        before_run_id = connection.execute("SELECT COALESCE(MAX(id),0) FROM managed_task_runs").fetchone()[0]
    before_business = business_facts(root)
    before_files = file_facts(root)
    before_owner_state = owner_candidate_state(root, task_key)
    archive_dataset = OWNER_ARCHIVE_DATASETS.get(task_key)
    before_archives = archive_manifest_facts(root, archive_dataset) if not dry_run and archive_dataset else []
    command = retention_command(task_key, dry_run)
    execute(candidate, root, *command)
    with sqlite3.connect(root / "maintenance.sqlite") as connection:
        records = connection.execute(
            "SELECT id,task_key,details FROM managed_task_runs WHERE id>? AND trigger_kind='manual' AND details IS NOT NULL ORDER BY id",
            (before_run_id,),
        ).fetchall()
    assert len(records) == 1, (task_key, dry_run, records)
    record_id, recorded_task_key, details = records[0]
    parsed = json.loads(details)
    assert recorded_task_key == task_key and parsed.get("ownerScope") == task_key, (task_key, record_id, parsed)
    assert parsed.get("ownershipVersion") == OWNER_VERSIONS[task_key], (task_key, parsed)
    assert parsed.get("dryRun") is dry_run, (task_key, dry_run, parsed)
    assert parsed.get("discovered", 0) > 0 and parsed.get("selected", 0) > 0, (task_key, parsed)
    if dry_run:
        assert parsed.get("committed") == 0, (task_key, parsed)
        assert owner_candidate_state(root, task_key) == before_owner_state, (task_key, parsed)
        if task_key in OWNER_RAW_PATHS:
            raw_files_planned = parsed.get("rawFilesPlanned")
            if raw_files_planned is None:
                raw_files_planned = (parsed.get("selectedScope") or {}).get("rawFilesPlanned", 0)
            assert raw_files_planned > 0, (task_key, parsed)
    else:
        assert parsed.get("committed", 0) > 0, (task_key, parsed)
        assert_real_owner_result(parsed, task_key)
        assert owner_candidate_state(root, task_key) < before_owner_state, (task_key, parsed)
        if archive_dataset:
            changed_archives = assert_new_archive_manifests(root, archive_dataset, before_archives)
            assert_archive_contains_fixture(root, archive_dataset, changed_archives, task_key)
        assert_fixture_raw_references_released(root, task_key)
    if dry_run:
        assert business_facts(root) == before_business, command
        assert file_facts(root) == before_files, command
    return parsed


def validate_source(candidate, source, target):
    assert not target.exists(), f"Output target already exists: {target}"
    shutil.copytree(source, target)
    with sqlite3.connect(target / "maintenance.sqlite") as connection:
        connection.execute("UPDATE managed_tasks SET enabled=0,interval_secs=600,cron_expr=NULL,schedule_source='override',next_trigger_at=NULL WHERE task_key='retention_archive'")
        connection.execute("UPDATE managed_tasks SET enabled=0 WHERE task_key='prompt_cache_materialization'")
    with sqlite3.connect(target / "business.sqlite") as connection:
        connection.execute("INSERT INTO prompt_cache_conversations(prompt_cache_key,conversation_id,created_at,updated_at) VALUES('upgrade-orphan','RZYZAA','2020-01-01','2020-01-01')")
        connection.execute("INSERT INTO prompt_cache_conversations(prompt_cache_key,conversation_id,created_at,updated_at) VALUES('upgrade-protected','RZYZAB','2020-01-01','2020-01-01')")
        connection.execute("INSERT INTO hourly_invoke_prefixes(utc_hour,prefix) VALUES(1,'HZYZAA')")
        connection.execute("INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,payload,raw_response) VALUES('RZYZABAAAA','2099-01-01T00:00:00Z','proxy','running','{\"promptCacheKey\":\"upgrade-protected\"}','{}')")
    seed_owner_candidates(target)
    (target / "raw").mkdir(exist_ok=True)
    raw = target / "raw" / "upgrade-orphan.json"
    raw.write_text('{"fixture":true}')
    os.utime(raw, (1577836800, 1577836800))
    # Explicit dry-run bypasses startup recovery, and the retired switches cannot reject parsing.
    execute(candidate, target, "--retention-dry-run")
    with sqlite3.connect(target / "maintenance.sqlite") as connection:
        owner_placeholders = ",".join("?" for _ in OWNERS)
        rows = {
            row[0]: row[1:]
            for row in connection.execute(
                f"SELECT task_key,enabled,interval_secs,cron_expr FROM managed_tasks WHERE task_key IN ({owner_placeholders})",
                OWNERS,
            )
        }
        assert rows["retention_archive"] == (0, 600, None), rows
        for key in RETENTION_OWNERS[1:]:
            assert rows[key] == (1, 300, None), (key, rows)
        assert rows["prompt_cache_materialization"][0] == 0, rows
        assert rows["invocation_identity_cleanup"] == (1, 300, None), rows
        assert rows["raw_orphan_sweep"] == (1, 300, None), rows
        assert connection.execute("SELECT value FROM maintenance_metadata WHERE key='retention_maintenance_ownership_v1'").fetchone() == ("applied",)
        connection.execute("UPDATE managed_tasks SET enabled=0,interval_secs=420,next_trigger_at=NULL WHERE task_key IN ('invocation_identity_cleanup','raw_orphan_sweep')")
    for task_key in RETENTION_OWNERS:
        run_scoped_command(candidate, target, task_key, True)
    before = business_facts(target)
    before_files = file_facts(target)
    for task_key, command in [
        ("invocation_identity_cleanup", ("maintenance", "invocation-identity-cleanup", "--dry-run")),
        ("raw_orphan_sweep", ("maintenance", "raw-orphan-sweep", "--dry-run")),
    ]:
        with sqlite3.connect(target / "maintenance.sqlite") as connection:
            before_run_id = connection.execute("SELECT COALESCE(MAX(id),0) FROM managed_task_runs").fetchone()[0]
        execute(candidate, target, *command)
        with sqlite3.connect(target / "maintenance.sqlite") as connection:
            records = connection.execute(
                "SELECT id,task_key,details FROM managed_task_runs WHERE id>? AND trigger_kind='manual' AND details IS NOT NULL ORDER BY id",
                (before_run_id,),
            ).fetchall()
        assert len(records) == 1 and records[0][1] == task_key, (task_key, records)
        parsed = json.loads(records[0][2])
        assert parsed.get("ownerScope") == task_key and parsed.get("ownershipVersion") == OWNER_VERSIONS[task_key]
        assert parsed.get("dryRun") is True
        assert business_facts(target) == before, command
        assert file_facts(target) == before_files and raw.exists(), command
    # Real retention owners are explicit even with RETENTION_DRY_RUN=true. Each one-shot is scoped to its owner.
    for task_key in RETENTION_OWNERS:
        run_scoped_command(candidate, target, task_key, False)
    # The archive owner leaves both orphan responsibilities alone.
    with sqlite3.connect(target / "business.sqlite") as connection:
        assert connection.execute("SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key='upgrade-orphan'").fetchone()[0] == 1
        assert connection.execute("SELECT COUNT(*) FROM hourly_invoke_prefixes WHERE utc_hour=1").fetchone()[0] == 1
        assert connection.execute("SELECT status FROM codex_invocations WHERE invoke_id='RZYZABAAAA'").fetchone() == ("running",)
    execute(candidate, target, "maintenance", "raw-orphan-sweep")
    with sqlite3.connect(target / "business.sqlite") as connection:
        assert connection.execute("SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key='upgrade-orphan'").fetchone()[0] == 1
    execute(candidate, target, "maintenance", "invocation-identity-cleanup")
    with sqlite3.connect(target / "business.sqlite") as connection:
        assert connection.execute("SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key='upgrade-orphan'").fetchone()[0] == 0
        assert connection.execute("SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key='upgrade-protected'").fetchone()[0] == 1
        assert connection.execute("SELECT COUNT(*) FROM hourly_invoke_prefixes WHERE utc_hour=1").fetchone()[0] == 0
    with sqlite3.connect(target / "maintenance.sqlite") as connection:
        rows = list(connection.execute("SELECT enabled,interval_secs FROM managed_tasks WHERE task_key IN ('invocation_identity_cleanup','raw_orphan_sweep')"))
        assert rows == [(0, 420), (0, 420)], rows
        records = connection.execute("SELECT task_key,details FROM managed_task_runs WHERE trigger_kind='manual' AND details IS NOT NULL").fetchall()
        assert {key for key, _ in records}.issuperset(set(RETENTION_OWNERS)), records
        for key, details in records:
            parsed = json.loads(details)
            assert parsed.get("ownerScope") == key
            assert parsed.get("ownershipVersion") == OWNER_VERSIONS[key]
            assert isinstance(parsed.get("dryRun"), bool)
        for key in RETENTION_OWNERS:
            owner_records = [json.loads(details) for record_key, details in records if record_key == key]
            assert any(record.get("dryRun") is True for record in owner_records), key
            assert any(record.get("dryRun") is False for record in owner_records), key
        observations = connection.execute("SELECT details,actual_started_at,actual_finished_at,actual_duration_ms FROM managed_task_runs WHERE trigger_kind='manual' AND details IS NOT NULL").fetchall()
        for details, actual_started, actual_finished, duration in observations:
            expected_start = json.loads(details).get("actualStartedAt")
            if expected_start is not None:
                assert actual_started == expected_start and actual_finished >= actual_started and duration >= 0
        assert connection.execute("SELECT COUNT(*) FROM managed_task_work_runs").fetchone()[0] >= len(records), "offline CLI must persist its workload observations before releasing runtime locks"
    # Inject a genuine initialization failure, then resume the same database forward.
    with sqlite3.connect(target / "maintenance.sqlite") as connection:
        connection.execute("DELETE FROM maintenance_metadata WHERE key='retention_maintenance_ownership_v1'")
        connection.execute("CREATE TRIGGER interrupted_owner_marker BEFORE INSERT ON maintenance_metadata WHEN NEW.key='retention_maintenance_ownership_v1' BEGIN SELECT RAISE(ABORT,'upgrade interruption'); END")
    execute(candidate, target, "maintenance", "invocation-identity-cleanup", "--dry-run", expect_success=False)
    with sqlite3.connect(target / "maintenance.sqlite") as connection:
        assert not connection.execute("SELECT 1 FROM maintenance_metadata WHERE key='retention_maintenance_ownership_v1'").fetchone()
        connection.execute("DROP TRIGGER interrupted_owner_marker")
    execute(candidate, target, "maintenance", "invocation-identity-cleanup", "--dry-run")
    execute(candidate, target, "maintenance", "invocation-identity-cleanup", "--dry-run")
    return {"source": json.loads((source / "source.json").read_text()), "status": "passed", "claims": ["controls-preserved", "new-defaults", "retired-environment-ignored", "all-retention-owner-previews-read-only", "all-retention-owner-one-shots-mutated-owner-fixtures-with-archive-raw-proofs", "explicit-real-mode", "archive-only", "protected-identities", "paused-manual", "no-startup-recovery", "interrupted-forward-reentry"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixtures-root", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=False)
    candidate_digest = hashlib.sha256(args.candidate.read_bytes()).hexdigest()
    (args.output_dir / "candidate.json").write_text(json.dumps({"candidateSha256": candidate_digest, "sourceVersions": VERSIONS}, indent=2) + "\n")
    results = []
    for version in VERSIONS:
        result = validate_source(args.candidate.resolve(), args.fixtures_root / version, args.output_dir / version)
        result["candidateSha256"] = candidate_digest
        results.append(result)
        (args.output_dir / "results.json").write_text(json.dumps(results, indent=2) + "\n")
        print(version, "passed", flush=True)


if __name__ == "__main__":
    main()
