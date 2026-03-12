#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib.util
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


class ContractError(RuntimeError):
    pass


REQUIRED_CHECKS = {
    "Validate PR labels",
    "Lint & Format Check",
    "Backend Tests",
    "Build Artifacts",
    "Review Policy Gate",
}

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
        default=str(Path(__file__).resolve().parents[2]),
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
        "require 'yaml'; "
        "path = ARGV.fetch(0); "
        "data = YAML.load_file(path); "
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


def event_config(workflow: dict[str, Any], event_name: str, where: str) -> dict[str, Any]:
    on_section = require_mapping(mapping_get(workflow, "on"), f"{where}.on")
    require("pull_request_target" not in on_section, f"{where}: pull_request_target must stay disabled")
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


def job_config(workflow: dict[str, Any], job_id: str, expected_name: str, where: str) -> dict[str, Any]:
    jobs = workflow_jobs(workflow, where)
    job = require_mapping(jobs.get(job_id), f"{where}.jobs.{job_id}")
    require(job.get("name") == expected_name, f"{where}.jobs.{job_id}.name must stay {expected_name!r}")
    return job


def step_config(job: dict[str, Any], step_name: str, where: str) -> dict[str, Any]:
    steps = job.get("steps")
    require(isinstance(steps, list), f"{where}.steps must be a list")
    for step in steps:
        if isinstance(step, dict) and step.get("name") == step_name:
            return step
    raise ContractError(f"{where}: missing step {step_name!r}")


def require_no_checkout(job: dict[str, Any], where: str) -> None:
    steps = job.get("steps")
    require(isinstance(steps, list), f"{where}.steps must be a list")
    for index, step in enumerate(steps):
        if not isinstance(step, dict):
            continue
        uses = step.get("uses")
        require(uses != "actions/checkout@v4", f"{where}.steps[{index}] must not use actions/checkout@v4")


def script_body(step: dict[str, Any], where: str) -> str:
    require(step.get("uses") == "actions/github-script@v8", f"{where}.uses must stay actions/github-script@v8")
    with_config = require_mapping(step.get("with"), f"{where}.with")
    script = with_config.get("script")
    require(isinstance(script, str) and script, f"{where}.with.script must be a non-empty string")
    return script


def validate_quality_gates(payload: dict[str, Any]) -> None:
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
    require(set(integrations) == REQUIRED_CHECKS, f"quality-gates.json: required_status_checks.integrations drifted: {sorted(integrations)}")
    require(
        all(value == 15368 for value in integrations.values()),
        f"quality-gates.json: required_status_checks.integrations must stay on GitHub Actions: {integrations}",
    )
    require(
        set(payload.get("required_checks", [])) == REQUIRED_CHECKS,
        f"quality-gates.json: required_checks drifted: {sorted(payload.get('required_checks', []))}",
    )

    require(review_policy.get("mode") == "conditional-required", "quality-gates.json: review_policy.mode drifted")
    require(review_policy.get("required_approvals") == 1, "quality-gates.json: review_policy.required_approvals drifted")
    require(review_policy.get("exempt_repository_owner") is True, "quality-gates.json: exempt_repository_owner must be true")
    require(
        set(review_policy.get("exempt_author_permissions", [])) == {"admin", "maintain"},
        "quality-gates.json: exempt_author_permissions drifted",
    )
    require(
        set(review_policy.get("allowed_reviewer_permissions", [])) == {"write", "maintain", "admin"},
        "quality-gates.json: allowed_reviewer_permissions drifted",
    )
    require(review_enforcement.get("mode") == "required-check", "quality-gates.json: enforcement.mode drifted")
    require(review_enforcement.get("check_name") == "Review Policy Gate", "quality-gates.json: enforcement.check_name drifted")

    expected = {
        (entry.get("workflow"), tuple(entry.get("jobs", [])))
        for entry in payload.get("expected_pr_workflows", [])
        if isinstance(entry, dict)
    }
    require(
        ("Label Gate", ("Validate PR labels",)) in expected,
        "quality-gates.json: expected_pr_workflows must include Label Gate workflow/job",
    )
    require(
        ("Review Policy", ("Review Policy Gate",)) in expected,
        "quality-gates.json: expected_pr_workflows must include Review Policy workflow/job",
    )
    require(
        (
            "CI Pipeline",
            (
                "Lint & Format Check",
                "Backend Tests",
                "Build Artifacts",
                "Release Meta (intent + version + tags)",
                "Build + Smoke + Push Candidate (linux/amd64)",
                "Build + Smoke + Push Candidate (linux/arm64)",
            ),
        ) in expected,
        "quality-gates.json: expected_pr_workflows must include CI Pipeline workflow/jobs",
    )

    waivers = payload.get("waivers", [])
    require(isinstance(waivers, list), "quality-gates.json: waivers must be an array")
    compat_waivers = [
        waiver
        for waiver in waivers
        if isinstance(waiver, dict)
        and waiver.get("kind") == "required-status-check-source-compat"
        and waiver.get("branch") == "main"
        and waiver.get("context") == "Review Policy Gate"
    ]
    require(len(compat_waivers) == 1, "quality-gates.json: missing Review Policy Gate source compatibility waiver")
    compat_waiver = compat_waivers[0]
    require(
        compat_waiver.get("allowed_integration_ids") == [None, 15368],
        "quality-gates.json: Review Policy Gate source compatibility waiver must allow [null, 15368]",
    )
    require(
        isinstance(compat_waiver.get("reason"), str) and compat_waiver["reason"],
        "quality-gates.json: Review Policy Gate source compatibility waiver must include a reason",
    )

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


def validate_ci(path: Path) -> None:
    workflow = load_yaml(path)
    require(workflow.get("name") == "CI Pipeline", "ci.yml: workflow name must stay 'CI Pipeline'")
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

    lint_job = job_config(workflow, "lint", "Lint & Format Check", "ci.yml")
    check_scripts = step_config(lint_job, "Check quality-gates scripts", "ci.yml.jobs.lint")
    require(".github/scripts/check_live_quality_gates.py" in str(check_scripts.get("run", "")), "ci.yml.jobs.lint: script check step drifted")
    contract_step = step_config(lint_job, "Quality-gates contract check", "ci.yml.jobs.lint")
    require(".github/scripts/check_quality_gates_contract.py" in str(contract_step.get("run", "")), "ci.yml.jobs.lint: contract check step drifted")
    live_step = step_config(lint_job, "Quality-gates live rules check", "ci.yml.jobs.lint")
    live_env = require_mapping(live_step.get("env"), "ci.yml.jobs.lint.steps['Quality-gates live rules check'].env")
    require(live_env.get("QUALITY_GATES_LIVE_RULES_MODE") == "require", "ci.yml.jobs.lint: live rules mode must stay require")
    require(".github/scripts/check_live_quality_gates.py" in str(live_step.get("run", "")), "ci.yml.jobs.lint: live rules step drifted")
    self_tests = step_config(lint_job, "Quality gates self-tests", "ci.yml.jobs.lint")
    run = str(self_tests.get("run", ""))
    require("test-quality-gates-contract.sh" in run and "test-live-quality-gates.sh" in run, "ci.yml.jobs.lint: self-tests step drifted")


def validate_label_gate(path: Path) -> None:
    workflow = load_yaml(path)
    require(workflow.get("name") == "Label Gate", "label-gate.yml: workflow name must stay 'Label Gate'")
    on_section = require_mapping(mapping_get(workflow, "on"), "label-gate.yml.on")
    require("merge_group" not in on_section, "label-gate.yml: merge_group must stay disabled")
    require("workflow_dispatch" not in on_section, "label-gate.yml: workflow_dispatch must stay disabled")
    pull_request_config = event_config(workflow, "pull_request", "label-gate.yml")
    assert_event_branches(pull_request_config, {"main"}, "label-gate.yml.on.pull_request")
    assert_event_types(pull_request_config, LABEL_GATE_PULL_REQUEST_TYPES, "label-gate.yml.on.pull_request")

    permissions = require_mapping(workflow.get("permissions"), "label-gate.yml.permissions")
    require(permissions.get("contents") == "read", "label-gate.yml.permissions.contents must stay read")
    require(permissions.get("pull-requests") == "read", "label-gate.yml.permissions.pull-requests must stay read")
    require(permissions.get("issues") == "read", "label-gate.yml.permissions.issues must stay read")

    job = job_config(workflow, "label-gate", "Validate PR labels", "label-gate.yml")
    require_no_checkout(job, "label-gate.yml.jobs.label-gate")
    step = step_config(job, "Validate release intent labels", "label-gate.yml.jobs.label-gate")
    script = script_body(step, "label-gate.yml.jobs.label-gate.steps['Validate release intent labels']")
    require("github.rest.issues.get" in script, "label-gate.yml: label gate must read labels via github.rest.issues.get")
    require("channel:stable" in script, "label-gate.yml: label gate must enforce channel labels")
    require("type:patch" in script and "type:skip" in script, "label-gate.yml: label gate must enforce type labels")
    require("createCommitStatus" not in script, "label-gate.yml: must not publish legacy commit statuses")


def validate_review_policy(path: Path, declaration: dict[str, Any]) -> None:
    workflow = load_yaml(path)
    require(workflow.get("name") == "Review Policy", "review-policy.yml: workflow name must stay 'Review Policy'")
    on_section = require_mapping(mapping_get(workflow, "on"), "review-policy.yml.on")
    require("merge_group" not in on_section, "review-policy.yml: merge_group must stay disabled")
    require("workflow_dispatch" not in on_section, "review-policy.yml: workflow_dispatch must stay disabled")
    pull_request_config = event_config(workflow, "pull_request", "review-policy.yml")
    assert_event_branches(pull_request_config, {"main"}, "review-policy.yml.on.pull_request")
    assert_event_types(pull_request_config, REVIEW_POLICY_PULL_REQUEST_TYPES, "review-policy.yml.on.pull_request")
    pull_request_review_config = event_config(workflow, "pull_request_review", "review-policy.yml")
    assert_event_types(pull_request_review_config, REVIEW_POLICY_REVIEW_TYPES, "review-policy.yml.on.pull_request_review")

    permissions = require_mapping(workflow.get("permissions"), "review-policy.yml.permissions")
    require(permissions.get("contents") == "read", "review-policy.yml.permissions.contents must stay read")
    require(permissions.get("pull-requests") == "read", "review-policy.yml.permissions.pull-requests must stay read")
    require(permissions.get("statuses") == "write", "review-policy.yml.permissions.statuses must stay write for rollout compatibility")

    job = job_config(workflow, "review-policy", "Review Policy Gate", "review-policy.yml")
    require_no_checkout(job, "review-policy.yml.jobs.review-policy")
    step = step_config(job, "Evaluate review policy", "review-policy.yml.jobs.review-policy")
    script = script_body(step, "review-policy.yml.jobs.review-policy.steps['Evaluate review policy']")
    review_policy = declaration["policy"]["review_policy"]
    require("createCommitStatus" in script, "review-policy.yml: must keep legacy commit-status dual-write during rollout")
    require("GET /repos/{owner}/{repo}/collaborators/{username}/permission" in script, "review-policy.yml: collaborator permission lookup drifted")
    require("GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews" in script, "review-policy.yml: reviews lookup drifted")
    require(f"const reviewRequiredApprovals = {review_policy['required_approvals']};" in script, "review-policy.yml: required approvals drifted")
    for permission in review_policy["exempt_author_permissions"]:
        require(f"'{permission}'" in script, f"review-policy.yml: missing exempt permission {permission!r}")
    for permission in review_policy["allowed_reviewer_permissions"]:
        require(f"'{permission}'" in script, f"review-policy.yml: missing reviewer permission {permission!r}")
    require("const reviewGateContext = 'Review Policy Gate';" in script, "review-policy.yml: rollout status context drifted")
    require("Author @${author} has ${authorPermission} permission; approval not required." in script, "review-policy.yml: owner/maintainer exemption copy drifted")


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


def main() -> int:
    args = parse_args()
    repo_root = Path(args.repo_root).resolve()
    scripts_dir = repo_root / ".github" / "scripts"
    declaration_path = Path(args.declaration).resolve() if args.declaration else repo_root / ".github" / "quality-gates.json"
    metadata_script_path = (
        Path(args.metadata_script).resolve() if args.metadata_script else scripts_dir / "metadata_gate.py"
    )

    try:
        declaration = json.loads(declaration_path.read_text())
        require(isinstance(declaration, dict), "quality-gates.json must decode to an object")
        module = load_module(metadata_script_path)
        validate_quality_gates(declaration)
        validate_ci(repo_root / ".github" / "workflows" / "ci.yml")
        validate_label_gate(repo_root / ".github" / "workflows" / "label-gate.yml")
        validate_review_policy(repo_root / ".github" / "workflows" / "review-policy.yml", declaration)
        validate_merge_group_helpers(module)
    except ContractError as exc:
        print(f"[quality-gates-contract] {exc}", file=sys.stderr)
        return 1

    print("[quality-gates-contract] metadata workflow contract checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
