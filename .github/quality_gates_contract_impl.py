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
    expected_pr_auxiliary_workflows: dict[str, tuple[str, ...]]
    expected_main_workflows: dict[str, tuple[str, ...]]
    expected_main_auxiliary_workflows: dict[str, tuple[str, ...]]
    expected_auxiliary_workflows: dict[str, tuple[str, ...]]
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
RUNNER_X64 = "ubuntu-24.04"
RUNNER_ARM64 = "ubuntu-24.04-arm"
ACTION_CHECKOUT = "actions/checkout@v7"


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
    step = uses_step_config(job, step_name, ACTION_CHECKOUT, where)
    return require_mapping(step.get("with"), f"{where}.steps[{step_name!r}].with")


def require_no_if(mapping: dict[str, Any], where: str) -> None:
    require("if" not in mapping, f"{where}.if must stay unset")


def require_exact_if(mapping: dict[str, Any], expected: str, where: str) -> None:
    require(mapping.get("if") == expected, f"{where}.if must stay {expected!r}")


def require_fail_closed(mapping: dict[str, Any], where: str) -> None:
    require(mapping.get("continue-on-error") in (None, False), f"{where}.continue-on-error must not ignore failures")


def parse_expected_workflows(payload: dict[str, Any], key: str) -> tuple[dict[str, tuple[str, ...]], dict[str, tuple[str, ...]]]:
    raw_expected = payload.get(key)
    require(isinstance(raw_expected, list) and raw_expected, f"quality-gates.json: {key} must be a non-empty array")
    expected: dict[str, tuple[str, ...]] = {}
    auxiliary: dict[str, tuple[str, ...]] = {}
    for index, raw_entry in enumerate(raw_expected):
        entry = require_mapping(raw_entry, f"quality-gates.json.{key}[{index}]")
        workflow_name = entry.get("workflow")
        require(
            isinstance(workflow_name, str) and workflow_name,
            f"quality-gates.json.{key}[{index}].workflow must be a non-empty string",
        )
        jobs = require_string_set(entry.get("jobs"), f"quality-gates.json.{key}[{index}].jobs")
        auxiliary_jobs = require_string_set(
            entry.get("auxiliary_jobs", []), f"quality-gates.json.{key}[{index}].auxiliary_jobs"
        )
        require(
            jobs.isdisjoint(auxiliary_jobs),
            f"quality-gates.json.{key}[{index}]: jobs and auxiliary_jobs must be disjoint",
        )
        require(workflow_name not in expected, f"quality-gates.json: duplicate {key} entry {workflow_name!r}")
        expected[workflow_name] = tuple(sorted(jobs))
        auxiliary[workflow_name] = tuple(sorted(auxiliary_jobs))
    return expected, auxiliary


def _validate_declaration_header(payload: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any], dict[str, Any], str]:
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
    return policy, branch_policy, review_policy, review_enforcement, implementation_profile


def validate_quality_gates(payload: dict[str, Any]) -> ContractModel:
    policy, branch_policy, review_policy, review_enforcement, implementation_profile = _validate_declaration_header(payload)

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
    require(not informational_checks, "quality-gates.json: informational_checks must stay empty")
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

    expected_pr_workflows, expected_pr_auxiliary_workflows = parse_expected_workflows(payload, "expected_pr_workflows")
    expected_main_workflows, expected_main_auxiliary_workflows = parse_expected_workflows(payload, "expected_main_workflows")
    expected_auxiliary_workflows, expected_auxiliary_auxiliary_workflows = parse_expected_workflows(payload, "expected_auxiliary_workflows")
    require(
        not any(expected_auxiliary_auxiliary_workflows.values()),
        "quality-gates.json: auxiliary workflows must not declare auxiliary jobs",
    )
    expected_release_workflows, expected_release_auxiliary_workflows = parse_expected_workflows(payload, "expected_release_workflows")
    require(
        not any(expected_release_auxiliary_workflows.values()),
        "quality-gates.json: release workflows must not declare auxiliary jobs",
    )

    declared_pr_jobs = {job for jobs in expected_pr_workflows.values() for job in jobs}
    require(
        declared_pr_jobs == required_checks,
        "quality-gates.json: expected_pr_workflows jobs must exactly cover required_checks",
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
        expected_pr_auxiliary_workflows=expected_pr_auxiliary_workflows,
        expected_main_workflows=expected_main_workflows,
        expected_main_auxiliary_workflows=expected_main_auxiliary_workflows,
        expected_auxiliary_workflows=expected_auxiliary_workflows,
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
    for filename in ("ci-pr.yml", "ci-main.yml", "release.yml", "label-gate.yml", "review-policy.yml", "release-snapshot-pr.yml"):
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
        validate_backend_test_image(repo_root / ".github" / "workflows" / "backend-test-image.yml", contract)
        validate_release(repo_root / ".github" / "workflows" / "release.yml", contract)
        validate_release_snapshot_pr(repo_root / ".github" / "workflows" / "release-snapshot-pr.yml")
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
