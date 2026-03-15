#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib.util
import json
import shlex
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any


class ContractError(RuntimeError):
    pass


@dataclass(frozen=True)
class ContractModel:
    implementation_profile: str
    required_checks: set[str]
    informational_checks: set[str]
    status_check_integrations: dict[str, int]
    review_check_name: str
    review_required_approvals: int
    review_exempt_permissions: set[str]
    review_allowed_permissions: set[str]
    expected_pr_workflows: dict[str, tuple[str, ...]]
    expected_main_workflows: dict[str, tuple[str, ...]]
    expected_release_workflows: dict[str, tuple[str, ...]]
    label_check_name: str


CI_PULL_REQUEST_TYPES = {"opened", "reopened", "synchronize", "ready_for_review", "edited"}
LABEL_GATE_PULL_REQUEST_TYPES = {
    "opened",
    "reopened",
    "synchronize",
    "labeled",
    "unlabeled",
    "ready_for_review",
    "edited",
}
REVIEW_POLICY_PULL_REQUEST_TYPES = {"opened", "reopened", "synchronize", "ready_for_review", "edited"}
REVIEW_POLICY_REVIEW_TYPES = {"submitted", "dismissed", "edited"}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate the codex-vibe-monitor quality-gates contract.")
    parser.add_argument(
        "--repo-root",
        default="",
        help="Repository root containing .github/workflows and the candidate quality-gates files.",
    )
    parser.add_argument(
        "--declaration",
        default="",
        help="Optional trusted quality-gates declaration path. Defaults to <repo-root>/.github/quality-gates.json.",
    )
    parser.add_argument(
        "--metadata-script",
        default="",
        help="Optional trusted metadata_gate.py path. Defaults to <repo-root>/.github/scripts/metadata_gate.py.",
    )
    parser.add_argument(
        "--profile",
        choices=("auto", "bootstrap", "final"),
        default="auto",
        help="Contract profile to validate. Defaults to auto-detect, or final when no repo-root is provided.",
    )
    return parser.parse_args()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("metadata_gate", path)
    if spec is None or spec.loader is None:
        raise ContractError(f"Unable to load module from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def load_yaml(path: Path) -> dict[str, Any]:
    ruby = (
        "require 'json'; "
        "require 'psych'; "
        "path = ARGV.fetch(0); "
        "data = Psych.safe_load("
        "File.read(path), "
        "permitted_classes: [], "
        "permitted_symbols: [], "
        "aliases: false, "
        "filename: path"
        "); "
        "print JSON.generate(data)"
    )
    result = subprocess.run(
        ["ruby", "-e", ruby, str(path)],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise ContractError(f"{path.name}: unable to parse YAML via ruby: {result.stderr.strip()}")
    payload = json.loads(result.stdout)
    if not isinstance(payload, dict):
        raise ContractError(f"{path.name}: workflow YAML must decode to an object")
    return payload


def mapping_get(mapping: dict[str, Any], key: str, default: Any = None) -> Any:
    if key in mapping:
        return mapping[key]
    if key == "on" and True in mapping:
        return mapping[True]
    if key == "on" and "true" in mapping:
        return mapping["true"]
    return default


def require_mapping(value: Any, where: str) -> dict[str, Any]:
    require(isinstance(value, dict), f"{where} must be an object")
    return value


def require_string_set(value: Any, where: str) -> set[str]:
    require(isinstance(value, list), f"{where} must be a list")
    normalized: set[str] = set()
    for index, item in enumerate(value):
        require(isinstance(item, str) and item, f"{where}[{index}] must be a non-empty string")
        normalized.add(item)
    return normalized


def require_string_collection(value: Any, where: str) -> set[str]:
    require(isinstance(value, (list, tuple, set, frozenset)), f"{where} must be a string collection")
    normalized: set[str] = set()
    for index, item in enumerate(value):
        require(isinstance(item, str) and item, f"{where}[{index}] must be a non-empty string")
        normalized.add(item)
    return normalized


def event_config(workflow: dict[str, Any], event_name: str, where: str) -> dict[str, Any]:
    on_section = require_mapping(mapping_get(workflow, "on"), f"{where}.on")
    config = mapping_get(on_section, event_name)
    require(config is not None, f"{where}.on.{event_name} must be configured")
    if isinstance(config, dict):
        return config
    raise ContractError(f"{where}.on.{event_name} must be an object")


def assert_event_branches(config: dict[str, Any], expected: set[str], where: str) -> None:
    branches = require_string_set(config.get("branches"), f"{where}.branches")
    require(branches == expected, f"{where}.branches drifted: {sorted(branches)}")


def assert_event_types(config: dict[str, Any], expected: set[str], where: str) -> None:
    types = require_string_set(config.get("types"), f"{where}.types")
    require(types == expected, f"{where}.types drifted: {sorted(types)}")


def workflow_jobs(workflow: dict[str, Any], where: str) -> dict[str, Any]:
    return require_mapping(workflow.get("jobs"), f"{where}.jobs")


def job_config(workflow: dict[str, Any], job_id: str, where: str) -> dict[str, Any]:
    jobs = workflow_jobs(workflow, where)
    return require_mapping(jobs.get(job_id), f"{where}.jobs.{job_id}")


def named_job_config(workflow: dict[str, Any], job_id: str, expected_jobs: set[str], where: str) -> dict[str, Any]:
    job = job_config(workflow, job_id, where)
    name = job.get("name")
    require(isinstance(name, str) and name, f"{where}.jobs.{job_id}.name must be a non-empty string")
    require(name in expected_jobs, f"{where}.jobs.{job_id}.name={name!r} must be declared in the contract")
    return job


def workflow_named_job_names(workflow: dict[str, Any], where: str) -> set[str]:
    names: set[str] = set()
    for job_id, raw_job in workflow_jobs(workflow, where).items():
        job = require_mapping(raw_job, f"{where}.jobs.{job_id}")
        name = job.get("name")
        require(isinstance(name, str) and name, f"{where}.jobs.{job_id}.name must be a non-empty string")
        names.add(name)
    return names


def require_exact_named_jobs(workflow: dict[str, Any], expected_jobs: set[str], where: str) -> None:
    actual_jobs = workflow_named_job_names(workflow, where)
    require(
        actual_jobs == expected_jobs,
        f"{where}: declared jobs drifted missing={sorted(expected_jobs - actual_jobs)} unexpected={sorted(actual_jobs - expected_jobs)}",
    )


def step_config(job: dict[str, Any], step_name: str, where: str) -> dict[str, Any]:
    steps = job.get("steps")
    require(isinstance(steps, list), f"{where}.steps must be a list")
    for step in steps:
        if isinstance(step, dict) and step.get("name") == step_name:
            return step
    raise ContractError(f"{where}: missing step {step_name!r}")


def maybe_step_config(job: dict[str, Any], step_name: str) -> dict[str, Any] | None:
    steps = job.get("steps")
    require(isinstance(steps, list), "job.steps must be a list")
    for step in steps:
        if isinstance(step, dict) and step.get("name") == step_name:
            return step
    return None


def uses_step_config(job: dict[str, Any], step_name: str, expected_uses: str, where: str) -> dict[str, Any]:
    step = step_config(job, step_name, where)
    require(step.get("uses") == expected_uses, f"{where}.steps[{step_name!r}].uses must stay {expected_uses!r}")
    return step


def step_run(step: dict[str, Any], where: str) -> str:
    run = step.get("run")
    require(isinstance(run, str) and run.strip(), f"{where}.run must be a non-empty string")
    return run


def shell_commands(step: dict[str, Any], where: str) -> list[list[str]]:
    commands: list[list[str]] = []
    current = ""
    for raw_line in step_run(step, where).splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if current:
            current = f"{current} {line}"
        else:
            current = line
        if current.endswith("\\"):
            current = current[:-1].rstrip()
            continue
        try:
            tokens = shlex.split(current, posix=True)
        except ValueError as exc:
            raise ContractError(f"{where}.run contains invalid shell syntax: {exc}") from exc
        if tokens:
            commands.append(tokens)
        current = ""
    require(not current, f"{where}.run ends with an unterminated shell continuation")
    require(commands, f"{where}.run must contain at least one shell command")
    return commands


def require_command(step: dict[str, Any], prefix: list[str], where: str, message: str) -> list[str]:
    for command in shell_commands(step, where):
        if command[: len(prefix)] == prefix:
            return command
    raise ContractError(message)


def command_option_map(command: list[str], where: str) -> dict[str, str]:
    options: dict[str, str] = {}
    index = 0
    while index < len(command):
        token = command[index]
        if not token.startswith("--"):
            index += 1
            continue
        require(index + 1 < len(command), f"{where}: option {token} is missing a value")
        options[token] = command[index + 1]
        index += 2
    return options


def checkout_step(job: dict[str, Any], step_name: str, where: str) -> dict[str, Any]:
    step = uses_step_config(job, step_name, "actions/checkout@v4", where)
    return require_mapping(step.get("with"), f"{where}.steps[{step_name!r}].with")


def require_no_if(mapping: dict[str, Any], where: str) -> None:
    require("if" not in mapping, f"{where}.if must stay unset")


def require_exact_if(mapping: dict[str, Any], expected: str, where: str) -> None:
    require(mapping.get("if") == expected, f"{where}.if must stay {expected!r}")


def require_fail_closed(mapping: dict[str, Any], where: str) -> None:
    require(mapping.get("continue-on-error") in (None, False), f"{where}.continue-on-error must not ignore failures")


def parse_expected_workflows(payload: dict[str, Any], key: str) -> dict[str, tuple[str, ...]]:
    raw_expected = payload.get(key)
    require(isinstance(raw_expected, list) and raw_expected, f"quality-gates.json: {key} must be a non-empty array")
    expected: dict[str, tuple[str, ...]] = {}
    for index, raw_entry in enumerate(raw_expected):
        entry = require_mapping(raw_entry, f"quality-gates.json.{key}[{index}]")
        workflow_name = entry.get("workflow")
        require(
            isinstance(workflow_name, str) and workflow_name,
            f"quality-gates.json.{key}[{index}].workflow must be a non-empty string",
        )
        jobs = require_string_set(entry.get("jobs"), f"quality-gates.json.{key}[{index}].jobs")
        require(workflow_name not in expected, f"quality-gates.json: duplicate {key} entry {workflow_name!r}")
        expected[workflow_name] = tuple(sorted(jobs))
    return expected


def validate_quality_gates(payload: dict[str, Any]) -> ContractModel:
    policy = require_mapping(payload.get("policy"), "quality-gates.json.policy")
    branch_policy = require_mapping(policy.get("branch_protection"), "quality-gates.json.policy.branch_protection")
    review_policy = require_mapping(policy.get("review_policy"), "quality-gates.json.policy.review_policy")
    review_enforcement = require_mapping(review_policy.get("enforcement"), "quality-gates.json.policy.review_policy.enforcement")

    require(payload.get("schema_version") == 1, "quality-gates.json: schema_version must be 1")
    implementation_profile = payload.get("implementation_profile")
    require(implementation_profile in {"final", "bootstrap"}, "quality-gates.json: implementation_profile must be 'bootstrap' or 'final'")
    require(policy.get("baseline_policy") == "explicit-waiver-required", "quality-gates.json: baseline_policy drifted")
    require(policy.get("require_signed_commits") is True, "quality-gates.json: require_signed_commits must be true")
    require(branch_policy.get("protected_branches") == ["main"], "quality-gates.json: protected_branches drifted")
    require(branch_policy.get("require_pull_request") is True, "quality-gates.json: require_pull_request must be true")
    require(branch_policy.get("disallow_direct_pushes") is True, "quality-gates.json: disallow_direct_pushes must be true")
    require(branch_policy.get("disallow_branch_deletions") is True, "quality-gates.json: disallow_branch_deletions must be true")
    require(branch_policy.get("disallow_force_pushes") is True, "quality-gates.json: disallow_force_pushes must be true")
    require(branch_policy.get("allow_merge_commits") is True, "quality-gates.json: allow_merge_commits must be true")
    require(branch_policy.get("require_merge_queue") is False, "quality-gates.json: require_merge_queue must be false")
    require(branch_policy.get("required_reviewers") == [], "quality-gates.json: required_reviewers must stay empty")

    status_check_policy = require_mapping(
        branch_policy.get("required_status_checks"),
        "quality-gates.json.policy.branch_protection.required_status_checks",
    )
    require(status_check_policy.get("strict") is True, "quality-gates.json: required_status_checks.strict must be true")
    integrations = require_mapping(
        status_check_policy.get("integrations"),
        "quality-gates.json.policy.branch_protection.required_status_checks.integrations",
    )
    required_checks = require_string_set(payload.get("required_checks"), "quality-gates.json.required_checks")
    informational_checks = require_string_set(payload.get("informational_checks"), "quality-gates.json.informational_checks")
    require(required_checks.isdisjoint(informational_checks), "quality-gates.json: required_checks and informational_checks must be disjoint")
    require(set(integrations) == required_checks, f"quality-gates.json: required_status_checks.integrations drifted: {sorted(integrations)}")
    normalized_integrations: dict[str, int] = {}
    for context, integration_id in integrations.items():
        require(isinstance(integration_id, int), f"quality-gates.json: required_status_checks.integrations[{context!r}] must be an integer")
        normalized_integrations[context] = integration_id

    require(review_policy.get("mode") == "conditional-required", "quality-gates.json: review_policy.mode drifted")
    review_required_approvals = review_policy.get("required_approvals")
    require(
        isinstance(review_required_approvals, int) and not isinstance(review_required_approvals, bool) and review_required_approvals >= 1,
        "quality-gates.json: review_policy.required_approvals must be a positive integer",
    )
    require(review_policy.get("exempt_repository_owner") is True, "quality-gates.json: exempt_repository_owner must be true")
    review_exempt_permissions = require_string_set(
        review_policy.get("exempt_author_permissions"),
        "quality-gates.json.policy.review_policy.exempt_author_permissions",
    )
    review_allowed_permissions = require_string_set(
        review_policy.get("allowed_reviewer_permissions"),
        "quality-gates.json.policy.review_policy.allowed_reviewer_permissions",
    )
    require(review_enforcement.get("mode") == "required-check", "quality-gates.json: enforcement.mode drifted")
    review_check_name = review_enforcement.get("check_name")
    require(isinstance(review_check_name, str) and review_check_name, "quality-gates.json: enforcement.check_name must be a non-empty string")
    require(review_check_name in required_checks, "quality-gates.json: enforcement.check_name must be required")

    expected_pr_workflows = parse_expected_workflows(payload, "expected_pr_workflows")
    expected_main_workflows = parse_expected_workflows(payload, "expected_main_workflows")
    expected_release_workflows = parse_expected_workflows(payload, "expected_release_workflows")

    declared_pr_jobs = {job for jobs in expected_pr_workflows.values() for job in jobs}
    require(
        declared_pr_jobs == (required_checks | informational_checks),
        "quality-gates.json: expected_pr_workflows jobs must exactly cover required_checks + informational_checks",
    )

    label_jobs = set(expected_pr_workflows.get("Label Gate", ()))
    require(label_jobs, "quality-gates.json: expected_pr_workflows must declare Label Gate jobs")
    label_required = sorted(required_checks & label_jobs)
    require(len(label_required) == 1, "quality-gates.json: Label Gate must expose exactly one required check")

    waivers = payload.get("waivers", [])
    require(isinstance(waivers, list), "quality-gates.json: waivers must be an array")
    for index, waiver in enumerate(waivers):
        entry = require_mapping(waiver, f"quality-gates.json.waivers[{index}]")
        require(entry.get("kind") == "bypass-actors-unverified", "quality-gates.json: only bypass-actors-unverified waivers are allowed")
        require(entry.get("branch") == "main", "quality-gates.json: waivers must target main")
        require(isinstance(entry.get("reason"), str) and entry["reason"], "quality-gates.json: waivers must include a non-empty reason")

    return ContractModel(
        implementation_profile=implementation_profile,
        required_checks=required_checks,
        informational_checks=informational_checks,
        status_check_integrations=normalized_integrations,
        review_check_name=review_check_name,
        review_required_approvals=review_required_approvals,
        review_exempt_permissions=review_exempt_permissions,
        review_allowed_permissions=review_allowed_permissions,
        expected_pr_workflows=expected_pr_workflows,
        expected_main_workflows=expected_main_workflows,
        expected_release_workflows=expected_release_workflows,
        label_check_name=label_required[0],
    )


def validate_metadata_policy(module: Any, contract: ContractModel) -> None:
    require(getattr(module, "REVIEW_REQUIRED_APPROVALS", None) == contract.review_required_approvals, "metadata_gate.REVIEW_REQUIRED_APPROVALS drifted from quality-gates.json")
    require(
        require_string_collection(getattr(module, "REVIEW_EXEMPT_PERMISSIONS", None), "metadata_gate.REVIEW_EXEMPT_PERMISSIONS")
        == contract.review_exempt_permissions,
        "metadata_gate.REVIEW_EXEMPT_PERMISSIONS drifted from quality-gates.json",
    )
    require(
        require_string_collection(getattr(module, "REVIEW_ALLOWED_PERMISSIONS", None), "metadata_gate.REVIEW_ALLOWED_PERMISSIONS")
        == contract.review_allowed_permissions,
        "metadata_gate.REVIEW_ALLOWED_PERMISSIONS drifted from quality-gates.json",
    )


def validate_ci_pr(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "ci-pr.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_pr_workflows.get(workflow_name, ()))
    require(expected_jobs, f"ci-pr.yml: workflow {workflow_name!r} must be declared in expected_pr_workflows")
    require_exact_named_jobs(workflow, expected_jobs, "ci-pr.yml")

    on_section = require_mapping(mapping_get(workflow, "on"), "ci-pr.yml.on")
    require("push" not in on_section, "ci-pr.yml: push must stay disabled")
    require("workflow_dispatch" not in on_section, "ci-pr.yml: workflow_dispatch must stay disabled")
    pull_request_config = event_config(workflow, "pull_request", "ci-pr.yml")
    assert_event_branches(pull_request_config, {"main"}, "ci-pr.yml.on.pull_request")
    assert_event_types(pull_request_config, CI_PULL_REQUEST_TYPES, "ci-pr.yml.on.pull_request")
    merge_group_config = event_config(workflow, "merge_group", "ci-pr.yml")
    assert_event_types(merge_group_config, {"checks_requested"}, "ci-pr.yml.on.merge_group")

    concurrency = require_mapping(workflow.get("concurrency"), "ci-pr.yml.concurrency")
    require(concurrency.get("group") == "ci-pr-${{ github.event.pull_request.number || github.ref }}", "ci-pr.yml.concurrency.group drifted")
    require(concurrency.get("cancel-in-progress") is True, "ci-pr.yml.concurrency.cancel-in-progress must stay true")

    permissions = require_mapping(workflow.get("permissions"), "ci-pr.yml.permissions")
    require(permissions.get("contents") == "read", "ci-pr.yml.permissions.contents must stay read")
    require("statuses" not in permissions, "ci-pr.yml.permissions.statuses must stay unset")

    lint_job = named_job_config(workflow, "lint", expected_jobs, "ci-pr.yml")
    require_no_if(lint_job, "ci-pr.yml.jobs.lint")
    require_fail_closed(lint_job, "ci-pr.yml.jobs.lint")
    checkout = checkout_step(lint_job, "Checkout", "ci-pr.yml.jobs.lint")
    require(checkout.get("fetch-depth") == 0, "ci-pr.yml.jobs.lint Checkout must fetch full history for trusted source resolution")
    trusted_step = step_config(lint_job, "Resolve trusted quality-gates sources", "ci-pr.yml.jobs.lint")
    trusted_run = str(trusted_step.get("run", ""))
    require('elif [ "${{ github.event_name }}" = "merge_group" ]; then' in trusted_run, "ci-pr.yml.jobs.lint: merge_group trusted-source branch handling drifted")
    require('queue_prefix="refs/heads/gh-readonly-queue/"' in trusted_run, "ci-pr.yml.jobs.lint: merge_group queue ref parsing drifted")
    require('supports_final_topology="true"' in trusted_run, "ci-pr.yml.jobs.lint: rollout support flag drifted")
    require('source_kind="merge-group-base-branch"' in trusted_run, "ci-pr.yml.jobs.lint: merge_group trusted source kind drifted")
    require(
        "keeping trusted scripts pinned to base and skipping trusted final-topology checks during rollout" in trusted_run,
        "ci-pr.yml.jobs.lint: rollout warning drifted",
    )

    contract_step = step_config(lint_job, "Quality-gates contract check", "ci-pr.yml.jobs.lint")
    require(
        contract_step.get("if") == "steps.trusted-quality-gates.outputs.supports_final_topology == 'true'",
        "ci-pr.yml.jobs.lint: contract check rollout gate drifted",
    )
    contract_run = str(contract_step.get("run", ""))
    require('steps.trusted-quality-gates.outputs.contract_script' in contract_run, "ci-pr.yml.jobs.lint: contract check must use trusted sources")

    live_step = step_config(lint_job, "Quality-gates live rules check", "ci-pr.yml.jobs.lint")
    require(
        live_step.get("if") == "steps.trusted-quality-gates.outputs.supports_final_topology == 'true'",
        "ci-pr.yml.jobs.lint: live rules rollout gate drifted",
    )
    live_env = require_mapping(live_step.get("env"), "ci-pr.yml.jobs.lint.steps['Quality-gates live rules check'].env")
    require(live_env.get("QUALITY_GATES_LIVE_RULES_MODE") == "require", "ci-pr.yml.jobs.lint: live rules mode must stay require")

    self_tests = step_config(lint_job, "Quality gates self-tests", "ci-pr.yml.jobs.lint")
    self_tests_run = str(self_tests.get("run", ""))
    require("test-quality-gates-contract.sh" in self_tests_run and "test-live-quality-gates.sh" in self_tests_run, "ci-pr.yml.jobs.lint: self-tests step drifted")

    build_job = named_job_config(workflow, "build", expected_jobs, "ci-pr.yml")
    require_exact_if(build_job, "github.event_name == 'pull_request'", "ci-pr.yml.jobs.build")


def validate_ci_main(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "ci-main.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_main_workflows.get(workflow_name, ()))
    require(expected_jobs, f"ci-main.yml: workflow {workflow_name!r} must be declared in expected_main_workflows")
    require_exact_named_jobs(workflow, expected_jobs, "ci-main.yml")

    on_section = require_mapping(mapping_get(workflow, "on"), "ci-main.yml.on")
    require("pull_request" not in on_section, "ci-main.yml: pull_request must stay disabled")
    require("merge_group" not in on_section, "ci-main.yml: merge_group must stay disabled")
    require("workflow_dispatch" not in on_section, "ci-main.yml: workflow_dispatch must stay disabled")
    push_config = event_config(workflow, "push", "ci-main.yml")
    assert_event_branches(push_config, {"main"}, "ci-main.yml.on.push")

    concurrency = require_mapping(workflow.get("concurrency"), "ci-main.yml.concurrency")
    require(
        concurrency.get("group") in {"ci-main-${{ github.sha }}", "ci-main-main"},
        "ci-main.yml.concurrency.group drifted",
    )
    require(concurrency.get("cancel-in-progress") is False, "ci-main.yml.concurrency.cancel-in-progress must stay false")

    permissions = require_mapping(workflow.get("permissions"), "ci-main.yml.permissions")
    require(permissions.get("contents") == "read", "ci-main.yml.permissions.contents must stay read")

    lint_job = named_job_config(workflow, "lint", expected_jobs, "ci-main.yml")
    require_no_if(lint_job, "ci-main.yml.jobs.lint")
    require_fail_closed(lint_job, "ci-main.yml.jobs.lint")
    trusted_step = step_config(lint_job, "Resolve trusted quality-gates sources", "ci-main.yml.jobs.lint")
    trusted_run = str(trusted_step.get("run", ""))
    require('source_ref="HEAD"' in trusted_run, "ci-main.yml.jobs.lint: trusted-source ref drifted")
    require('source_kind="current-branch"' in trusted_run, "ci-main.yml.jobs.lint: trusted-source kind drifted")
    require("cp \"$path\" \"$trusted_root/$path\"" in trusted_run, "ci-main.yml.jobs.lint: trusted-source copy drifted")

    release_snapshot = named_job_config(workflow, "release-snapshot", expected_jobs, "ci-main.yml")
    require(
        release_snapshot.get("needs") == ["lint", "frontend-tests", "records-overlay-e2e", "unit-tests"],
        "ci-main.yml.jobs.release-snapshot.needs drifted",
    )
    release_snapshot_permissions = require_mapping(
        release_snapshot.get("permissions"), "ci-main.yml.jobs.release-snapshot.permissions"
    )
    require(
        release_snapshot_permissions.get("actions") == "read",
        "ci-main.yml.jobs.release-snapshot.permissions.actions must stay read",
    )
    require(
        release_snapshot_permissions.get("contents") == "write",
        "ci-main.yml.jobs.release-snapshot.permissions.contents must stay write",
    )
    require(
        release_snapshot_permissions.get("issues") == "read",
        "ci-main.yml.jobs.release-snapshot.permissions.issues must stay read",
    )
    require(
        release_snapshot_permissions.get("pull-requests") == "read",
        "ci-main.yml.jobs.release-snapshot.permissions.pull-requests must stay read",
    )
    ensure_step = step_config(
        release_snapshot, "Ensure immutable release snapshot", "ci-main.yml.jobs.release-snapshot"
    )
    ensure_run = str(ensure_step.get("run", ""))
    require(
        "release_snapshot.py ensure" in ensure_run,
        "ci-main.yml.jobs.release-snapshot: snapshot writer must use release_snapshot.py ensure",
    )
    require(
        "RELEASE_SNAPSHOT_NOTES_REF" in ensure_run,
        "ci-main.yml.jobs.release-snapshot: notes-ref plumbing drifted",
    )
    target_only = "--target-only" in ensure_run
    require(
        target_only or "TARGET_SHA" not in ensure_run,
        "ci-main.yml.jobs.release-snapshot: automatic snapshot ensure must not use TARGET_SHA without --target-only",
    )


def validate_label_gate(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "label-gate.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_pr_workflows.get(workflow_name, ()))
    require(expected_jobs, f"label-gate.yml: workflow {workflow_name!r} must be declared in expected_pr_workflows")
    require_exact_named_jobs(workflow, expected_jobs, "label-gate.yml")

    on_section = require_mapping(mapping_get(workflow, "on"), "label-gate.yml.on")
    require("merge_group" not in on_section, "label-gate.yml: merge_group must stay disabled")
    require("workflow_dispatch" not in on_section, "label-gate.yml: workflow_dispatch must stay disabled")
    require("pull_request_target" not in on_section, "label-gate.yml: pull_request_target must stay disabled")
    pull_request_config = event_config(workflow, "pull_request", "label-gate.yml")
    assert_event_branches(pull_request_config, {"main"}, "label-gate.yml.on.pull_request")
    assert_event_types(pull_request_config, LABEL_GATE_PULL_REQUEST_TYPES, "label-gate.yml.on.pull_request")

    permissions = require_mapping(workflow.get("permissions"), "label-gate.yml.permissions")
    require(permissions.get("contents") == "read", "label-gate.yml.permissions.contents must stay read")
    require(permissions.get("pull-requests") == "read", "label-gate.yml.permissions.pull-requests must stay read")
    require(permissions.get("issues") == "read", "label-gate.yml.permissions.issues must stay read")

    concurrency = require_mapping(workflow.get("concurrency"), "label-gate.yml.concurrency")
    require(concurrency.get("group") == "label-gate-${{ github.event.pull_request.number || github.run_id }}", "label-gate.yml.concurrency.group drifted")
    require(concurrency.get("cancel-in-progress") is True, "label-gate.yml.concurrency.cancel-in-progress must stay true")

    job = named_job_config(workflow, "validate-pr-labels", expected_jobs, "label-gate.yml")
    require(job.get("name") == contract.label_check_name, "label-gate.yml: required label check name drifted")
    require_exact_if(job, "${{ github.event.pull_request.base.ref == 'main' }}", "label-gate.yml.jobs.validate-pr-labels")
    require_fail_closed(job, "label-gate.yml.jobs.validate-pr-labels")

    trusted_checkout = checkout_step(job, "Checkout trusted base", "label-gate.yml.jobs.validate-pr-labels")
    require(trusted_checkout.get("ref") == "${{ github.event.pull_request.base.ref }}", "label-gate.yml: trusted checkout ref drifted")
    require(trusted_checkout.get("path") == "trusted", "label-gate.yml: trusted checkout path drifted")
    require(trusted_checkout.get("persist-credentials") is False, "label-gate.yml: trusted checkout must disable persisted credentials")

    candidate_checkout = checkout_step(job, "Checkout candidate pull request", "label-gate.yml.jobs.validate-pr-labels")
    require(candidate_checkout.get("repository") == "${{ github.event.pull_request.head.repo.full_name }}", "label-gate.yml: candidate checkout repository drifted")
    require(candidate_checkout.get("ref") == "${{ github.event.pull_request.head.sha }}", "label-gate.yml: candidate checkout ref drifted")
    require(candidate_checkout.get("path") == "candidate", "label-gate.yml: candidate checkout path drifted")
    require(candidate_checkout.get("persist-credentials") is False, "label-gate.yml: candidate checkout must disable persisted credentials")

    contract_step = step_config(job, "Validate trusted label-gate contract", "label-gate.yml.jobs.validate-pr-labels")
    contract_command = require_command(
        contract_step,
        ["python3", "trusted/.github/scripts/check_quality_gates_contract.py"],
        "label-gate.yml.jobs.validate-pr-labels.steps['Validate trusted label-gate contract']",
        "label-gate.yml: trusted label gate must invoke the trusted contract checker",
    )
    contract_options = command_option_map(contract_command[2:], "label-gate.yml: trusted label gate contract step")
    require(contract_options.get("--repo-root") == "$PWD/candidate", "label-gate.yml: trusted label gate must validate the candidate checkout")
    require(contract_options.get("--declaration") == "$PWD/candidate/.github/quality-gates.json", "label-gate.yml: trusted label gate declaration drifted")
    require(contract_options.get("--metadata-script") == "$PWD/trusted/.github/scripts/metadata_gate.py", "label-gate.yml: trusted label gate metadata script drifted")

    label_step = step_config(job, "Evaluate PR labels", "label-gate.yml.jobs.validate-pr-labels")
    label_env = require_mapping(label_step.get("env"), "label-gate.yml.jobs.validate-pr-labels.steps['Evaluate PR labels'].env")
    require(label_env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}", "label-gate.yml: Evaluate PR labels must pass GITHUB_TOKEN via env")
    label_command = require_command(
        label_step,
        ["python3", "trusted/.github/scripts/metadata_gate.py", "label"],
        "label-gate.yml.jobs.validate-pr-labels.steps['Evaluate PR labels']",
        "label-gate.yml: Evaluate PR labels must execute the trusted metadata gate in label mode",
    )
    require(
        label_command[:3] == ["python3", "trusted/.github/scripts/metadata_gate.py", "label"],
        "label-gate.yml: label gate command drifted",
    )
    label_options = command_option_map(label_command[3:], "label-gate.yml: label gate command options")
    require(
        "--write-intent" not in label_options,
        "label-gate.yml: Evaluate PR labels must validate labels before any release-intent write step",
    )

    rollout_step = maybe_step_config(job, "Detect rollout contract support")
    trusted_write_step = maybe_step_config(job, "Write trusted release intent artifact")
    upload_step = maybe_step_config(job, "Upload release intent artifact")
    if rollout_step is not None:
        rollout_run = str(rollout_step.get("run", ""))
        require(
            "supports_final_contract=false" in rollout_run and "supports_final_contract=true" in rollout_run,
            "label-gate.yml: rollout support detection outputs drifted",
        )
        require(
            "skipping trusted contract validation during rollout" in rollout_run,
            "label-gate.yml: rollout warning drifted",
        )
        require(
            contract_step.get("if") == "steps.rollout.outputs.supports_final_contract == 'true'",
            "label-gate.yml: trusted contract validation gate drifted",
        )
        require(trusted_write_step is not None, "label-gate.yml: rollout topology must retain trusted release intent write step")
        require(upload_step is not None, "label-gate.yml: rollout topology must retain release intent upload step")
    else:
        require("if" not in contract_step, "label-gate.yml: simplified trusted contract step must run unconditionally")
        require(trusted_write_step is None, "label-gate.yml: simplified topology must not write release intent artifacts")
        require(upload_step is None, "label-gate.yml: simplified topology must not upload release intent artifacts")

    if trusted_write_step is not None:
        require(
            trusted_write_step.get("if") == "steps.rollout.outputs.supports_final_contract == 'true'",
            "label-gate.yml: trusted release intent write gate drifted",
        )
        trusted_write_env = require_mapping(
            trusted_write_step.get("env"),
            "label-gate.yml.jobs.validate-pr-labels.steps['Write trusted release intent artifact'].env",
        )
        require(
            trusted_write_env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}",
            "label-gate.yml: trusted release intent write step must pass GITHUB_TOKEN via env",
        )
        trusted_write_command = require_command(
            trusted_write_step,
            ["python3", "trusted/.github/scripts/metadata_gate.py", "label"],
            "label-gate.yml.jobs.validate-pr-labels.steps['Write trusted release intent artifact']",
            "label-gate.yml: trusted release intent write step must invoke the trusted metadata gate in label mode",
        )
        trusted_write_options = command_option_map(
            trusted_write_command[3:],
            "label-gate.yml: trusted release intent write command options",
        )
        require(
            trusted_write_options.get("--write-intent") == "$RUNNER_TEMP/release-intent.json",
            "label-gate.yml: trusted release intent write step must write to the runner temp artifact path",
        )

    if upload_step is not None:
        require(
            upload_step.get("if") == "steps.rollout.outputs.supports_final_contract == 'true'",
            "label-gate.yml: release intent artifact upload gate drifted",
        )
        uses_value = str(upload_step.get("uses") or "")
        require(
            uses_value == "actions/upload-artifact@v4",
            "label-gate.yml: release intent artifact step must use actions/upload-artifact@v4",
        )
        upload_with = require_mapping(upload_step.get("with"), "label-gate.yml.jobs.validate-pr-labels.steps['Upload release intent artifact'].with")
        require(
            upload_with.get("name") == "release-intent-pr-${{ github.event.pull_request.number }}-${{ github.event.pull_request.head.sha }}",
            "label-gate.yml: release intent artifact name drifted",
        )
        require(
            upload_with.get("path") == "${{ runner.temp }}/release-intent.json",
            "label-gate.yml: release intent artifact path drifted",
        )
        require(
            upload_with.get("retention-days") == 30,
            "label-gate.yml: release intent artifact retention must stay 30 days",
        )
        require(
            upload_with.get("if-no-files-found") == "error",
            "label-gate.yml: release intent artifact must fail when the JSON is missing",
        )


def validate_review_policy(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "review-policy.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_pr_workflows.get(workflow_name, ()))
    require(expected_jobs, f"review-policy.yml: workflow {workflow_name!r} must be declared in expected_pr_workflows")
    require_exact_named_jobs(workflow, expected_jobs, "review-policy.yml")

    on_section = require_mapping(mapping_get(workflow, "on"), "review-policy.yml.on")
    require("merge_group" not in on_section, "review-policy.yml: merge_group must stay disabled")
    require("workflow_dispatch" not in on_section, "review-policy.yml: workflow_dispatch must stay disabled")
    require("pull_request_target" not in on_section, "review-policy.yml: pull_request_target must stay disabled")
    pull_request_config = event_config(workflow, "pull_request", "review-policy.yml")
    assert_event_branches(pull_request_config, {"main"}, "review-policy.yml.on.pull_request")
    assert_event_types(pull_request_config, REVIEW_POLICY_PULL_REQUEST_TYPES, "review-policy.yml.on.pull_request")
    review_config = event_config(workflow, "pull_request_review", "review-policy.yml")
    assert_event_types(review_config, REVIEW_POLICY_REVIEW_TYPES, "review-policy.yml.on.pull_request_review")

    permissions = require_mapping(workflow.get("permissions"), "review-policy.yml.permissions")
    require(permissions.get("contents") == "read", "review-policy.yml.permissions.contents must stay read")
    require(permissions.get("pull-requests") == "read", "review-policy.yml.permissions.pull-requests must stay read")

    concurrency = require_mapping(workflow.get("concurrency"), "review-policy.yml.concurrency")
    require(concurrency.get("group") == "review-policy-${{ github.event.pull_request.number || github.run_id }}", "review-policy.yml.concurrency.group drifted")
    require(concurrency.get("cancel-in-progress") is True, "review-policy.yml.concurrency.cancel-in-progress must stay true")

    job = named_job_config(workflow, "review-policy", expected_jobs, "review-policy.yml")
    require(job.get("name") == contract.review_check_name, "review-policy.yml: review check name drifted")
    require_exact_if(job, "${{ github.event.pull_request.base.ref == 'main' }}", "review-policy.yml.jobs.review-policy")
    require_fail_closed(job, "review-policy.yml.jobs.review-policy")

    trusted_step = step_config(job, "Resolve trusted quality-gates sources", "review-policy.yml.jobs.review-policy")
    trusted_run = str(trusted_step.get("run", ""))
    require("git fetch --no-tags --depth=1 origin" in trusted_run, "review-policy.yml: trusted-source fetch drifted")
    require("metadata_script=$trusted_root/.github/scripts/metadata_gate.py" in trusted_run, "review-policy.yml: metadata trusted-source output drifted")

    step = step_config(job, "Evaluate review policy", "review-policy.yml.jobs.review-policy")
    env = require_mapping(step.get("env"), "review-policy.yml.jobs.review-policy.steps['Evaluate review policy'].env")
    require(env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}", "review-policy.yml: review gate must pass GITHUB_TOKEN via env")
    require_command(
        step,
        ["python3", "${{ steps.trusted-quality-gates.outputs.metadata_script }}", "review"],
        "review-policy.yml.jobs.review-policy.steps['Evaluate review policy']",
        "review-policy.yml: Evaluate review policy must execute the trusted metadata gate in review mode",
    )


def validate_release(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "release.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_release_workflows.get(workflow_name, ()))
    require(expected_jobs, f"release.yml: workflow {workflow_name!r} must be declared in expected_release_workflows")
    require_exact_named_jobs(workflow, expected_jobs, "release.yml")

    on_section = require_mapping(mapping_get(workflow, "on"), "release.yml.on")
    workflow_run_config = event_config(workflow, "workflow_run", "release.yml")
    assert_event_types(workflow_run_config, {"completed"}, "release.yml.on.workflow_run")
    assert_event_branches(workflow_run_config, {"main"}, "release.yml.on.workflow_run")
    require(workflow_run_config.get("workflows") == ["CI Main"], "release.yml.on.workflow_run.workflows drifted")
    workflow_dispatch_config = event_config(workflow, "workflow_dispatch", "release.yml")
    inputs = require_mapping(workflow_dispatch_config.get("inputs"), "release.yml.on.workflow_dispatch.inputs")
    commit_sha = require_mapping(inputs.get("commit_sha"), "release.yml.on.workflow_dispatch.inputs.commit_sha")
    require(commit_sha.get("required") is True, "release.yml: workflow_dispatch.commit_sha must stay required")
    require(commit_sha.get("type") == "string", "release.yml: workflow_dispatch.commit_sha must stay string")

    concurrency = require_mapping(workflow.get("concurrency"), "release.yml.concurrency")
    require(
        concurrency.get("group")
        in {
            "release-${{ github.event_name == 'workflow_dispatch' && inputs.commit_sha || github.event.workflow_run.head_sha }}",
            "release-main",
        },
        "release.yml.concurrency.group drifted",
    )
    require(concurrency.get("cancel-in-progress") is False, "release.yml.concurrency.cancel-in-progress must stay false")

    permissions = require_mapping(workflow.get("permissions"), "release.yml.permissions")
    require(permissions.get("contents") == "read", "release.yml.permissions.contents must stay read")

    release_meta = named_job_config(workflow, "release-meta", expected_jobs, "release.yml")
    require_exact_if(
        release_meta,
        "${{ github.event_name == 'workflow_dispatch' || (github.event_name == 'workflow_run' && github.event.workflow_run.conclusion == 'success') }}",
        "release.yml.jobs.release-meta",
    )
    release_meta_permissions = require_mapping(release_meta.get("permissions"), "release.yml.jobs.release-meta.permissions")
    require(
        release_meta_permissions.get("actions") == "read",
        "release.yml.jobs.release-meta.permissions.actions must stay read",
    )
    require(
        release_meta_permissions.get("contents") == "read",
        "release.yml.jobs.release-meta.permissions.contents must stay read",
    )
    outputs = require_mapping(release_meta.get("outputs"), "release.yml.jobs.release-meta.outputs")
    require("target_sha" in outputs, "release.yml.jobs.release-meta.outputs.target_sha must be exported")

    requested_step = maybe_step_config(release_meta, "Resolve requested commit")
    legacy_target_step = maybe_step_config(release_meta, "Resolve target commit")
    require(
        (requested_step is None) != (legacy_target_step is None),
        "release.yml.jobs.release-meta must define exactly one target resolution step",
    )
    target_step_name = "Resolve requested commit" if requested_step is not None else "Resolve target commit"
    target_step = requested_step or legacy_target_step
    target_run = str(target_step.get("run", ""))
    require("inputs.commit_sha" in target_run, f"release.yml.jobs.release-meta: {target_step_name} manual commit_sha resolution drifted")
    require("git merge-base --is-ancestor" in target_run, f"release.yml.jobs.release-meta: {target_step_name} main ancestry gate drifted")

    backfill_step = step_config(release_meta, "Validate manual backfill target passed CI Main", "release.yml.jobs.release-meta")
    require(backfill_step.get("if") == "github.event_name == 'workflow_dispatch'", "release.yml.jobs.release-meta: manual backfill validation gate drifted")
    backfill_env = require_mapping(backfill_step.get("env"), "release.yml.jobs.release-meta.steps['Validate manual backfill target passed CI Main'].env")
    expected_backfill_target = "${{ steps.requested-target.outputs.target_sha }}" if requested_step is not None else "${{ steps.target.outputs.target_sha }}"
    require(backfill_env.get("TARGET_SHA") == expected_backfill_target, "release.yml.jobs.release-meta: manual backfill validation must consume target_sha")
    backfill_script = str(backfill_step.get("with", {}).get("script", ""))
    require("snapshot-only CI Main failure" in backfill_script, "release.yml.jobs.release-meta: snapshot-only backfill exception drifted")
    require("listJobsForWorkflowRun" in backfill_script, "release.yml.jobs.release-meta: snapshot-only backfill job inspection drifted")
    ensure_step = step_config(release_meta, "Ensure immutable release snapshot for manual backfill", "release.yml.jobs.release-meta")
    require(ensure_step.get("if") == "github.event_name == 'workflow_dispatch'", "release.yml.jobs.release-meta: manual snapshot ensure gate drifted")
    ensure_env = require_mapping(ensure_step.get("env"), "release.yml.jobs.release-meta.steps['Ensure immutable release snapshot for manual backfill'].env")
    require(ensure_env.get("TARGET_SHA") == expected_backfill_target, "release.yml.jobs.release-meta: manual snapshot ensure must consume target_sha")
    require(ensure_env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}", "release.yml.jobs.release-meta: manual snapshot ensure must use GITHUB_TOKEN")
    ensure_run = str(ensure_step.get("run", ""))
    require("release_snapshot.py ensure" in ensure_run, "release.yml.jobs.release-meta: manual snapshot ensure must use release_snapshot.py ensure")

    pending_step = maybe_step_config(release_meta, "Select pending release target")
    if pending_step is None:
        require("--allow-current-pr-label-fallback" in ensure_run, "release.yml.jobs.release-meta: legacy manual snapshot ensure must allow current PR label fallback")

    snapshot_step = step_config(release_meta, "Load immutable release snapshot", "release.yml.jobs.release-meta")
    snapshot_env = require_mapping(snapshot_step.get("env"), "release.yml.jobs.release-meta.steps['Load immutable release snapshot'].env")
    expected_snapshot_target = "${{ steps.pending-target.outputs.target_sha }}" if pending_step is not None else "${{ steps.target.outputs.target_sha }}"
    require(
        snapshot_env.get("TARGET_SHA") == expected_snapshot_target,
        "release.yml.jobs.release-meta: snapshot loader must consume target_sha",
    )
    if pending_step is not None:
        require(
            pending_step.get("id") == "pending-target",
            "release.yml.jobs.release-meta: pending release target step must expose pending-target outputs",
        )
        require(
            snapshot_step.get("if") == "steps.pending-target.outputs.target_sha != ''",
            "release.yml.jobs.release-meta: pending snapshot loader gate drifted",
        )
        pending_run = str(pending_step.get("run", ""))
        require("release_snapshot.py next-pending" in pending_run, "release.yml.jobs.release-meta: pending release selector must use release_snapshot.py next-pending")
    snapshot_run = str(snapshot_step.get("run", ""))
    require(
        "release_snapshot.py export" in snapshot_run,
        "release.yml.jobs.release-meta: snapshot loader must use release_snapshot.py export",
    )
    require(
        "RELEASE_SNAPSHOT_NOTES_REF" in snapshot_run,
        "release.yml.jobs.release-meta: snapshot notes-ref plumbing drifted",
    )
    candidate_step = step_config(release_meta, "Compute candidate suffix", "release.yml.jobs.release-meta")
    expected_candidate_if = (
        "steps.pending-target.outputs.target_sha != '' && steps.snapshot.outputs.release_enabled == 'true'"
        if pending_step is not None
        else "steps.snapshot.outputs.release_enabled == 'true'"
    )
    require(candidate_step.get("if") == expected_candidate_if, "release.yml.jobs.release-meta: candidate suffix gate drifted")
    candidate_run = str(candidate_step.get("run", ""))
    require("candidate_suffix=${TARGET_SHA:0:12}" in candidate_run, "release.yml.jobs.release-meta: candidate suffix drifted")

    docker_amd = named_job_config(workflow, "docker-amd64", expected_jobs, "release.yml")
    require(docker_amd.get("needs") == ["release-meta"], "release.yml.jobs.docker-amd64.needs drifted")
    require(docker_amd.get("if") == "needs.release-meta.outputs.release_enabled == 'true'", "release.yml.jobs.docker-amd64.if drifted")
    amd_checkout = checkout_step(docker_amd, "Checkout code", "release.yml.jobs.docker-amd64")
    require(amd_checkout.get("ref") == "${{ needs.release-meta.outputs.target_sha }}", "release.yml.jobs.docker-amd64 checkout ref drifted")
    amd_smoke_step = step_config(docker_amd, "Smoke test image (linux/amd64)", "release.yml.jobs.docker-amd64")
    amd_smoke_env = require_mapping(amd_smoke_step.get("env"), "release.yml.jobs.docker-amd64.steps['Smoke test image (linux/amd64)'].env")
    require(
        amd_smoke_env.get("SMOKE_TAG") == "${{ env.REGISTRY }}/${{ needs.release-meta.outputs.image_name_lower }}:smoke-${{ needs.release-meta.outputs.candidate_suffix }}-amd64",
        "release.yml.jobs.docker-amd64 smoke tag drifted",
    )

    docker_arm = named_job_config(workflow, "docker-arm64", expected_jobs, "release.yml")
    arm_checkout = checkout_step(docker_arm, "Checkout code", "release.yml.jobs.docker-arm64")
    require(arm_checkout.get("ref") == "${{ needs.release-meta.outputs.target_sha }}", "release.yml.jobs.docker-arm64 checkout ref drifted")
    arm_smoke_step = step_config(docker_arm, "Smoke test image (linux/arm64)", "release.yml.jobs.docker-arm64")
    arm_smoke_env = require_mapping(arm_smoke_step.get("env"), "release.yml.jobs.docker-arm64.steps['Smoke test image (linux/arm64)'].env")
    require(
        arm_smoke_env.get("SMOKE_TAG") == "${{ env.REGISTRY }}/${{ needs.release-meta.outputs.image_name_lower }}:smoke-${{ needs.release-meta.outputs.candidate_suffix }}-arm64",
        "release.yml.jobs.docker-arm64 smoke tag drifted",
    )

    publish = named_job_config(workflow, "release-publish", expected_jobs, "release.yml")
    require(publish.get("needs") == ["release-meta", "docker-amd64", "docker-arm64"], "release.yml.jobs.release-publish.needs drifted")
    publish_permissions = require_mapping(publish.get("permissions"), "release.yml.jobs.release-publish.permissions")
    require(publish_permissions.get("contents") == "write", "release.yml.jobs.release-publish.permissions.contents must stay write")
    publish_checkout = checkout_step(publish, "Checkout code", "release.yml.jobs.release-publish")
    require(publish_checkout.get("ref") == "${{ needs.release-meta.outputs.target_sha }}", "release.yml.jobs.release-publish checkout ref drifted")
    tag_step = step_config(publish, "Create and push git tag", "release.yml.jobs.release-publish")
    tag_run = str(tag_step.get("run", ""))
    require('sha="${TARGET_SHA}"' in tag_run, "release.yml.jobs.release-publish tag step must use target_sha")
    next_pending_step = maybe_step_config(publish, "Resolve next pending release target")
    continue_queue_step = maybe_step_config(publish, "Continue release queue")
    if next_pending_step is None:
        require(continue_queue_step is None, "release.yml.jobs.release-publish: queue continuation requires a next-pending resolver")
    else:
        require(continue_queue_step is not None, "release.yml.jobs.release-publish: pending queue topology must continue the release queue")
        require(
            publish_permissions.get("actions") == "write",
            "release.yml.jobs.release-publish.permissions.actions must stay write when continuing the queue",
        )
        next_pending_run = str(next_pending_step.get("run", ""))
        require("release_snapshot.py next-pending" in next_pending_run, "release.yml.jobs.release-publish: next pending resolver must use release_snapshot.py next-pending")
        require(
            continue_queue_step.get("uses") == "actions/github-script@v7",
            "release.yml.jobs.release-publish: continue queue step must use actions/github-script@v7",
        )


def validate_merge_group_helpers(module: Any) -> None:
    try:
        module.resolve_pull_numbers(
            module.GateContext(
                gate="label",
                owner="IvanLi-CN",
                repo="codex-vibe-monitor",
                api_root="https://api.github.com",
                token="",
                event_name="merge_group",
                event_payload={},
                manual_pull_number=None,
            ),
            module.GitHubClient("IvanLi-CN", "codex-vibe-monitor", "https://api.github.com", ""),
        )
    except module.GateError as exc:
        require("unsupported" in str(exc), f"metadata_gate: unexpected merge_group error {exc}")
    else:
        raise ContractError("metadata_gate: merge_group must fail closed")


def materialize_default_repo_root(script_repo_root: Path) -> Path:
    fixtures_root = script_repo_root / ".github" / "scripts" / "fixtures" / "quality-gates-contract"
    if not fixtures_root.is_dir():
        return script_repo_root

    tempdir = Path(tempfile.mkdtemp(prefix="quality-gates-contract-"))
    shutil.copytree(script_repo_root / ".github", tempdir / ".github", dirs_exist_ok=True)
    shutil.copyfile(fixtures_root / "quality-gates.json", tempdir / ".github" / "quality-gates.json")
    for filename in ("ci-pr.yml", "ci-main.yml", "release.yml", "label-gate.yml", "review-policy.yml"):
        shutil.copyfile(fixtures_root / filename, tempdir / ".github" / "workflows" / filename)
    return tempdir


def detect_profile(repo_root: Path) -> str:
    if (repo_root / ".github" / "workflows" / "ci-pr.yml").is_file():
        return "final"
    return "bootstrap"


def main() -> int:
    args = parse_args()
    script_repo_root = Path(__file__).resolve().parents[2]
    temp_repo_root: Path | None = None
    if args.repo_root:
        repo_root = Path(args.repo_root).resolve()
    else:
        temp_repo_root = materialize_default_repo_root(script_repo_root)
        repo_root = temp_repo_root
    declaration_path = Path(args.declaration).resolve() if args.declaration else repo_root / ".github" / "quality-gates.json"
    metadata_script_path = Path(args.metadata_script).resolve() if args.metadata_script else repo_root / ".github" / "scripts" / "metadata_gate.py"

    try:
        declaration = json.loads(declaration_path.read_text())
        require(isinstance(declaration, dict), "quality-gates.json must decode to an object")
        module = load_module(metadata_script_path)
        contract = validate_quality_gates(declaration)
        validate_metadata_policy(module, contract)
        profile = "final" if not args.repo_root and args.profile == "auto" else args.profile
        if profile == "auto":
            profile = detect_profile(repo_root)
        require(profile == contract.implementation_profile, f"quality-gates.json: implementation_profile={contract.implementation_profile!r} does not match workflow profile {profile!r}")
        require(profile == "final", "quality-gates-contract: bootstrap profile is no longer supported by this repository")
        validate_ci_pr(repo_root / ".github" / "workflows" / "ci-pr.yml", contract)
        validate_ci_main(repo_root / ".github" / "workflows" / "ci-main.yml", contract)
        validate_release(repo_root / ".github" / "workflows" / "release.yml", contract)
        validate_label_gate(repo_root / ".github" / "workflows" / "label-gate.yml", contract)
        validate_review_policy(repo_root / ".github" / "workflows" / "review-policy.yml", contract)
        validate_merge_group_helpers(module)
    except ContractError as exc:
        print(f"[quality-gates-contract] {exc}", file=sys.stderr)
        return 1
    finally:
        if temp_repo_root is not None:
            shutil.rmtree(temp_repo_root, ignore_errors=True)

    print("[quality-gates-contract] metadata workflow contract checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
