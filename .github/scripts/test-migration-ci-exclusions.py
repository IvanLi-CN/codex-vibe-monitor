#!/usr/bin/env python3
"""Fixed, in-memory contract mutations; no builds, services, or suite execution."""
from __future__ import annotations

import copy
import importlib.util
import json
from pathlib import Path
import sys
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("migration_quality_contract", ROOT / ".github/scripts/check_quality_gates_contract.py")
assert SPEC is not None and SPEC.loader is not None
GATE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = GATE
SPEC.loader.exec_module(GATE)
CONTRACT = GATE.validate_quality_gates(json.loads((ROOT / ".github/quality-gates.json").read_text()))


def step(workflow, job, name):
    return next(item for item in workflow["jobs"][job]["steps"] if item.get("name") == name)


def check(workflow, validator, filename):
    with patch.object(GATE, "load_yaml", return_value=workflow):
        validator(ROOT / ".github/workflows" / filename, CONTRACT)


REJECTIONS = 0


def reject(source, mutate, validator, filename):
    global REJECTIONS
    workflow = copy.deepcopy(source)
    mutate(workflow)
    try:
        check(workflow, validator, filename)
    except GATE.ContractError:
        REJECTIONS += 1
        return
    raise AssertionError("unsafe migration CI exclusion was accepted")


# YAML is loaded only twice. All mutations are fixed-size Python objects.
PR = GATE.load_yaml(ROOT / ".github/workflows/ci-pr.yml")
PG = GATE.load_yaml(ROOT / ".github/workflows/ci-postgresql-acceptance.yml")
check(PR, GATE.validate_ci_pr, "ci-pr.yml")
check(PG, GATE.validate_postgres_functional_workflow, "ci-postgresql-acceptance.yml")
for job in (
    "frontend-tests", "build-pr-smoke-artifacts", "build", "backend-test-archive", "backend-tests-lightweight",
    "backend-tests-stateful-sqlite-shard-1", "backend-tests-stateful-sqlite-shard-2",
    "backend-tests-stateful-sqlite", "backend-tests-archive-file-io",
    "backend-tests-representative-scale", "retention-migration-validation",
):
    old_branch_scope = PR["jobs"][job]["if"].replace("github.event.pull_request.number != 1212", "github.head_ref != 'th/postgresql-migration-design'")
    for condition in ("${{ false }}", "always()", old_branch_scope):
        reject(PR, lambda w, j=job, c=condition: w["jobs"][j].update({"if": c}), GATE.validate_ci_pr, "ci-pr.yml")

retention = "Verify retention migration validation"
reject(PR, lambda w: step(w, "build", retention).update({"run": "exit 0"}), GATE.validate_ci_pr, "ci-pr.yml")
reject(PR, lambda w: step(w, "build", retention)["env"].update({"MIGRATION_PR_EXCLUSION": "${{ true }}"}), GATE.validate_ci_pr, "ci-pr.yml")
reject(PR, lambda w: step(w, "build", retention).update({"run": step(w, "build", retention)["run"].replace('test "$RETENTION_MIGRATION_RESULT" = success', 'test "$RETENTION_MIGRATION_RESULT" = skipped')}), GATE.validate_ci_pr, "ci-pr.yml")
e2e = "Run records overlay and Web Demo Playwright regression"
reject(PR, lambda w: step(w, "records-overlay-e2e-producer", e2e)["env"].update({"MIGRATION_PR_EXCLUSION": "${{ true }}"}), GATE.validate_ci_pr, "ci-pr.yml")
reject(PR, lambda w: step(w, "records-overlay-e2e-producer", e2e).update({"run": step(w, "records-overlay-e2e-producer", e2e)["run"].replace("exit $(( functional_status || performance_status ))", "exit 0")}), GATE.validate_ci_pr, "ci-pr.yml")
for name in ("Start performance Web Demo server", "Wait for performance Web Demo readiness"):
    reject(PR, lambda w, n=name: step(w, "records-overlay-e2e-producer", n).update({"if": "${{ false }}"}), GATE.validate_ci_pr, "ci-pr.yml")
for name, job in ((retention, "build"), (e2e, "records-overlay-e2e-producer")):
    old_branch_scope = step(PR, job, name)["env"]["MIGRATION_PR_EXCLUSION"].replace("github.event.pull_request.number == 1212", "github.head_ref == 'th/postgresql-migration-design'")
    reject(PR, lambda w, n=name, j=job, c=old_branch_scope: step(w, j, n)["env"].update({"MIGRATION_PR_EXCLUSION": c}), GATE.validate_ci_pr, "ci-pr.yml")
old_pg_scope = PG["jobs"]["postgres-migration-functional"]["if"].replace("github.event.pull_request.number != 1212", "github.head_ref != 'th/postgresql-migration-design'")
for condition in ("${{ false }}", "always()", old_pg_scope):
    reject(PG, lambda w, c=condition: w["jobs"]["postgres-migration-functional"].update({"if": c}), GATE.validate_postgres_functional_workflow, "ci-postgresql-acceptance.yml")
print(f"migration CI exact-exclusion contract: {REJECTIONS} mutations passed")
