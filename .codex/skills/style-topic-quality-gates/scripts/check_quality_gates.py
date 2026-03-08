#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def parse_scalar(value: str) -> str:
    value = value.strip()
    if not value:
        return ""
    if value[0] == value[-1] and value[0] in {'"', "'"}:
        return value[1:-1]
    return value


@dataclass
class WorkflowInventory:
    path: str
    workflow: str
    jobs: list[str]


def scan_workflows(repo_root: Path) -> list[WorkflowInventory]:
    workflows_dir = repo_root / '.github' / 'workflows'
    items: list[WorkflowInventory] = []
    if not workflows_dir.exists():
        return items

    for path in sorted(list(workflows_dir.glob('*.yml')) + list(workflows_dir.glob('*.yaml'))):
        lines = path.read_text(encoding='utf-8').splitlines()
        workflow_name = path.stem
        in_jobs = False
        current_job: str | None = None
        jobs: list[str] = []
        for raw in lines:
            if not raw.strip() or raw.lstrip().startswith('#'):
                continue
            indent = len(raw) - len(raw.lstrip(' '))
            stripped = raw.strip()
            if indent == 0 and stripped.startswith('name:'):
                workflow_name = parse_scalar(stripped.split(':', 1)[1]) or workflow_name
                continue
            if indent == 0 and stripped == 'jobs:':
                in_jobs = True
                current_job = None
                continue
            if in_jobs and indent == 0 and stripped.endswith(':') and stripped != 'jobs:':
                in_jobs = False
                current_job = None
            if not in_jobs:
                continue
            match = re.match(r'([A-Za-z0-9_-]+):\s*$', stripped)
            if indent == 2 and match:
                current_job = match.group(1)
                jobs.append(current_job)
                continue
            if current_job and indent == 4 and stripped.startswith('name:'):
                jobs[-1] = parse_scalar(stripped.split(':', 1)[1]) or jobs[-1]
        items.append(WorkflowInventory(path=str(path.relative_to(repo_root)), workflow=workflow_name, jobs=jobs))
    return items


def workflow_lookup(inventory: list[WorkflowInventory]) -> dict[str, set[str]]:
    return {item.workflow: set(item.jobs) for item in inventory}


def all_job_names(inventory: list[WorkflowInventory]) -> set[str]:
    names: set[str] = set()
    for item in inventory:
        names.update(item.jobs)
    return names


def validate_declaration(data: Any) -> list[str]:
    errors: list[str] = []
    if not isinstance(data, dict):
        return ['declaration must be a JSON object']

    if data.get('schema_version') != 1:
        errors.append('schema_version must equal 1')

    policy = data.get('policy')
    if not isinstance(policy, dict):
        errors.append('policy must be an object')
        policy = {}

    if policy.get('baseline_policy') != 'explicit-waiver-required':
        errors.append('policy.baseline_policy must equal explicit-waiver-required')

    if policy.get('require_signed_commits') is not True:
        errors.append('policy.require_signed_commits must be true')

    branch_protection = policy.get('branch_protection')
    if not isinstance(branch_protection, dict):
        errors.append('policy.branch_protection must be an object')
    else:
        branches = branch_protection.get('protected_branches')
        if not isinstance(branches, list) or not branches or not all(isinstance(item, str) and item for item in branches):
            errors.append('policy.branch_protection.protected_branches must be a non-empty string list')
        if branch_protection.get('require_pull_request') is not True:
            errors.append('policy.branch_protection.require_pull_request must be true')
        if branch_protection.get('disallow_direct_pushes') is not True:
            errors.append('policy.branch_protection.disallow_direct_pushes must be true')

    review = policy.get('review_policy')
    if not isinstance(review, dict):
        errors.append('policy.review_policy must be an object')
        review = {}

    if review.get('mode') != 'conditional-required':
        errors.append('policy.review_policy.mode must equal conditional-required')

    if not isinstance(review.get('required_approvals'), int) or review.get('required_approvals', 0) < 1:
        errors.append('policy.review_policy.required_approvals must be an integer >= 1')

    for key in ('exempt_author_permissions', 'allowed_reviewer_permissions'):
        value = review.get(key)
        if not isinstance(value, list) or not value or not all(isinstance(item, str) and item for item in value):
            errors.append(f'policy.review_policy.{key} must be a non-empty string list')

    enforcement = review.get('enforcement')
    if not isinstance(enforcement, dict):
        errors.append('policy.review_policy.enforcement must be an object')
    else:
        if enforcement.get('mode') != 'required-check':
            errors.append('policy.review_policy.enforcement.mode must equal required-check')
        if not isinstance(enforcement.get('check_name'), str) or not enforcement.get('check_name'):
            errors.append('policy.review_policy.enforcement.check_name must be a non-empty string')

    for key in ('required_checks', 'informational_checks', 'waivers', 'expected_pr_workflows'):
        value = data.get(key)
        if not isinstance(value, list):
            errors.append(f'{key} must be a list')

    for item in data.get('expected_pr_workflows', []):
        if not isinstance(item, dict):
            errors.append('expected_pr_workflows entries must be objects')
            continue
        if not isinstance(item.get('workflow'), str) or not item.get('workflow'):
            errors.append('expected_pr_workflows.workflow must be a non-empty string')
        jobs = item.get('jobs')
        if not isinstance(jobs, list) or not jobs or not all(isinstance(job, str) and job for job in jobs):
            errors.append('expected_pr_workflows.jobs must be a non-empty string list')

    return errors


def load_github_required_checks(args: argparse.Namespace) -> list[str] | None:
    if args.github_required_checks_file:
        data = load_json(Path(args.github_required_checks_file))
        if isinstance(data, dict):
            data = data.get('required_checks')
        if not isinstance(data, list) or not all(isinstance(item, str) for item in data):
            raise SystemExit('--github-required-checks-file must contain a JSON string array or an object with required_checks')
        return data
    if args.github_required_check:
        return args.github_required_check
    return None


def build_report(repo_root: Path, declaration_path: Path, strict: bool, github_checks: list[str] | None) -> tuple[int, dict[str, Any]]:
    data = load_json(declaration_path)
    errors = validate_declaration(data)
    inventory = scan_workflows(repo_root)
    workflow_map = workflow_lookup(inventory)
    job_names = all_job_names(inventory)

    required_checks = [item for item in data.get('required_checks', []) if isinstance(item, str)] if isinstance(data, dict) else []
    missing_required_jobs = sorted(check for check in required_checks if check not in job_names)

    missing_workflows: list[dict[str, Any]] = []
    if isinstance(data, dict):
        for item in data.get('expected_pr_workflows', []):
            if not isinstance(item, dict):
                continue
            workflow_name = item.get('workflow')
            jobs = item.get('jobs') if isinstance(item.get('jobs'), list) else []
            existing = workflow_map.get(workflow_name)
            if existing is None:
                missing_workflows.append({'workflow': workflow_name, 'missing_jobs': jobs})
                continue
            missing_jobs = [job for job in jobs if job not in existing]
            if missing_jobs:
                missing_workflows.append({'workflow': workflow_name, 'missing_jobs': missing_jobs})

    github_drift = None
    exit_code = 0
    if errors:
        exit_code = 2
    elif github_checks is not None:
        declared = set(required_checks)
        actual = set(github_checks)
        github_drift = {
            'missing_on_github': sorted(declared - actual),
            'unexpected_on_github': sorted(actual - declared),
        }
        if github_drift['missing_on_github'] or github_drift['unexpected_on_github']:
            exit_code = 1
    elif strict and (missing_required_jobs or missing_workflows):
        exit_code = 1

    report = {
        'repo_root': str(repo_root),
        'declaration': str(declaration_path),
        'declaration_errors': errors,
        'workflow_inventory': [
            {'path': item.path, 'workflow': item.workflow, 'jobs': item.jobs}
            for item in inventory
        ],
        'required_checks_missing_from_workflow_inventory': missing_required_jobs,
        'expected_pr_workflow_drift': missing_workflows,
        'github_required_checks': github_checks,
        'github_alignment': github_drift,
        'status': 'invalid' if exit_code == 2 else 'drift' if exit_code == 1 else 'ok',
    }
    return exit_code, report


def main() -> int:
    parser = argparse.ArgumentParser(description='Validate a quality-gates declaration against repo workflow inventory and optional GitHub required checks.')
    parser.add_argument('--repo-root', default='.')
    parser.add_argument('--declaration', default='.github/quality-gates.json')
    parser.add_argument('--strict', action='store_true', help='Fail when declaration entries drift from local workflow inventory.')
    parser.add_argument('--github-required-check', action='append', default=[], help='Repeatable exact GitHub required check names to compare against the declaration.')
    parser.add_argument('--github-required-checks-file', help='JSON file containing a string array of GitHub required checks or an object with required_checks.')
    args = parser.parse_args()

    repo_root = Path(args.repo_root).expanduser().resolve()
    declaration_path = Path(args.declaration)
    if not declaration_path.is_absolute():
        declaration_path = (repo_root / declaration_path).resolve()
    if not declaration_path.exists():
        raise SystemExit(f'declaration not found: {declaration_path}')

    github_checks = load_github_required_checks(args)
    exit_code, report = build_report(repo_root, declaration_path, args.strict, github_checks)
    print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    return exit_code


if __name__ == '__main__':
    raise SystemExit(main())
