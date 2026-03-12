#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib.util
import json
import shlex
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
    checkout = uses_step_config(lint_job, "Checkout", "actions/checkout@v4", "ci.yml.jobs.lint")
    checkout_with = require_mapping(checkout.get("with"), "ci.yml.jobs.lint.steps['Checkout'].with")
    require(checkout_with.get("fetch-depth") == 0, "ci.yml.jobs.lint Checkout must fetch full history for trusted source resolution")
    check_scripts = step_config(lint_job, "Check quality-gates scripts", "ci.yml.jobs.lint")
    require(".github/scripts/check_live_quality_gates.py" in str(check_scripts.get("run", "")), "ci.yml.jobs.lint: script check step drifted")
    trusted_step = step_config(lint_job, "Resolve trusted quality-gates sources", "ci.yml.jobs.lint")
    trusted_run = str(trusted_step.get("run", ""))
    require("git fetch --no-tags --depth=1 origin" in trusted_run, "ci.yml.jobs.lint: trusted-source fetch drifted")
    require("git show \"${source_ref}:${path}\"" in trusted_run, "ci.yml.jobs.lint: trusted-source materialization drifted")
    require("Base branch is missing trusted quality-gates source" in trusted_run, "ci.yml.jobs.lint: trusted-source hard-fail drifted")
    require("bootstrap-current-branch" not in trusted_run, "ci.yml.jobs.lint: bootstrap fallback must stay removed")
    require("trusted_root/.github/scripts/check_quality_gates_contract.py" in trusted_run, "ci.yml.jobs.lint: trusted-source outputs drifted")
    contract_step = step_config(lint_job, "Quality-gates contract check", "ci.yml.jobs.lint")
    contract_run = str(contract_step.get("run", ""))
    require(
        'steps.trusted-quality-gates.outputs.contract_script' in contract_run
        and 'steps.trusted-quality-gates.outputs.declaration' in contract_run
        and 'steps.trusted-quality-gates.outputs.metadata_script' in contract_run,
        "ci.yml.jobs.lint: contract check must use trusted quality-gates sources",
    )
    live_step = step_config(lint_job, "Quality-gates live rules check", "ci.yml.jobs.lint")
    live_env = require_mapping(live_step.get("env"), "ci.yml.jobs.lint.steps['Quality-gates live rules check'].env")
    require(live_env.get("QUALITY_GATES_LIVE_RULES_MODE") == "require", "ci.yml.jobs.lint: live rules mode must stay require")
    live_run = str(live_step.get("run", ""))
    require(
        'steps.trusted-quality-gates.outputs.live_script' in live_run
        and 'steps.trusted-quality-gates.outputs.declaration' in live_run,
        "ci.yml.jobs.lint: live rules step must use trusted quality-gates sources",
    )
    self_tests = step_config(lint_job, "Quality gates self-tests", "ci.yml.jobs.lint")
    run = str(self_tests.get("run", ""))
    require("test-quality-gates-contract.sh" in run and "test-live-quality-gates.sh" in run, "ci.yml.jobs.lint: self-tests step drifted")


def validate_label_gate(path: Path) -> None:
    workflow = load_yaml(path)
    require(workflow.get("name") == "Label Gate", "label-gate.yml: workflow name must stay 'Label Gate'")
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

    job = job_config(workflow, "label-gate", "Validate PR labels", "label-gate.yml")
    trusted_checkout = checkout_step(job, "Checkout trusted base", "label-gate.yml.jobs.label-gate")
    require(trusted_checkout.get("ref") == "${{ github.event.pull_request.base.ref }}", "label-gate.yml: trusted checkout ref drifted")
    require(trusted_checkout.get("path") == "trusted", "label-gate.yml: trusted checkout path must stay 'trusted'")
    require(trusted_checkout.get("persist-credentials") is False, "label-gate.yml: trusted checkout must not persist credentials")

    candidate_checkout = checkout_step(job, "Checkout candidate changes", "label-gate.yml.jobs.label-gate")
    require(
        candidate_checkout.get("repository") == "${{ github.event.pull_request.head.repo.full_name }}",
        "label-gate.yml: candidate checkout repository drifted",
    )
    require(candidate_checkout.get("ref") == "${{ github.event.pull_request.head.sha }}", "label-gate.yml: candidate checkout ref drifted")
    require(candidate_checkout.get("path") == "candidate", "label-gate.yml: candidate checkout path must stay 'candidate'")
    require(candidate_checkout.get("persist-credentials") is False, "label-gate.yml: candidate checkout must not persist credentials")

    contract_step = step_config(job, "Validate workflow contract", "label-gate.yml.jobs.label-gate")
    contract_command = require_command(
        contract_step,
        ["python3", "trusted/.github/scripts/check_quality_gates_contract.py"],
        "label-gate.yml.jobs.label-gate.steps['Validate workflow contract']",
        "label-gate.yml: Validate workflow contract must invoke trusted contract checker",
    )
    contract_options = command_option_map(contract_command[2:], "label-gate.yml: Validate workflow contract")
    require(contract_options.get("--repo-root") == "candidate", "label-gate.yml: contract checker must validate the candidate repo")
    require(
        contract_options.get("--declaration") == "candidate/.github/quality-gates.json",
        "label-gate.yml: contract checker must read the candidate declaration",
    )
    require(
        contract_options.get("--metadata-script") == "trusted/.github/scripts/metadata_gate.py",
        "label-gate.yml: contract checker must anchor to trusted metadata_gate.py",
    )

    step = step_config(job, "Validate release intent labels", "label-gate.yml.jobs.label-gate")
    env = require_mapping(step.get("env"), "label-gate.yml.jobs.label-gate.steps['Validate release intent labels'].env")
    require(env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}", "label-gate.yml: label gate must pass GITHUB_TOKEN via env")
    require_command(
        step,
        ["python3", "trusted/.github/scripts/metadata_gate.py", "label"],
        "label-gate.yml.jobs.label-gate.steps['Validate release intent labels']",
        "label-gate.yml: Validate PR labels must invoke trusted metadata gate",
    )


def validate_review_policy(path: Path) -> None:
    workflow = load_yaml(path)
    require(workflow.get("name") == "Review Policy", "review-policy.yml: workflow name must stay 'Review Policy'")
    on_section = require_mapping(mapping_get(workflow, "on"), "review-policy.yml.on")
    require("merge_group" not in on_section, "review-policy.yml: merge_group must stay disabled")
    require("workflow_dispatch" not in on_section, "review-policy.yml: workflow_dispatch must stay disabled")
    require("pull_request" not in on_section, "review-policy.yml: pull_request must stay disabled")
    pull_request_target_config = event_config(workflow, "pull_request_target", "review-policy.yml")
    assert_event_branches(pull_request_target_config, {"main"}, "review-policy.yml.on.pull_request_target")
    assert_event_types(pull_request_target_config, REVIEW_POLICY_PULL_REQUEST_TYPES, "review-policy.yml.on.pull_request_target")
    pull_request_review_config = event_config(workflow, "pull_request_review", "review-policy.yml")
    assert_event_types(pull_request_review_config, REVIEW_POLICY_REVIEW_TYPES, "review-policy.yml.on.pull_request_review")

    permissions = require_mapping(workflow.get("permissions"), "review-policy.yml.permissions")
    require(permissions.get("contents") == "read", "review-policy.yml.permissions.contents must stay read")
    require(permissions.get("pull-requests") == "read", "review-policy.yml.permissions.pull-requests must stay read")
    require("statuses" not in permissions, "review-policy.yml.permissions.statuses must stay unset")

    job = job_config(workflow, "review-policy", "Review Policy Gate", "review-policy.yml")
    require(job.get("if") == "${{ github.event.pull_request.base.ref == 'main' }}", "review-policy.yml.jobs.review-policy.if must stay pinned to the main base branch")
    trusted_checkout = checkout_step(job, "Checkout trusted base", "review-policy.yml.jobs.review-policy")
    require(trusted_checkout.get("ref") == "${{ github.event.pull_request.base.ref }}", "review-policy.yml: trusted checkout ref drifted")
    require(trusted_checkout.get("path") == "trusted", "review-policy.yml: trusted checkout path must stay 'trusted'")
    require(trusted_checkout.get("persist-credentials") is False, "review-policy.yml: trusted checkout must not persist credentials")

    step = step_config(job, "Evaluate review policy", "review-policy.yml.jobs.review-policy")
    env = require_mapping(step.get("env"), "review-policy.yml.jobs.review-policy.steps['Evaluate review policy'].env")
    require(env.get("GITHUB_TOKEN") == "${{ secrets.GITHUB_TOKEN }}", "review-policy.yml: review gate must pass GITHUB_TOKEN via env")
    require_command(
        step,
        ["python3", "trusted/.github/scripts/metadata_gate.py", "review"],
        "review-policy.yml.jobs.review-policy.steps['Evaluate review policy']",
        "review-policy.yml: Evaluate review policy must invoke trusted metadata gate",
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
        validate_review_policy(repo_root / ".github" / "workflows" / "review-policy.yml")
        validate_merge_group_helpers(module)
    except ContractError as exc:
        print(f"[quality-gates-contract] {exc}", file=sys.stderr)
        return 1

    print("[quality-gates-contract] metadata workflow contract checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
