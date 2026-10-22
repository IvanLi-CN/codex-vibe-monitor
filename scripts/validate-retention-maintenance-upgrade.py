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


def retention_command(task_key, dry_run):
    if task_key == "retention_archive":
        return ("--retention-dry-run" if dry_run else "--retention-run-once",)
    command = ("maintenance", RETENTION_COMMANDS[task_key])
    return command + (("--dry-run",) if dry_run else ())


def run_scoped_command(candidate, root, task_key, dry_run):
    with sqlite3.connect(root / "maintenance.sqlite") as connection:
        before_run_id = connection.execute("SELECT COALESCE(MAX(id),0) FROM managed_task_runs").fetchone()[0]
    before_business = business_facts(root)
    before_files = file_facts(root)
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
    return {"source": json.loads((source / "source.json").read_text()), "status": "passed", "claims": ["controls-preserved", "new-defaults", "retired-environment-ignored", "all-retention-owner-previews-read-only", "all-retention-owner-one-shots-scoped", "explicit-real-mode", "archive-only", "protected-identities", "paused-manual", "no-startup-recovery", "interrupted-forward-reentry"]}


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
