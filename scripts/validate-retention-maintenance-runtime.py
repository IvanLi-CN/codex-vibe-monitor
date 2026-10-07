#!/usr/bin/env python3
"""Exercise owned maintenance against a real Linux daemon and a caller-leased HTTP port."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import time
import urllib.error
import urllib.request

from importlib.util import module_from_spec, spec_from_file_location

SPEC = spec_from_file_location("ownership_upgrade", Path(__file__).with_name("validate-retention-maintenance-upgrade.py"))
UPGRADE = module_from_spec(SPEC)
SPEC.loader.exec_module(UPGRADE)


def await_condition(predicate, timeout, message):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise AssertionError(message)


def runtime_locks(root):
    handles = []
    for database in [root / "business.sqlite", root / "maintenance.sqlite"]:
        handle = Path(str(database.resolve()) + ".runtime.lock").open("w+")
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        handle.write("service:ownership-v1:ready")
        handle.flush()
        handles.append(handle)
    return handles


def validate_unavailable(candidate, source, target):
    target.mkdir()
    shutil.copy2(source / "business.sqlite", target / "business.sqlite")
    handles = runtime_locks(target)
    try:
        before = UPGRADE.business_facts(target)
        result = UPGRADE.execute(candidate, target, "maintenance", "invocation-identity-cleanup", "--dry-run", expect_success=False)
        assert "maintenance unavailable" in result.stderr, result.stderr
        assert not (target / "maintenance.sqlite").exists(), "online CLI must not create the unavailable maintenance store"
        assert UPGRADE.business_facts(target) == before
    finally:
        for handle in handles:
            handle.close()


def validate_online(candidate, source, target, port):
    shutil.copytree(source, target)
    UPGRADE.execute(candidate, target, "--retention-dry-run")
    with sqlite3.connect(target / "maintenance.sqlite") as connection:
        connection.execute("UPDATE managed_tasks SET enabled=0,next_trigger_at=NULL,next_catchup_at=NULL")
        controls = connection.execute("SELECT task_key,enabled,interval_secs,cron_expr,next_trigger_at,next_catchup_at FROM managed_tasks WHERE task_key IN (?,?,?,?) ORDER BY task_key", UPGRADE.OWNERS).fetchall()
        baseline_run_id = connection.execute("SELECT COALESCE(MAX(id),0) FROM managed_task_runs").fetchone()[0]
    environment = dict(os.environ, DATABASE_PATH=str(target / "business.sqlite"), MAINTENANCE_DATABASE_PATH=str(target / "maintenance.sqlite"), ARCHIVE_DIR=str(target / "archives"), PROXY_RAW_DIR=str(target / "raw"), HTTP_BIND=f"127.0.0.1:{port}", OPENAI_UPSTREAM_BASE_URL="http://127.0.0.1:1/", OBSERVABILITY_ENABLED="false", RUST_LOG="warn", RETENTION_ENABLED="invalid-retired-value", XY_RETENTION_ENABLED="false")
    (target / "raw").mkdir(exist_ok=True)
    with (target / "daemon.log").open("w") as daemon_log:
        daemon = subprocess.Popen([str(candidate)], env=environment, stdout=daemon_log, stderr=subprocess.STDOUT)
        pending_cli = None
        blocker = None
        try:
            def healthy():
                assert daemon.poll() is None, "daemon exited before HTTP readiness; inspect daemon.log"
                try:
                    with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=1) as response:
                        return response.status == 200
                except (OSError, urllib.error.URLError):
                    return False
            await_condition(healthy, 180, "daemon HTTP readiness timed out")
            # Insert after startup: invoking CLI must not rerun startup orphan recovery.
            with sqlite3.connect(target / "business.sqlite") as connection:
                connection.execute("INSERT INTO prompt_cache_conversations(prompt_cache_key,conversation_id,created_at,updated_at) VALUES('runtime-orphan','TZYZAA','2020-01-01','2020-01-01')")
                connection.execute("INSERT INTO prompt_cache_conversations(prompt_cache_key,conversation_id,created_at,updated_at) VALUES('runtime-protected','TZYZAB','2020-01-01','2020-01-01')")
                connection.execute("INSERT INTO hourly_invoke_prefixes(utc_hour,prefix) VALUES(1,'JZYZAA')")
                connection.execute("INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,payload,raw_response) VALUES('TZYZABAAAA','2099-01-01T00:00:00Z','proxy','running','{\"promptCacheKey\":\"runtime-protected\"}','{}')")
            before = UPGRADE.business_facts(target)
            for command in [("--retention-dry-run",), ("maintenance", "invocation-identity-cleanup", "--dry-run"), ("maintenance", "raw-orphan-sweep", "--dry-run")]:
                UPGRADE.execute(candidate, target, *command)
                assert UPGRADE.business_facts(target) == before, command
            UPGRADE.execute(candidate, target, "--retention-run-once")
            with sqlite3.connect(target / "business.sqlite") as connection:
                assert connection.execute("SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key='runtime-orphan'").fetchone() == (1,)
                assert connection.execute("SELECT COUNT(*) FROM hourly_invoke_prefixes WHERE utc_hour=1").fetchone() == (1,)
                assert connection.execute("SELECT status FROM codex_invocations WHERE invoke_id='TZYZABAAAA'").fetchone() == ("running",)
            UPGRADE.execute(candidate, target, "maintenance", "raw-orphan-sweep")
            UPGRADE.execute(candidate, target, "maintenance", "invocation-identity-cleanup")
            with sqlite3.connect(target / "business.sqlite") as connection:
                assert connection.execute("SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key='runtime-orphan'").fetchone() == (0,)
                assert connection.execute("SELECT COUNT(*) FROM hourly_invoke_prefixes WHERE utc_hour=1").fetchone() == (0,)
                assert connection.execute("SELECT status FROM codex_invocations WHERE invoke_id='TZYZABAAAA'").fetchone() == ("running",)
            with sqlite3.connect(target / "maintenance.sqlite") as connection:
                prompt_schedule = connection.execute("SELECT task_name,next_run_after,next_probe_at,suspension_reason,enabled FROM startup_backfill_progress WHERE task_name LIKE '%prompt_cache%' ORDER BY task_name").fetchall()
            request = urllib.request.Request(f"http://127.0.0.1:{port}/api/system/managed-tasks/prompt_cache_materialization/run", data=b"{}", headers={"content-type": "application/json"}, method="POST")
            with urllib.request.urlopen(request, timeout=5) as response:
                assert response.status == 200
            def prompt_finished():
                with sqlite3.connect(target / "maintenance.sqlite") as connection:
                    row = connection.execute("SELECT status FROM managed_task_runs WHERE task_key='prompt_cache_materialization' AND id>? ORDER BY id DESC LIMIT 1", (baseline_run_id,)).fetchone()
                    return row is not None and row[0] not in ('requested', 'running')
            await_condition(prompt_finished, 90, "paused manual materialization did not complete")
            with sqlite3.connect(target / "maintenance.sqlite") as connection:
                assert connection.execute("SELECT task_name,next_run_after,next_probe_at,suspension_reason,enabled FROM startup_backfill_progress WHERE task_name LIKE '%prompt_cache%' ORDER BY task_name").fetchall() == prompt_schedule, "manual materialization scheduled automatic continuation"
                assert connection.execute("SELECT status FROM managed_task_runs WHERE task_key='prompt_cache_materialization' AND id>? ORDER BY id DESC LIMIT 1", (baseline_run_id,)).fetchone()[0] in ('success', 'skipped')
            # An external transaction keeps the first real identity request active while
            # another CLI and the page API compete for the same production task lease.
            blocker = sqlite3.connect(target / "business.sqlite")
            blocker.execute("BEGIN IMMEDIATE")
            pending_cli = subprocess.Popen([str(candidate), "maintenance", "invocation-identity-cleanup"], env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            def identity_active():
                with sqlite3.connect(target / "maintenance.sqlite") as connection:
                    return connection.execute("SELECT 1 FROM managed_task_runs WHERE task_key='invocation_identity_cleanup' AND status IN ('requested','running')").fetchone() is not None
            await_condition(identity_active, 10, "first CLI did not enqueue its identity request")
            duplicate = UPGRADE.execute(candidate, target, "maintenance", "invocation-identity-cleanup", expect_success=False)
            assert "active run" in duplicate.stderr, duplicate.stderr
            request = urllib.request.Request(f"http://127.0.0.1:{port}/api/system/managed-tasks/invocation_identity_cleanup/run", data=b"{}", headers={"content-type": "application/json"}, method="POST")
            try:
                urllib.request.urlopen(request, timeout=2).close()
                raise AssertionError("page API accepted a duplicate identity request")
            except urllib.error.HTTPError as error:
                assert error.code == 409, error.code
            blocker.rollback()
            blocker.close()
            blocker = None
            stdout, stderr = pending_cli.communicate(timeout=90)
            assert pending_cli.returncode == 0, stderr
            with (target / "candidate.log").open("a") as log:
                log.write(json.dumps({"arguments": ["maintenance", "invocation-identity-cleanup"], "exitCode": pending_cli.returncode, "competition": True}) + "\n" + stdout + stderr)
            pending_cli = None
            with sqlite3.connect(target / "maintenance.sqlite") as connection:
                assert connection.execute("SELECT task_key,enabled,interval_secs,cron_expr,next_trigger_at,next_catchup_at FROM managed_tasks WHERE task_key IN (?,?,?,?) ORDER BY task_key", UPGRADE.OWNERS).fetchall() == controls, "manual requests changed automatic controls or planned a continuation"
                records = connection.execute("SELECT task_key,trigger_kind,details FROM managed_task_runs WHERE id > ? AND task_key IN (?,?,?,?)", [baseline_run_id, *UPGRADE.OWNERS]).fetchall()
                assert records
                for key, trigger, details in records:
                    assert trigger == "manual", (key, trigger)
                    details = json.loads(details)
                    assert details["ownershipVersion"] == 1 and details["ownerScope"] == key
                    assert isinstance(details["dryRun"], bool)
        finally:
            if blocker is not None:
                blocker.rollback()
                blocker.close()
            if pending_cli is not None and pending_cli.poll() is None:
                pending_cli.terminate()
                pending_cli.communicate(timeout=15)
            if daemon.poll() is None:
                daemon.send_signal(signal.SIGTERM)
                try:
                    daemon.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    daemon.kill()
                    daemon.wait()
                    raise AssertionError("daemon failed to drain shutdown within 30 seconds")
        assert daemon.returncode == 0, f"daemon exit: {daemon.returncode}"
    # Graceful shutdown must release both lifetime locks without unlinking either inode.
    handles = runtime_locks(target)
    for handle in handles:
        handle.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-fixture", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--http-port", type=int, required=True, help="A caller-leased port, checked free before this command")
    args = parser.parse_args()
    assert 1 <= args.http_port <= 65535
    args.output_dir.mkdir(parents=True, exist_ok=False)
    candidate = args.candidate.resolve()
    validate_unavailable(candidate, args.source_fixture, args.output_dir / "unavailable")
    validate_online(candidate, args.source_fixture, args.output_dir / "online", args.http_port)
    results = {"candidateSha256": hashlib.sha256(candidate.read_bytes()).hexdigest(), "source": json.loads((args.source_fixture / "source.json").read_text()), "status": "passed", "claims": ["online-cli-does-not-initialize-unavailable-store", "online-three-read-only-scopes", "online-archive-only", "four-paused-manual-owners", "cli-page-request-competition", "no-startup-recovery", "no-manual-continuation", "graceful-lock-release"]}
    (args.output_dir / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(results), flush=True)


if __name__ == "__main__":
    main()
