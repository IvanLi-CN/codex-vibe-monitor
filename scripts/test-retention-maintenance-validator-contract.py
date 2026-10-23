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


if __name__ == "__main__":
    test_archive_and_other_owner_results_use_public_result_field()
    test_real_owner_result_rejects_internal_or_legacy_fields()
    test_missing_maintenance_store_is_not_created_for_diagnostics()
    print("test-retention-maintenance-validator-contract: all checks passed")
