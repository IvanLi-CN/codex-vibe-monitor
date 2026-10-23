#!/usr/bin/env python3
"""Pure contract checks for retention owner validator output fields."""
import importlib.util
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


if __name__ == "__main__":
    test_archive_and_other_owner_results_use_public_result_field()
    test_real_owner_result_rejects_internal_or_legacy_fields()
    test_missing_maintenance_store_is_not_created_for_diagnostics()
    test_dry_run_snapshot_covers_every_seeded_owner_business_table()
    test_relative_output_paths_are_canonicalized_before_validation()
    print("test-retention-maintenance-validator-contract: all checks passed")
