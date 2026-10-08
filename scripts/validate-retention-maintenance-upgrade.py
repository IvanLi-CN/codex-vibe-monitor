#!/usr/bin/env python3
"""Validate each released ownership source with an already-built candidate on Linux."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess

VERSIONS = [*[f"v4.0.{index}" for index in range(7)], "v4.1.0", "v4.1.1", "v4.1.2", "v4.1.3", "v4.2.0", "v4.2.1"]
OWNERS = ["retention_archive", "invocation_identity_cleanup", "raw_orphan_sweep", "prompt_cache_materialization"]


def execute(candidate, root, *arguments, expect_success=True):
    environment = dict(os.environ, DATABASE_PATH=str(root / "business.sqlite"), MAINTENANCE_DATABASE_PATH=str(root / "maintenance.sqlite"), ARCHIVE_DIR=str(root / "archives"), PROXY_RAW_DIR=str(root / "raw"), RETENTION_ENABLED="invalid-retired-value", XY_RETENTION_ENABLED="false", RETENTION_DRY_RUN="true", RUST_LOG="warn")
    result = subprocess.run([str(candidate), *arguments], env=environment, capture_output=True, text=True, timeout=90)
    with (root / "candidate.log").open("a") as log:
        log.write(json.dumps({"arguments": arguments, "exitCode": result.returncode}) + "\n" + result.stdout + result.stderr)
    assert (result.returncode == 0) == expect_success, result.stderr[-2000:]
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
    (target / "raw").mkdir(exist_ok=True)
    raw = target / "raw" / "upgrade-orphan.json"
    raw.write_text('{"fixture":true}')
    os.utime(raw, (1577836800, 1577836800))
    # Explicit dry-run bypasses startup recovery, and the retired switches cannot reject parsing.
    execute(candidate, target, "--retention-dry-run")
    with sqlite3.connect(target / "maintenance.sqlite") as connection:
        rows = {row[0]: row[1:] for row in connection.execute("SELECT task_key,enabled,interval_secs,cron_expr FROM managed_tasks WHERE task_key IN (?,?,?,?)", OWNERS)}
        assert rows["retention_archive"] == (0, 600, None), rows
        assert rows["prompt_cache_materialization"][0] == 0, rows
        assert rows["invocation_identity_cleanup"] == (1, 300, None), rows
        assert rows["raw_orphan_sweep"] == (1, 300, None), rows
        assert connection.execute("SELECT value FROM maintenance_metadata WHERE key='retention_maintenance_ownership_v1'").fetchone() == ("applied",)
        connection.execute("UPDATE managed_tasks SET enabled=0,interval_secs=420,next_trigger_at=NULL WHERE task_key IN ('invocation_identity_cleanup','raw_orphan_sweep')")
    before = business_facts(target)
    for command in [("--retention-dry-run",), ("maintenance", "invocation-identity-cleanup", "--dry-run"), ("maintenance", "raw-orphan-sweep", "--dry-run")]:
        execute(candidate, target, *command)
        assert business_facts(target) == before, command
        assert raw.exists(), command
    # Real archive is explicit even with RETENTION_DRY_RUN=true and leaves both orphan responsibilities alone.
    execute(candidate, target, "--retention-run-once")
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
        assert all(json.loads(details).get("ownerScope") == key and json.loads(details).get("ownershipVersion") == 1 for key, details in records)
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
    return {"source": json.loads((source / "source.json").read_text()), "status": "passed", "claims": ["controls-preserved", "new-defaults", "retired-environment-ignored", "three-preview-scopes-read-only", "explicit-real-mode", "archive-only", "protected-identities", "paused-manual", "no-startup-recovery", "interrupted-forward-reentry"]}


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
