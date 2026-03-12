#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

API_VERSION = "2022-11-28"


class ValidationError(RuntimeError):
    pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate the live GitHub branch rules against .github/quality-gates.json."
    )
    parser.add_argument(
        "--declaration",
        default=".github/quality-gates.json",
        help="Path to the quality gates declaration file.",
    )
    parser.add_argument(
        "--repo",
        default=os.environ.get("GITHUB_REPOSITORY", ""),
        help="GitHub repository in owner/name form. Defaults to GITHUB_REPOSITORY.",
    )
    parser.add_argument(
        "--branch",
        default="",
        help="Protected branch to validate. Defaults to all protected branches declared in the contract.",
    )
    parser.add_argument(
        "--api-root",
        default=os.environ.get("GITHUB_API_URL", "https://api.github.com"),
        help="GitHub API root URL. Defaults to GITHUB_API_URL or https://api.github.com.",
    )
    parser.add_argument(
        "--mode",
        choices=("auto", "require", "skip"),
        default=os.environ.get("QUALITY_GATES_LIVE_RULES_MODE", "auto"),
        help="skip: never validate; auto: validate only on GitHub Actions; require: always validate.",
    )
    parser.add_argument(
        "--rules-file",
        default="",
        help="Use a local branch-rules JSON fixture instead of calling the GitHub API.",
    )
    return parser.parse_args()


def should_skip(mode: str) -> bool:
    if mode == "skip":
        print("[live-quality-gates] skipped: QUALITY_GATES_LIVE_RULES_MODE=skip")
        return True
    if mode == "auto" and os.environ.get("GITHUB_ACTIONS") != "true":
        print("[live-quality-gates] skipped: outside GitHub Actions")
        return True
    return False


def load_declaration(path: str) -> dict[str, Any]:
    payload = json.loads(Path(path).read_text())
    if not isinstance(payload, dict):
        raise ValidationError("quality-gates declaration must be a JSON object")
    return payload


def choose_branches(declaration: dict[str, Any], override: str) -> list[str]:
    if override:
        return [override]
    raw_branches = (
        declaration.get("policy", {})
        .get("branch_protection", {})
        .get("protected_branches", [])
    )
    if not isinstance(raw_branches, list) or not raw_branches:
        raise ValidationError("protected_branches must declare at least one protected branch")
    branches: list[str] = []
    for index, branch in enumerate(raw_branches):
        if not isinstance(branch, str) or not branch:
            raise ValidationError(f"protected_branches[{index}] must be a non-empty string")
        if branch not in branches:
            branches.append(branch)
    return branches


def split_repo(repo: str) -> tuple[str, str]:
    owner, sep, name = repo.partition("/")
    if not sep or not owner or not name:
        raise ValidationError("--repo must be in owner/name form")
    return owner, name


def fetch_branch_rules(api_root: str, owner: str, repo: str, branch: str) -> Any:
    base_path = "/repos/{owner}/{repo}/rules/branches/{branch}".format(
        owner=urllib.parse.quote(owner, safe=""),
        repo=urllib.parse.quote(repo, safe=""),
        branch=urllib.parse.quote(branch, safe=""),
    )
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "codex-vibe-monitor-quality-gates-live-check/1.0",
        "X-GitHub-Api-Version": API_VERSION,
    }
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN") or ""
    if token:
        headers["Authorization"] = f"Bearer {token}"
    rules: list[Any] = []
    page = 1
    while True:
        query = urllib.parse.urlencode({"per_page": 100, "page": page})
        url = api_root.rstrip("/") + base_path + "?" + query
        request = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                payload = json.load(response)
        except urllib.error.HTTPError as exc:
            detail = exc.read().decode("utf-8", errors="replace")
            raise ValidationError(f"GitHub API request failed ({exc.code}): {detail}") from exc
        except urllib.error.URLError as exc:
            raise ValidationError(f"GitHub API request failed: {exc.reason}") from exc

        if isinstance(payload, dict) and isinstance(payload.get("data"), list):
            page_rules = payload["data"]
        elif isinstance(payload, list):
            page_rules = payload
        else:
            raise ValidationError("Unsupported GitHub branch rules payload type")

        rules.extend(page_rules)
        if len(page_rules) < 100:
            break
        page += 1

    return rules


def extract_rules(payload: Any) -> list[dict[str, Any]]:
    if isinstance(payload, dict) and isinstance(payload.get("data"), list):
        payload = payload["data"]
    if not isinstance(payload, list):
        raise ValidationError("Unsupported GitHub branch rules payload type")
    rules = [item for item in payload if isinstance(item, dict) and isinstance(item.get("type"), str)]
    if not rules:
        raise ValidationError("GitHub branch rules payload did not contain any typed rules")
    return rules


def bool_field(parameters: dict[str, Any], name: str) -> bool:
    return bool(parameters.get(name, False))


def normalize_required_status_checks(
    rules: list[dict[str, Any]],
) -> tuple[list[str], dict[str, set[int | None]], set[bool]]:
    contexts: set[str] = set()
    integrations: dict[str, set[int | None]] = {}
    strict_values: set[bool] = set()
    for rule in rules:
        parameters = rule.get("parameters") or {}
        if not isinstance(parameters, dict):
            continue
        strict = parameters.get("strict_required_status_checks_policy")
        if isinstance(strict, bool):
            strict_values.add(strict)
        raw_checks = parameters.get("required_status_checks") or []
        if not isinstance(raw_checks, list):
            continue
        for item in raw_checks:
            if not isinstance(item, dict):
                continue
            context = item.get("context")
            if isinstance(context, str) and context:
                contexts.add(context)
                integration = item.get("integration_id")
                if integration is not None and not isinstance(integration, int):
                    integration = None
                integrations.setdefault(context, set()).add(integration)
    return sorted(contexts), integrations, strict_values


def branch_waivers(
    declaration: dict[str, Any], branch: str
) -> tuple[dict[str, set[int | None]], str | None]:
    raw_waivers = declaration.get("waivers", [])
    if raw_waivers is None:
        raw_waivers = []
    if not isinstance(raw_waivers, list):
        raise ValidationError("waivers must be a JSON array")

    source_compat: dict[str, set[int | None]] = {}
    bypass_reason: str | None = None
    for index, waiver in enumerate(raw_waivers):
        if not isinstance(waiver, dict):
            raise ValidationError(f"waivers[{index}] must be a JSON object")
        kind = waiver.get("kind")
        waiver_branch = waiver.get("branch")
        reason = waiver.get("reason")
        if not isinstance(kind, str) or not kind:
            raise ValidationError(f"waivers[{index}].kind must be a non-empty string")
        if not isinstance(waiver_branch, str) or not waiver_branch:
            raise ValidationError(f"waivers[{index}].branch must be a non-empty string")
        if not isinstance(reason, str) or not reason:
            raise ValidationError(f"waivers[{index}].reason must be a non-empty string")
        if waiver_branch != branch:
            continue
        if kind == "required-status-check-source-compat":
            context = waiver.get("context")
            raw_integrations = waiver.get("allowed_integration_ids")
            if not isinstance(context, str) or not context:
                raise ValidationError(
                    f"waivers[{index}].context must be a non-empty string for required-status-check-source-compat"
                )
            if not isinstance(raw_integrations, list) or not raw_integrations:
                raise ValidationError(
                    f"waivers[{index}].allowed_integration_ids must be a non-empty array for required-status-check-source-compat"
                )
            normalized: set[int | None] = set()
            for value in raw_integrations:
                if value is None:
                    normalized.add(None)
                elif isinstance(value, int):
                    normalized.add(value)
                else:
                    raise ValidationError(
                        f"waivers[{index}].allowed_integration_ids entries must be integers or null"
                    )
            source_compat[context] = normalized
        elif kind == "bypass-actors-unverified":
            bypass_reason = reason

    return source_compat, bypass_reason


def validate_rules(
    declaration: dict[str, Any], rules: list[dict[str, Any]], branch: str
) -> tuple[list[str], list[str]]:
    notes: list[str] = []
    errors: list[str] = []
    policy = declaration.get("policy", {})
    if not isinstance(policy, dict):
        raise ValidationError("policy must be a JSON object")

    branch_policy = policy.get("branch_protection", {})
    if not isinstance(branch_policy, dict):
        raise ValidationError("policy.branch_protection must be a JSON object")

    review_policy = policy.get("review_policy", {})
    if not isinstance(review_policy, dict):
        raise ValidationError("policy.review_policy must be a JSON object")

    review_enforcement = review_policy.get("enforcement", {})
    if not isinstance(review_enforcement, dict):
        raise ValidationError("policy.review_policy.enforcement must be a JSON object")

    required_checks = declaration.get("required_checks", [])
    if not isinstance(required_checks, list) or not all(isinstance(item, str) and item for item in required_checks):
        raise ValidationError("required_checks must be a list of non-empty strings")
    required_checks = sorted(set(required_checks))

    require_signed_commits = bool(policy.get("require_signed_commits", False))
    require_pull_request = bool(branch_policy.get("require_pull_request", False))
    disallow_branch_deletions = bool(branch_policy.get("disallow_branch_deletions", False))
    disallow_force_pushes = bool(branch_policy.get("disallow_force_pushes", False))
    allow_merge_commits = branch_policy.get("allow_merge_commits", True)
    if not isinstance(allow_merge_commits, bool):
        raise ValidationError("policy.branch_protection.allow_merge_commits must be a boolean")
    require_merge_queue = bool(branch_policy.get("require_merge_queue", False))
    declared_required_reviewers = branch_policy.get("required_reviewers", [])
    if declared_required_reviewers is None:
        declared_required_reviewers = []
    if not isinstance(declared_required_reviewers, list):
        raise ValidationError("policy.branch_protection.required_reviewers must be a JSON array")
    if declared_required_reviewers:
        raise ValidationError(
            "policy.branch_protection.required_reviewers only supports an empty array in this repository"
        )
    status_check_policy = branch_policy.get("required_status_checks", {})
    if status_check_policy is None:
        status_check_policy = {}
    if not isinstance(status_check_policy, dict):
        raise ValidationError("policy.branch_protection.required_status_checks must be a JSON object")
    expected_strict_status_checks = status_check_policy.get("strict")
    if expected_strict_status_checks is not None and not isinstance(expected_strict_status_checks, bool):
        raise ValidationError("policy.branch_protection.required_status_checks.strict must be a boolean")
    expected_integrations = status_check_policy.get("integrations", {})
    if expected_integrations is None:
        expected_integrations = {}
    if not isinstance(expected_integrations, dict):
        raise ValidationError("policy.branch_protection.required_status_checks.integrations must be a JSON object")
    normalized_expected_integrations: dict[str, int] = {}
    for context, integration in expected_integrations.items():
        if not isinstance(context, str) or not context:
            raise ValidationError("policy.branch_protection.required_status_checks.integrations keys must be strings")
        if not isinstance(integration, int):
            raise ValidationError(
                "policy.branch_protection.required_status_checks.integrations values must be integers"
            )
        normalized_expected_integrations[context] = integration
    source_compat_waivers, bypass_reason = branch_waivers(declaration, branch)
    enforcement_mode = str(review_enforcement.get("mode", ""))
    expected_native_approvals = int(review_policy.get("required_approvals", 0)) if enforcement_mode == "github-native" else 0

    grouped: dict[str, list[dict[str, Any]]] = {}
    for rule in rules:
        grouped.setdefault(rule.get("type", ""), []).append(rule)

    if disallow_branch_deletions and "deletion" not in grouped:
        errors.append(f"{branch}: missing deletion rule")
    if not disallow_branch_deletions and "deletion" in grouped:
        errors.append(f"{branch}: unexpected deletion rule")

    if disallow_force_pushes and "non_fast_forward" not in grouped:
        errors.append(f"{branch}: missing non_fast_forward rule")
    if not disallow_force_pushes and "non_fast_forward" in grouped:
        errors.append(f"{branch}: unexpected non_fast_forward rule")

    if require_signed_commits and "required_signatures" not in grouped:
        errors.append(f"{branch}: missing required_signatures rule")

    if require_merge_queue and "merge_queue" not in grouped:
        errors.append(f"{branch}: missing merge_queue rule")
    if not require_merge_queue and "merge_queue" in grouped:
        errors.append(f"{branch}: unexpected merge_queue rule")
    if allow_merge_commits and "required_linear_history" in grouped:
        errors.append(f"{branch}: merge commits must remain allowed")

    if branch_policy.get("disallow_direct_pushes") and "pull_request" not in grouped:
        errors.append(f"{branch}: missing pull_request rule required to block direct pushes")

    if require_pull_request:
        pull_request_rules = grouped.get("pull_request", [])
        if not pull_request_rules:
            errors.append(f"{branch}: missing pull_request rule")
        else:
            max_approvals = 0
            stale_review = False
            code_owner_review = False
            last_push_approval = False
            thread_resolution = False
            merge_method_block = False
            required_reviewers_present = False
            for rule in pull_request_rules:
                parameters = rule.get("parameters") or {}
                if not isinstance(parameters, dict):
                    continue
                value = parameters.get("required_approving_review_count", 0)
                if isinstance(value, bool):
                    value = int(value)
                if isinstance(value, int):
                    max_approvals = max(max_approvals, value)
                stale_review = stale_review or bool_field(parameters, "dismiss_stale_reviews_on_push")
                code_owner_review = code_owner_review or bool_field(parameters, "require_code_owner_review")
                last_push_approval = last_push_approval or bool_field(parameters, "require_last_push_approval")
                thread_resolution = thread_resolution or bool_field(parameters, "required_review_thread_resolution")
                allowed_merge_methods = parameters.get("allowed_merge_methods")
                if isinstance(allowed_merge_methods, list) and allowed_merge_methods:
                    merge_method_block = merge_method_block or ("merge" not in allowed_merge_methods)
                required_reviewers = parameters.get("required_reviewers")
                if required_reviewers is None:
                    continue
                if not isinstance(required_reviewers, list):
                    errors.append(f"{branch}: pull_request.required_reviewers must be an array when present")
                    continue
                required_reviewers_present = required_reviewers_present or bool(required_reviewers)
            if max_approvals != expected_native_approvals:
                errors.append(
                    f"{branch}: required_approving_review_count={max_approvals} expected={expected_native_approvals}"
                )
            if stale_review:
                errors.append(f"{branch}: dismiss_stale_reviews_on_push must stay disabled")
            if code_owner_review:
                errors.append(f"{branch}: require_code_owner_review must stay disabled")
            if last_push_approval:
                errors.append(f"{branch}: require_last_push_approval must stay disabled")
            if thread_resolution:
                errors.append(f"{branch}: required_review_thread_resolution must stay disabled")
            if declared_required_reviewers == [] and required_reviewers_present:
                errors.append(f"{branch}: required_reviewers must stay empty")
            if allow_merge_commits and merge_method_block:
                errors.append(f"{branch}: merge commits must remain allowed")

    if enforcement_mode not in {"github-native", "required-check"}:
        errors.append(f"{branch}: unsupported review_policy.enforcement.mode={enforcement_mode!r}")
    elif enforcement_mode == "github-native":
        if review_enforcement.get("bypass_mode") != "pull-request-only":
            errors.append(f"{branch}: review_policy bypass must stay pull-request-only")
    else:
        check_name = review_enforcement.get("check_name")
        if not isinstance(check_name, str) or not check_name:
            errors.append(f"{branch}: review_policy.enforcement.check_name must be set for required-check mode")

    live_required_checks, live_integrations, live_strict_values = normalize_required_status_checks(
        grouped.get("required_status_checks", [])
    )
    if live_required_checks != required_checks:
        missing = sorted(set(required_checks) - set(live_required_checks))
        unexpected = sorted(set(live_required_checks) - set(required_checks))
        details: list[str] = []
        if missing:
            details.append(f"missing={', '.join(missing)}")
        if unexpected:
            details.append(f"unexpected={', '.join(unexpected)}")
        if not details:
            details.append("required status check order/content drifted")
        errors.append(f"{branch}: required_status_checks drift ({'; '.join(details)})")

    if expected_strict_status_checks is not None:
        if live_strict_values != {expected_strict_status_checks}:
            errors.append(
                f"{branch}: strict_required_status_checks_policy={sorted(live_strict_values)} expected={expected_strict_status_checks}"
            )

    if normalized_expected_integrations:
        integration_errors: list[str] = []
        live_contexts = set(live_integrations)
        missing_contexts = sorted(set(normalized_expected_integrations) - live_contexts)
        unexpected_contexts = sorted(live_contexts - set(normalized_expected_integrations))
        for context in missing_contexts:
            integration_errors.append(f"{context}: missing")
        for context in unexpected_contexts:
            integration_errors.append(f"{context}: unexpected")
        for context, expected_integration in sorted(normalized_expected_integrations.items()):
            if context not in live_integrations:
                continue
            actual_integrations = live_integrations[context]
            allowed_integrations = {expected_integration}
            compat_integrations = source_compat_waivers.get(context, set())
            if compat_integrations:
                allowed_integrations |= compat_integrations
            if not actual_integrations:
                integration_errors.append(f"{context}: missing integration source")
                continue
            if actual_integrations.issubset(allowed_integrations):
                if actual_integrations != {expected_integration} and compat_integrations:
                    notes.append(
                        f"{branch}: {context} is using compatibility status sources {sorted(actual_integrations, key=lambda item: (-1 if item is None else item))}"
                    )
                continue
            integration_errors.append(
                f"{context}: expected one of {sorted(allowed_integrations, key=lambda item: (-1 if item is None else item))} actual={sorted(actual_integrations, key=lambda item: (-1 if item is None else item))}"
            )
        if integration_errors:
            errors.append(
                f"{branch}: required_status_check integrations drift ({'; '.join(integration_errors)})"
            )

    if bypass_reason:
        notes.append(f"{branch}: bypass actor verification waived explicitly ({bypass_reason})")
    else:
        errors.append(
            f"{branch}: bypass actor verification unavailable without explicit waiver"
        )

    return errors, notes


def main() -> int:
    args = parse_args()
    if should_skip(args.mode):
        return 0

    try:
        declaration = load_declaration(args.declaration)
        branches = choose_branches(declaration, args.branch)
        owner, repo = split_repo(args.repo)
        rules_fixture = json.loads(Path(args.rules_file).read_text()) if args.rules_file else None
        errors: list[str] = []
        notes: list[str] = []
        checked_rules: dict[str, list[str]] = {}
        for branch in branches:
            rules = extract_rules(rules_fixture if rules_fixture is not None else fetch_branch_rules(args.api_root, owner, repo, branch))
            checked_rules[branch] = sorted({rule.get("type", "") for rule in rules})
            branch_errors, branch_notes = validate_rules(declaration, rules, branch)
            errors.extend(branch_errors)
            notes.extend(branch_notes)
    except ValidationError as exc:
        print(f"[live-quality-gates] {exc}", file=sys.stderr)
        return 1

    if errors:
        print("[live-quality-gates] drift detected:", file=sys.stderr)
        for item in errors:
            print(f"- {item}", file=sys.stderr)
        return 1

    print(
        json.dumps(
            {
                "status": "ok",
                "repo": args.repo,
                "branches": branches,
                "checked_rules": checked_rules,
                "notes": [
                    "Validated effective branch rules via GET /repos/{owner}/{repo}/rules/branches/{branch} or a local fixture.",
                    *notes,
                ],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
