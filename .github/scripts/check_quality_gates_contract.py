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
    required_checks: set[str]
    status_check_integrations: dict[str, int]
    review_check_name: str
    review_required_approvals: int
    review_exempt_permissions: set[str]
    review_allowed_permissions: set[str]
    expected_workflows: dict[str, tuple[str, ...]]
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


def require_string_set(value: Any, where: str) -> set[str]:
    require(isinstance(value, list), f"{where} must be a list")
    normalized: set[str] = set()
    for index, item in enumerate(value):
        require(isinstance(item, str) and item, f"{where}[{index}] must be a non-empty string")
        normalized.add(item)
    return normalized


def require_mapping(value: Any, where: str) -> dict[str, Any]:
    require(isinstance(value, dict), f"{where} must be an object")
    return value


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
    if config is None:
        return {}
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
    require(name in expected_jobs, f"{where}.jobs.{job_id}.name={name!r} must be declared in expected_pr_workflows")
    return job


def step_config(job: dict[str, Any], step_name: str, where: str) -> dict[str, Any]:
    steps = job.get("steps")
    require(isinstance(steps, list), f"{where}.steps must be a list")
    for step in steps:
        if isinstance(step, dict) and step.get("name") == step_name:
            return step
    raise ContractError(f"{where}: missing step {step_name!r}")


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


def require_command(
    step: dict[str, Any],
    prefix: list[str],
    where: str,
    message: str,
) -> list[str]:
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
    require(
        mapping.get("continue-on-error") in (None, False),
        f"{where}.continue-on-error must not ignore failures",
    )


def validate_quality_gates(payload: dict[str, Any]) -> ContractModel:
    policy = require_mapping(payload.get("policy"), "quality-gates.json.policy")
    branch_policy = require_mapping(policy.get("branch_protection"), "quality-gates.json.policy.branch_protection")
    review_policy = require_mapping(policy.get("review_policy"), "quality-gates.json.policy.review_policy")
    review_enforcement = require_mapping(
        review_policy.get("enforcement"), "quality-gates.json.policy.review_policy.enforcement"
    )

    require(payload.get("schema_version") == 1, "quality-gates.json: schema_version must be 1")
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
    require(
        status_check_policy.get("strict") is True,
        "quality-gates.json: required_status_checks.strict must be true",
    )
    integrations = require_mapping(
        status_check_policy.get("integrations"),
        "quality-gates.json.policy.branch_protection.required_status_checks.integrations",
    )
    required_checks = require_string_set(payload.get("required_checks"), "quality-gates.json.required_checks")
    require(
        set(integrations) == required_checks,
        f"quality-gates.json: required_status_checks.integrations drifted: {sorted(integrations)}",
    )
    normalized_integrations: dict[str, int] = {}
    for context, integration_id in integrations.items():
        require(
            isinstance(integration_id, int),
            f"quality-gates.json: required_status_checks.integrations[{context!r}] must be an integer",
        )
        normalized_integrations[context] = integration_id

    require(review_policy.get("mode") == "conditional-required", "quality-gates.json: review_policy.mode drifted")
    review_required_approvals = review_policy.get("required_approvals")
    require(
        isinstance(review_required_approvals, int)
        and not isinstance(review_required_approvals, bool)
        and review_required_approvals >= 1,
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
    require(
        isinstance(review_check_name, str) and review_check_name,
        "quality-gates.json: enforcement.check_name must be a non-empty string",
    )
    require(review_check_name in required_checks, "quality-gates.json: enforcement.check_name must be required")

    raw_expected_workflows = payload.get("expected_pr_workflows")
    require(
        isinstance(raw_expected_workflows, list) and raw_expected_workflows,
        "quality-gates.json: expected_pr_workflows must be a non-empty array",
    )
    expected_workflows: dict[str, tuple[str, ...]] = {}
    declared_job_names: set[str] = set()
    for index, raw_entry in enumerate(raw_expected_workflows):
        entry = require_mapping(raw_entry, f"quality-gates.json.expected_pr_workflows[{index}]")
        workflow_name = entry.get("workflow")
        require(
            isinstance(workflow_name, str) and workflow_name,
            f"quality-gates.json.expected_pr_workflows[{index}].workflow must be a non-empty string",
        )
        jobs = require_string_set(
            entry.get("jobs"),
            f"quality-gates.json.expected_pr_workflows[{index}].jobs",
        )
        require(
            workflow_name not in expected_workflows,
            f"quality-gates.json: duplicate expected_pr_workflows entry {workflow_name!r}",
        )
        expected_workflows[workflow_name] = tuple(sorted(jobs))
        declared_job_names.update(jobs)

    label_workflow_jobs = set(expected_workflows.get("Label Gate", ()))
    require(label_workflow_jobs, "quality-gates.json: expected_pr_workflows must declare Label Gate jobs")
    label_required_checks = sorted(required_checks & label_workflow_jobs)
    require(
        len(label_required_checks) == 1,
        "quality-gates.json: Label Gate must expose exactly one required check",
    )

    waivers = payload.get("waivers", [])
    require(isinstance(waivers, list), "quality-gates.json: waivers must be an array")
    require(len(waivers) == 1, "quality-gates.json: only the bypass-actors-unverified waiver is allowed")
    bypass_waivers = [
        waiver
        for waiver in waivers
        if isinstance(waiver, dict)
        and waiver.get("kind") == "bypass-actors-unverified"
        and waiver.get("branch") == "main"
    ]
    require(len(bypass_waivers) == 1, "quality-gates.json: missing bypass-actors-unverified waiver for main")
    require(
        isinstance(bypass_waivers[0].get("reason"), str) and bypass_waivers[0]["reason"],
        "quality-gates.json: bypass-actors-unverified waiver must include a reason",
    )
    return ContractModel(
        required_checks=required_checks,
        status_check_integrations=normalized_integrations,
        review_check_name=review_check_name,
        review_required_approvals=review_required_approvals,
        review_exempt_permissions=review_exempt_permissions,
        review_allowed_permissions=review_allowed_permissions,
        expected_workflows=expected_workflows,
        label_check_name=label_required_checks[0],
    )


def validate_metadata_policy(module: Any, contract: ContractModel) -> None:
    require(
        getattr(module, "REVIEW_REQUIRED_APPROVALS", None) == contract.review_required_approvals,
        "metadata_gate.REVIEW_REQUIRED_APPROVALS drifted from quality-gates.json",
    )
    require(
        require_string_collection(
            getattr(module, "REVIEW_EXEMPT_PERMISSIONS", None),
            "metadata_gate.REVIEW_EXEMPT_PERMISSIONS",
        )
        == contract.review_exempt_permissions,
        "metadata_gate.REVIEW_EXEMPT_PERMISSIONS drifted from quality-gates.json",
    )
    require(
        require_string_collection(
            getattr(module, "REVIEW_ALLOWED_PERMISSIONS", None),
            "metadata_gate.REVIEW_ALLOWED_PERMISSIONS",
        )
        == contract.review_allowed_permissions,
        "metadata_gate.REVIEW_ALLOWED_PERMISSIONS drifted from quality-gates.json",
    )


def validate_ci(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "ci.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_workflows.get(workflow_name, ()))
    require(expected_jobs, f"ci.yml: workflow {workflow_name!r} must be declared in expected_pr_workflows")
    push_config = event_config(workflow, "push", "ci.yml")
    assert_event_branches(push_config, {"main"}, "ci.yml.on.push")
    pull_request_config = event_config(workflow, "pull_request", "ci.yml")
    assert_event_branches(pull_request_config, {"main"}, "ci.yml.on.pull_request")
    assert_event_types(pull_request_config, CI_PULL_REQUEST_TYPES, "ci.yml.on.pull_request")
    merge_group_config = event_config(workflow, "merge_group", "ci.yml")
    assert_event_types(merge_group_config, {"checks_requested"}, "ci.yml.on.merge_group")

    permissions = require_mapping(workflow.get("permissions"), "ci.yml.permissions")
    require(permissions.get("contents") == "read", "ci.yml.permissions.contents must stay read")
    require("statuses" not in permissions, "ci.yml.permissions.statuses must stay unset")

    lint_job = named_job_config(workflow, "lint", expected_jobs, "ci.yml")
    require_no_if(lint_job, "ci.yml.jobs.lint")
    require_fail_closed(lint_job, "ci.yml.jobs.lint")
    checkout = uses_step_config(lint_job, "Checkout", "actions/checkout@v4", "ci.yml.jobs.lint")
    checkout_with = require_mapping(checkout.get("with"), "ci.yml.jobs.lint.steps['Checkout'].with")
    require(checkout_with.get("fetch-depth") == 0, "ci.yml.jobs.lint Checkout must fetch full history for trusted source resolution")
    check_scripts = step_config(lint_job, "Check quality-gates scripts", "ci.yml.jobs.lint")
    require_no_if(check_scripts, "ci.yml.jobs.lint.steps['Check quality-gates scripts']")
    require_fail_closed(check_scripts, "ci.yml.jobs.lint.steps['Check quality-gates scripts']")
    check_scripts_run = str(check_scripts.get("run", ""))
    require(".github/scripts/check_live_quality_gates.py" in check_scripts_run, "ci.yml.jobs.lint: script check step drifted")
    trusted_step = step_config(lint_job, "Resolve trusted quality-gates sources", "ci.yml.jobs.lint")
    require_no_if(trusted_step, "ci.yml.jobs.lint.steps['Resolve trusted quality-gates sources']")
    require_fail_closed(trusted_step, "ci.yml.jobs.lint.steps['Resolve trusted quality-gates sources']")
    trusted_run = str(trusted_step.get("run", ""))
    require("base_branch=" in trusted_run, "ci.yml.jobs.lint: trusted-source base_branch resolution drifted")
    require("git fetch --no-tags --depth=1 origin" in trusted_run, "ci.yml.jobs.lint: trusted-source fetch drifted")
    require("git show \"${source_ref}:${path}\"" in trusted_run, "ci.yml.jobs.lint: trusted-source materialization drifted")
    require(
        "bootstrap-current-branch" not in trusted_run and "using current branch for bootstrap rollout only" not in trusted_run,
        "ci.yml.jobs.lint: bootstrap fallback must stay disabled",
    )
    require(
        'elif [ "${{ github.event_name }}" = "merge_group" ]; then' in trusted_run,
        "ci.yml.jobs.lint: merge_group trusted-source branch handling drifted",
    )
    require(
        'queue_prefix="refs/heads/gh-readonly-queue/"' in trusted_run,
        "ci.yml.jobs.lint: merge_group queue ref parsing drifted",
    )
    require(
        'source_kind="merge-group-base-branch"' in trusted_run,
        "ci.yml.jobs.lint: merge_group trusted source kind drifted",
    )
    require(
        "trusted quality-gates sources required for" in trusted_run,
        "ci.yml.jobs.lint: merge_group trusted-source fail-closed guard drifted",
    )
    require("trusted_root/.github/scripts/check_quality_gates_contract.py" in trusted_run, "ci.yml.jobs.lint: trusted-source outputs drifted")
    contract_step = step_config(lint_job, "Quality-gates contract check", "ci.yml.jobs.lint")
    require_no_if(contract_step, "ci.yml.jobs.lint.steps['Quality-gates contract check']")
    require_fail_closed(contract_step, "ci.yml.jobs.lint.steps['Quality-gates contract check']")
    contract_run = str(contract_step.get("run", ""))
    require(
        'steps.trusted-quality-gates.outputs.contract_script' in contract_run
        and 'steps.trusted-quality-gates.outputs.declaration' in contract_run
        and 'steps.trusted-quality-gates.outputs.metadata_script' in contract_run,
        "ci.yml.jobs.lint: contract check must use trusted quality-gates sources",
    )
    live_step = step_config(lint_job, "Quality-gates live rules check", "ci.yml.jobs.lint")
    require_no_if(live_step, "ci.yml.jobs.lint.steps['Quality-gates live rules check']")
    require_fail_closed(live_step, "ci.yml.jobs.lint.steps['Quality-gates live rules check']")
    live_env = require_mapping(live_step.get("env"), "ci.yml.jobs.lint.steps['Quality-gates live rules check'].env")
    require(live_env.get("QUALITY_GATES_LIVE_RULES_MODE") == "require", "ci.yml.jobs.lint: live rules mode must stay require")
    live_run = str(live_step.get("run", ""))
    require(
        'steps.trusted-quality-gates.outputs.live_script' in live_run
        and '--declaration ".github/quality-gates.json"' in live_run,
        "ci.yml.jobs.lint: live rules step must use the trusted checker against the candidate declaration",
    )
    self_tests = step_config(lint_job, "Quality gates self-tests", "ci.yml.jobs.lint")
    require_no_if(self_tests, "ci.yml.jobs.lint.steps['Quality gates self-tests']")
    require_fail_closed(self_tests, "ci.yml.jobs.lint.steps['Quality gates self-tests']")
    run = str(self_tests.get("run", ""))
    require(
        "test-quality-gates-contract.sh" in run and "test-live-quality-gates.sh" in run,
        "ci.yml.jobs.lint: self-tests step drifted",
    )


def validate_label_gate(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "label-gate.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_workflows.get(workflow_name, ()))
    require(expected_jobs, f"label-gate.yml: workflow {workflow_name!r} must be declared in expected_pr_workflows")
    on_section = require_mapping(mapping_get(workflow, "on"), "label-gate.yml.on")
    require("merge_group" not in on_section, "label-gate.yml: merge_group must stay disabled")
    require("workflow_dispatch" not in on_section, "label-gate.yml: workflow_dispatch must stay disabled")
    require("pull_request" not in on_section, "label-gate.yml: pull_request must stay disabled")
    pull_request_target_config = event_config(workflow, "pull_request_target", "label-gate.yml")
    assert_event_branches(pull_request_target_config, {"main"}, "label-gate.yml.on.pull_request_target")
    assert_event_types(pull_request_target_config, LABEL_GATE_PULL_REQUEST_TYPES, "label-gate.yml.on.pull_request_target")

    permissions = require_mapping(workflow.get("permissions"), "label-gate.yml.permissions")
    require(permissions.get("contents") == "read", "label-gate.yml.permissions.contents must stay read")
    require(permissions.get("pull-requests") == "read", "label-gate.yml.permissions.pull-requests must stay read")
    require(permissions.get("issues") == "read", "label-gate.yml.permissions.issues must stay read")
    require("checks" not in permissions, "label-gate.yml.permissions.checks must stay unset")
    concurrency = require_mapping(workflow.get("concurrency"), "label-gate.yml.concurrency")
    require(
        concurrency.get("group") == "label-gate-${{ github.event.pull_request.number || github.run_id }}",
        "label-gate.yml.concurrency.group drifted",
    )
    require(concurrency.get("cancel-in-progress") is True, "label-gate.yml.concurrency.cancel-in-progress must stay true")

    job = named_job_config(workflow, "validate-pr-labels", expected_jobs, "label-gate.yml")
    require(job.get("name") == contract.label_check_name, "label-gate.yml: required label check name drifted")
    require_exact_if(
        job,
        "${{ github.event.pull_request.base.ref == 'main' }}",
        "label-gate.yml.jobs.validate-pr-labels",
    )
    require_fail_closed(job, "label-gate.yml.jobs.validate-pr-labels")
    trusted_checkout = checkout_step(job, "Checkout trusted base", "label-gate.yml.jobs.validate-pr-labels")
    require(
        trusted_checkout.get("ref") == "${{ github.event.pull_request.base.ref }}",
        "label-gate.yml: trusted base checkout ref drifted",
    )
    require(trusted_checkout.get("fetch-depth") == 1, "label-gate.yml: trusted base checkout must stay shallow")
    require(trusted_checkout.get("path") == "trusted", "label-gate.yml: trusted base checkout path drifted")
    require(
        trusted_checkout.get("persist-credentials") is False,
        "label-gate.yml: trusted base checkout must disable persisted credentials",
    )
    candidate_checkout = checkout_step(job, "Checkout candidate pull request", "label-gate.yml.jobs.validate-pr-labels")
    require(
        candidate_checkout.get("repository") == "${{ github.event.pull_request.head.repo.full_name }}",
        "label-gate.yml: candidate checkout repository drifted",
    )
    require(
        candidate_checkout.get("ref") == "${{ github.event.pull_request.head.sha }}",
        "label-gate.yml: candidate checkout ref drifted",
    )
    require(candidate_checkout.get("fetch-depth") == 1, "label-gate.yml: candidate checkout must stay shallow")
    require(candidate_checkout.get("path") == "candidate", "label-gate.yml: candidate checkout path drifted")
    require(
        candidate_checkout.get("persist-credentials") is False,
        "label-gate.yml: candidate checkout must disable persisted credentials",
    )
    contract_step = step_config(job, "Validate trusted label-gate contract", "label-gate.yml.jobs.validate-pr-labels")
    require_no_if(contract_step, "label-gate.yml.jobs.validate-pr-labels.steps['Validate trusted label-gate contract']")
    require_fail_closed(contract_step, "label-gate.yml.jobs.validate-pr-labels.steps['Validate trusted label-gate contract']")
    contract_command = require_command(
        contract_step,
        ["python3"],
        "label-gate.yml.jobs.validate-pr-labels.steps['Validate trusted label-gate contract']",
        "label-gate.yml: trusted label gate must invoke the trusted contract checker",
    )
    require(
        contract_command[1] == "trusted/.github/scripts/check_quality_gates_contract.py",
        "label-gate.yml: trusted label gate must run the trusted contract checker",
    )
    contract_options = command_option_map(
        contract_command[2:],
        "label-gate.yml: trusted label gate contract step",
    )
    require(
        contract_options.get("--repo-root") == "$PWD/candidate",
        "label-gate.yml: trusted label gate must validate the candidate checkout",
    )
    require(
        contract_options.get("--declaration") == "$PWD/candidate/.github/quality-gates.json",
        "label-gate.yml: trusted label gate declaration source drifted",
    )
    require(
        contract_options.get("--metadata-script") == "$PWD/trusted/.github/scripts/metadata_gate.py",
        "label-gate.yml: trusted label gate metadata script source drifted",
    )

    label_step = step_config(job, "Evaluate PR labels", "label-gate.yml.jobs.validate-pr-labels")
    require_no_if(label_step, "label-gate.yml.jobs.validate-pr-labels.steps['Evaluate PR labels']")
    require_fail_closed(label_step, "label-gate.yml.jobs.validate-pr-labels.steps['Evaluate PR labels']")
    label_env = require_mapping(
        label_step.get("env"),
        "label-gate.yml.jobs.validate-pr-labels.steps['Evaluate PR labels'].env",
    )
    require(
        label_env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}",
        "label-gate.yml: Evaluate PR labels must pass GITHUB_TOKEN via env",
    )
    label_command = require_command(
        label_step,
        ["python3"],
        "label-gate.yml.jobs.validate-pr-labels.steps['Evaluate PR labels']",
        "label-gate.yml: Evaluate PR labels must invoke the trusted metadata gate",
    )
    require(
        label_command[1] == "trusted/.github/scripts/metadata_gate.py" and label_command[2:] == ["label"],
        "label-gate.yml: Evaluate PR labels must execute the trusted metadata gate in label mode",
    )


def validate_review_policy(path: Path, contract: ContractModel) -> None:
    workflow = load_yaml(path)
    workflow_name = workflow.get("name")
    require(isinstance(workflow_name, str) and workflow_name, "review-policy.yml: workflow name must stay non-empty")
    expected_jobs = set(contract.expected_workflows.get(workflow_name, ()))
    require(expected_jobs, f"review-policy.yml: workflow {workflow_name!r} must be declared in expected_pr_workflows")
    on_section = require_mapping(mapping_get(workflow, "on"), "review-policy.yml.on")
    require("merge_group" not in on_section, "review-policy.yml: merge_group must stay disabled")
    require("workflow_dispatch" not in on_section, "review-policy.yml: workflow_dispatch must stay disabled")
    require("pull_request_target" not in on_section, "review-policy.yml: pull_request_target must stay disabled")
    pull_request_config = event_config(workflow, "pull_request", "review-policy.yml")
    assert_event_branches(pull_request_config, {"main"}, "review-policy.yml.on.pull_request")
    assert_event_types(pull_request_config, REVIEW_POLICY_PULL_REQUEST_TYPES, "review-policy.yml.on.pull_request")
    pull_request_review_config = event_config(workflow, "pull_request_review", "review-policy.yml")
    assert_event_types(pull_request_review_config, REVIEW_POLICY_REVIEW_TYPES, "review-policy.yml.on.pull_request_review")

    permissions = require_mapping(workflow.get("permissions"), "review-policy.yml.permissions")
    require(permissions.get("contents") == "read", "review-policy.yml.permissions.contents must stay read")
    require(permissions.get("pull-requests") == "read", "review-policy.yml.permissions.pull-requests must stay read")
    require("statuses" not in permissions, "review-policy.yml.permissions.statuses must stay unset")

    job = named_job_config(workflow, "review-policy", expected_jobs, "review-policy.yml")
    require_exact_if(
        job,
        "${{ github.event.pull_request.base.ref == 'main' }}",
        "review-policy.yml.jobs.review-policy",
    )
    require_fail_closed(job, "review-policy.yml.jobs.review-policy")
    checkout = checkout_step(job, "Checkout", "review-policy.yml.jobs.review-policy")
    require(checkout.get("fetch-depth") == 0, "review-policy.yml: Checkout must fetch full history")

    trusted_step = step_config(job, "Resolve trusted quality-gates sources", "review-policy.yml.jobs.review-policy")
    require_no_if(
        trusted_step,
        "review-policy.yml.jobs.review-policy.steps['Resolve trusted quality-gates sources']",
    )
    require_fail_closed(
        trusted_step,
        "review-policy.yml.jobs.review-policy.steps['Resolve trusted quality-gates sources']",
    )
    trusted_run = str(trusted_step.get("run", ""))
    require("git fetch --no-tags --depth=1 origin" in trusted_run, "review-policy.yml: trusted-source fetch drifted")
    require(
        "bootstrap-current-branch" not in trusted_run and "using current branch for bootstrap rollout only" not in trusted_run,
        "review-policy.yml: bootstrap fallback must stay disabled",
    )
    require("metadata_script=$trusted_root/.github/scripts/metadata_gate.py" in trusted_run, "review-policy.yml: metadata trusted-source output drifted")

    step = step_config(job, "Evaluate review policy", "review-policy.yml.jobs.review-policy")
    require_no_if(step, "review-policy.yml.jobs.review-policy.steps['Evaluate review policy']")
    require_fail_closed(step, "review-policy.yml.jobs.review-policy.steps['Evaluate review policy']")
    env = require_mapping(step.get("env"), "review-policy.yml.jobs.review-policy.steps['Evaluate review policy'].env")
    require(env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}", "review-policy.yml: review gate must pass GITHUB_TOKEN via env")
    metadata_command = require_command(
        step,
        ["python3"],
        "review-policy.yml.jobs.review-policy.steps['Evaluate review policy']",
        "review-policy.yml: Evaluate review policy must invoke trusted metadata gate",
    )
    require(
        "${{ steps.trusted-quality-gates.outputs.metadata_script }}" in metadata_command[1]
        and metadata_command[2:] == ["review"],
        "review-policy.yml: Evaluate review policy must execute the trusted metadata gate in review mode",
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
    shutil.copyfile(fixtures_root / "ci.yml", tempdir / ".github" / "workflows" / "ci.yml")
    shutil.copyfile(fixtures_root / "label-gate.yml", tempdir / ".github" / "workflows" / "label-gate.yml")
    shutil.copyfile(fixtures_root / "review-policy.yml", tempdir / ".github" / "workflows" / "review-policy.yml")
    return tempdir


def main() -> int:
    args = parse_args()
    script_repo_root = Path(__file__).resolve().parents[2]
    temp_repo_root: Path | None = None
    if args.repo_root:
        repo_root = Path(args.repo_root).resolve()
    else:
        temp_repo_root = materialize_default_repo_root(script_repo_root)
        repo_root = temp_repo_root
    scripts_dir = repo_root / ".github" / "scripts"
    declaration_path = Path(args.declaration).resolve() if args.declaration else repo_root / ".github" / "quality-gates.json"
    metadata_script_path = Path(args.metadata_script).resolve() if args.metadata_script else script_repo_root / ".github" / "scripts" / "metadata_gate.py"

    try:
        declaration = json.loads(declaration_path.read_text())
        require(isinstance(declaration, dict), "quality-gates.json must decode to an object")
        module = load_module(metadata_script_path)
        contract = validate_quality_gates(declaration)
        validate_metadata_policy(module, contract)
        validate_ci(repo_root / ".github" / "workflows" / "ci.yml", contract)
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
