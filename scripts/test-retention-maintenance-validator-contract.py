#!/usr/bin/env python3
"""Pure contract checks for retention owner validator output fields."""
import importlib.util
import json
import threading
from unittest.mock import patch
from pathlib import Path
import sys
import tempfile


SOURCE = Path(__file__).with_name("validate-retention-maintenance-upgrade.py")
SPEC = importlib.util.spec_from_file_location("retention_maintenance_upgrade", SOURCE)
UPGRADE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = UPGRADE
SPEC.loader.exec_module(UPGRADE)


def owner_details(task_key, result):
    return {
        "ownershipVersion": 2,
        "ownerScope": task_key,
        "result": result,
    }


def test_archive_and_other_owner_results_use_public_result_field():
    archive = owner_details("retention_archive", "complete")
    other = owner_details("pool_upstream_attempt_archive", "partial")

    assert UPGRADE.assert_real_owner_result(archive, "retention_archive") == "complete"
    assert UPGRADE.assert_real_owner_result(other, "pool_upstream_attempt_archive") == "partial"


def test_real_owner_result_rejects_internal_or_legacy_fields():
    for details in [
        {"completion": "complete"},
        {"result": "completed"},
        {"result": "deferred"},
        {"result": "failed"},
    ]:
        try:
            UPGRADE.assert_real_owner_result(details, "retention_archive")
        except AssertionError:
            continue
        raise AssertionError(f"accepted invalid owner result payload: {details}")


def test_missing_maintenance_store_is_not_created_for_diagnostics():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        assert UPGRADE.read_recorded_task_result(root) is None
        assert not (root / "maintenance.sqlite").exists()


def test_dry_run_snapshot_covers_every_seeded_owner_business_table():
    expected = {
        "pool_upstream_request_attempts",
        "forward_proxy_attempts",
        "codex_quota_snapshots",
        "pool_upstream_accounts",
        "pool_upstream_account_model_routes",
        "system_task_runs",
    }
    assert expected.issubset(UPGRADE.BUSINESS_FACT_TABLES)


def test_relative_output_paths_are_canonicalized_before_validation():
    assert UPGRADE.resolve_output_dir(Path("relative-results")).is_absolute()


def test_version_scheduler_bounds_overlap_and_preserves_evidence():
    versions = ["v-first", "v-second", "v-third"]
    barrier = threading.Barrier(2)
    active = 0
    maximum = 0
    lock = threading.Lock()
    seen = []
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        candidate = root / "candidate"
        candidate.write_bytes(b"candidate")

        def validate(binary, source, target):
            nonlocal active, maximum
            assert binary == candidate.resolve()
            assert source == root / source.name
            assert target == root / source.name
            with lock:
                active += 1
                maximum = max(maximum, active)
                seen.append(source.name)
            if source.name != "v-third":
                barrier.wait(timeout=5)
            with lock:
                active -= 1
            return {"source": {"tag": source.name}, "status": "passed", "claims": ["unchanged"]}

        with patch.object(UPGRADE, "VERSIONS", versions), patch.object(UPGRADE, "validate_source", validate):
            UPGRADE.validate_versions(candidate, root, root, 2)
        assert maximum == 2
        assert sorted(seen) == sorted(versions)
        results = json.loads((root / "results.json").read_text())
        assert [item["source"]["tag"] for item in results] == versions
        assert all(item["claims"] == ["unchanged"] for item in results)
        assert all(item["candidateSha256"] == json.loads((root / "candidate.json").read_text())["candidateSha256"] for item in results)


def test_version_failure_is_not_hidden_and_success_evidence_survives():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        candidate = root / "candidate"
        candidate.write_bytes(b"candidate")

        def validate(binary, source, target):
            if source.name == "bad":
                raise RuntimeError("injected version failure")
            return {"source": {"tag": source.name}, "status": "passed"}

        with patch.object(UPGRADE, "VERSIONS", ["bad", "good"]), patch.object(UPGRADE, "validate_source", validate):
            try:
                UPGRADE.validate_versions(candidate, root, root, 2)
            except RuntimeError as error:
                assert str(error) == "injected version failure"
            else:
                raise AssertionError("version failure was swallowed")
        results = json.loads((root / "results.json").read_text())
        assert [item["source"]["tag"] for item in results] == ["good"]


def test_jobs_are_bounded_and_serial_fallback_is_available():
    import argparse
    for value in ["0", "3", "-1"]:
        try:
            UPGRADE.positive_jobs(value)
        except argparse.ArgumentTypeError:
            pass
        else:
            raise AssertionError(f"unbounded jobs accepted: {value}")
    assert UPGRADE.positive_jobs("1") == 1
    assert UPGRADE.positive_jobs("2") == 2


if __name__ == "__main__":
    test_archive_and_other_owner_results_use_public_result_field()
    test_real_owner_result_rejects_internal_or_legacy_fields()
    test_missing_maintenance_store_is_not_created_for_diagnostics()
    test_dry_run_snapshot_covers_every_seeded_owner_business_table()
    test_relative_output_paths_are_canonicalized_before_validation()
    test_version_scheduler_bounds_overlap_and_preserves_evidence()
    test_version_failure_is_not_hidden_and_success_evidence_survives()
    test_jobs_are_bounded_and_serial_fallback_is_available()
    print("test-retention-maintenance-validator-contract: all checks passed")
