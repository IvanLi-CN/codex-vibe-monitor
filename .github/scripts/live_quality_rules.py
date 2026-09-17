from __future__ import annotations

from typing import Any


def _policy_values(declaration: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any], list[str], bool, bool, bool, bool, bool, list[str], dict[str, Any], bool | None, dict[str, int], str, int]:
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
    allow_merge_commits = branch_policy.get("allow_merge_commits", True)
    if not isinstance(allow_merge_commits, bool):
        raise ValidationError("policy.branch_protection.allow_merge_commits must be a boolean")
    declared_required_reviewers = branch_policy.get("required_reviewers", [])
    if declared_required_reviewers is None:
        declared_required_reviewers = []
    if not isinstance(declared_required_reviewers, list):
        raise ValidationError("policy.branch_protection.required_reviewers must be a JSON array")
    if declared_required_reviewers:
        raise ValidationError("policy.branch_protection.required_reviewers only supports an empty array in this repository")
    status_check_policy = branch_policy.get("required_status_checks", {}) or {}
    if not isinstance(status_check_policy, dict):
        raise ValidationError("policy.branch_protection.required_status_checks must be a JSON object")
    expected_strict = status_check_policy.get("strict")
    if expected_strict is not None and not isinstance(expected_strict, bool):
        raise ValidationError("policy.branch_protection.required_status_checks.strict must be a boolean")
    raw_integrations = status_check_policy.get("integrations", {}) or {}
    if not isinstance(raw_integrations, dict):
        raise ValidationError("policy.branch_protection.required_status_checks.integrations must be a JSON object")
    integrations: dict[str, int] = {}
    for context, integration in raw_integrations.items():
        if not isinstance(context, str) or not context:
            raise ValidationError("policy.branch_protection.required_status_checks.integrations keys must be strings")
        if not isinstance(integration, int):
            raise ValidationError("policy.branch_protection.required_status_checks.integrations values must be integers")
        integrations[context] = integration
    enforcement_mode = str(review_enforcement.get("mode", ""))
    expected_approvals = int(review_policy.get("required_approvals", 0)) if enforcement_mode == "github-native" else 0
    return (
        policy,
        branch_policy,
        review_enforcement,
        required_checks,
        bool(policy.get("require_signed_commits", False)),
        bool(branch_policy.get("require_pull_request", False)),
        bool(branch_policy.get("disallow_branch_deletions", False)),
        bool(branch_policy.get("disallow_force_pushes", False)),
        bool(branch_policy.get("require_merge_queue", False)),
        declared_required_reviewers,
        status_check_policy,
        expected_strict,
        integrations,
        enforcement_mode,
        expected_approvals,
    )


def _validate_presence(branch: str, grouped: dict[str, list[dict[str, Any]]], branch_policy: dict[str, Any], policy_flags: tuple[bool, bool, bool, bool, bool]) -> list[str]:
    require_signed, disallow_deletions, disallow_force_pushes, require_queue, allow_merge_commits = policy_flags
    errors: list[str] = []
    expected = ((disallow_deletions, "deletion"), (disallow_force_pushes, "non_fast_forward"), (require_queue, "merge_queue"))
    for required, rule_type in expected:
        present = rule_type in grouped
        if required and not present:
            errors.append(f"{branch}: missing {rule_type} rule")
        if not required and present:
            errors.append(f"{branch}: unexpected {rule_type} rule")
    if require_signed and "required_signatures" not in grouped:
        errors.append(f"{branch}: missing required_signatures rule")
    if allow_merge_commits and "required_linear_history" in grouped:
        errors.append(f"{branch}: merge commits must remain allowed")
    if branch_policy.get("disallow_direct_pushes") and "pull_request" not in grouped:
        errors.append(f"{branch}: missing pull_request rule required to block direct pushes")
    return errors


def _validate_pull_request(branch: str, grouped: dict[str, list[dict[str, Any]]], required: bool, approvals: int, declared_reviewers: list[str], allow_merge_commits: bool) -> list[str]:
    if not required:
        return []
    pull_request_rules = grouped.get("pull_request", [])
    if not pull_request_rules:
        return [f"{branch}: missing pull_request rule"]
    max_approvals = 0
    flags = {"stale": False, "owner": False, "last_push": False, "threads": False, "merge_block": False, "reviewers": False}
    errors: list[str] = []
    for rule in pull_request_rules:
        parameters = rule.get("parameters") or {}
        if not isinstance(parameters, dict):
            continue
        value = parameters.get("required_approving_review_count", 0)
        if isinstance(value, bool):
            value = int(value)
        if isinstance(value, int):
            max_approvals = max(max_approvals, value)
        flags["stale"] |= bool_field(parameters, "dismiss_stale_reviews_on_push")
        flags["owner"] |= bool_field(parameters, "require_code_owner_review")
        flags["last_push"] |= bool_field(parameters, "require_last_push_approval")
        flags["threads"] |= bool_field(parameters, "required_review_thread_resolution")
        methods = parameters.get("allowed_merge_methods")
        if isinstance(methods, list) and methods:
            flags["merge_block"] |= "merge" not in methods
        reviewers = parameters.get("required_reviewers")
        if reviewers is None:
            continue
        if not isinstance(reviewers, list):
            errors.append(f"{branch}: pull_request.required_reviewers must be an array when present")
            continue
        flags["reviewers"] |= bool(reviewers)
    if max_approvals != approvals:
        errors.append(f"{branch}: required_approving_review_count={max_approvals} expected={approvals}")
    messages = (("stale", "dismiss_stale_reviews_on_push must stay disabled"), ("owner", "require_code_owner_review must stay disabled"), ("last_push", "require_last_push_approval must stay disabled"), ("threads", "required_review_thread_resolution must stay disabled"))
    errors.extend(f"{branch}: {message}" for flag, message in messages if flags[flag])
    if not declared_reviewers and flags["reviewers"]:
        errors.append(f"{branch}: required_reviewers must stay empty")
    if allow_merge_commits and flags["merge_block"]:
        errors.append(f"{branch}: merge commits must remain allowed")
    return errors


def _validate_enforcement(branch: str, enforcement: dict[str, Any], mode: str) -> list[str]:
    if mode not in {"github-native", "required-check"}:
        return [f"{branch}: unsupported review_policy.enforcement.mode={mode!r}"]
    if mode == "github-native" and enforcement.get("bypass_mode") != "pull-request-only":
        return [f"{branch}: review_policy bypass must stay pull-request-only"]
    if mode == "required-check" and not isinstance(enforcement.get("check_name"), str):
        return [f"{branch}: review_policy.enforcement.check_name must be set for required-check mode"]
    return []


def _validate_status(branch: str, grouped: dict[str, list[dict[str, Any]]], required: list[str], expected_strict: bool | None, expected_integrations: dict[str, int]) -> list[str]:
    live, integrations, strict_values = normalize_required_status_checks(grouped.get("required_status_checks", []))
    errors: list[str] = []
    if live != required:
        missing = sorted(set(required) - set(live))
        unexpected = sorted(set(live) - set(required))
        details = [f"missing={', '.join(missing)}" for _ in [0] if missing]
        details.extend(f"unexpected={', '.join(unexpected)}" for _ in [0] if unexpected)
        errors.append(f"{branch}: required_status_checks drift ({'; '.join(details or ['required status check order/content drifted'])})")
    if expected_strict is not None and strict_values != {expected_strict}:
        errors.append(f"{branch}: strict_required_status_checks_policy={sorted(strict_values)} expected={expected_strict}")
    integration_errors: list[str] = []
    live_contexts = set(integrations)
    for context in sorted(set(expected_integrations) - live_contexts):
        integration_errors.append(f"{context}: missing")
    for context in sorted(live_contexts - set(expected_integrations)):
        integration_errors.append(f"{context}: unexpected")
    for context, expected in sorted(expected_integrations.items()):
        if context not in integrations:
            continue
        actual = integrations[context]
        if not actual:
            integration_errors.append(f"{context}: missing integration source")
        elif actual != {expected}:
            integration_errors.append(f"{context}: expected one of {[expected]} actual={sorted(actual, key=lambda item: (-1 if item is None else item))}")
    if integration_errors:
        errors.append(f"{branch}: required_status_check integrations drift ({'; '.join(integration_errors)})")
    return errors


def validate_rules(declaration: dict[str, Any], rules: list[dict[str, Any]], rulesets: list[dict[str, Any]], branch: str) -> tuple[list[str], list[str], str]:
    values = _policy_values(declaration)
    _, branch_policy, enforcement, required, signed, pull_request, deletions, force_pushes, queue, reviewers, _status_policy, strict, integrations, mode, approvals = values
    grouped: dict[str, list[dict[str, Any]]] = {}
    for rule in rules:
        grouped.setdefault(rule.get("type", ""), []).append(rule)
    policy_flags = (signed, deletions, force_pushes, queue, bool(branch_policy.get("allow_merge_commits", True)))
    errors = _validate_presence(branch, grouped, branch_policy, policy_flags)
    errors.extend(_validate_pull_request(branch, grouped, pull_request, approvals, reviewers, bool(branch_policy.get("allow_merge_commits", True))))
    errors.extend(_validate_enforcement(branch, enforcement, mode))
    errors.extend(_validate_status(branch, grouped, required, strict, integrations))
    bypass_errors, notes, status = validate_bypass_actors(declaration, rulesets, branch)
    return errors + bypass_errors, notes, status
