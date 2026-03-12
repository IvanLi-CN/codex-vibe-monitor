#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib.util
import json
import re
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


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("metadata_gate", path)
    if spec is None or spec.loader is None:
        raise ContractError(f"Unable to load module from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def require_text(text: str, needle: str, where: str) -> None:
    require(needle in text, f"{where}: missing required text {needle!r}")


def forbid_text(text: str, needle: str, where: str) -> None:
    require(needle not in text, f"{where}: unexpected text {needle!r}")


def extract_js_int(text: str, constant: str, where: str) -> int:
    match = re.search(rf"\bconst\s+{re.escape(constant)}\s*=\s*(\d+)\s*;", text)
    require(match is not None, f"{where}: missing integer constant {constant}")
    return int(match.group(1))


def extract_js_string_set(text: str, constant: str, where: str) -> set[str]:
    match = re.search(rf"\bconst\s+{re.escape(constant)}\s*=\s*new Set\(\[(.*?)\]\);", text, re.S)
    require(match is not None, f"{where}: missing string-set constant {constant}")
    return set(re.findall(r"'([^']+)'", match.group(1)))


def validate_quality_gates(path: Path) -> None:
    payload = json.loads(path.read_text())
    branch_policy = payload["policy"]["branch_protection"]
    require(branch_policy.get("require_merge_queue") is False, "quality-gates.json: require_merge_queue must be false")
    require(
        branch_policy.get("disallow_branch_deletions") is True,
        "quality-gates.json: disallow_branch_deletions must be true",
    )
    require(
        branch_policy.get("disallow_force_pushes") is True,
        "quality-gates.json: disallow_force_pushes must be true",
    )
    status_check_policy = branch_policy.get("required_status_checks")
    require(isinstance(status_check_policy, dict), "quality-gates.json: branch_protection.required_status_checks must be an object")
    require(status_check_policy.get("strict") is True, "quality-gates.json: required_status_checks.strict must be true")
    integrations = status_check_policy.get("integrations")
    require(isinstance(integrations, dict), "quality-gates.json: required_status_checks.integrations must be an object")
    required_checks = set(payload.get("required_checks", []))
    require(required_checks == REQUIRED_CHECKS, f"quality-gates.json: required_checks drifted: {sorted(required_checks)}")
    require(
        set(integrations) == REQUIRED_CHECKS,
        f"quality-gates.json: required_status_checks.integrations drifted: {sorted(integrations)}",
    )
    require(
        all(value == 15368 for value in integrations.values()),
        f"quality-gates.json: required_status_checks.integrations must stay on GitHub Actions: {integrations}",
    )

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
        ("CI Pipeline", ("Lint & Format Check", "Backend Tests", "Build Artifacts", "Release Meta (intent + version + tags)", "Build + Smoke + Push Candidate (linux/amd64)", "Build + Smoke + Push Candidate (linux/arm64)")) in expected,
        "quality-gates.json: expected_pr_workflows must include CI Pipeline workflow/jobs",
    )


def validate_ci(path: Path) -> None:
    text = path.read_text()
    require_text(text, "merge_group:", "ci.yml")
    require_text(text, "checks_requested", "ci.yml")
    require_text(text, "github.event_name != 'push'", "ci.yml")
    forbid_text(text, "pull_request_target:", "ci.yml")


def validate_label_gate(path: Path) -> None:
    text = path.read_text()
    require(re.search(r"(?m)^\s*pull_request:\s*$", text) is not None, "label-gate.yml: must trigger on pull_request")
    require(re.search(r"(?m)^\s*merge_group:\s*$", text) is None, "label-gate.yml: merge_group must stay disabled")
    require(re.search(r"(?m)^\s*pull_request_target:\s*$", text) is None, "label-gate.yml: must not trigger on pull_request_target")
    require(re.search(r"(?m)^\s*workflow_dispatch:\s*$", text) is None, "label-gate.yml: workflow_dispatch must stay disabled")
    require_text(text, "edited", "label-gate.yml")
    require_text(text, "contents: read", "label-gate.yml")
    require_text(text, "actions/github-script@v8", "label-gate.yml")
    require_text(text, "issues.get", "label-gate.yml")
    require_text(text, "channel:stable", "label-gate.yml")
    forbid_text(text, "actions/checkout", "label-gate.yml")
    forbid_text(text, "metadata_gate.py", "label-gate.yml")
    forbid_text(text, "createCommitStatus", "label-gate.yml")
    forbid_text(text, "GET /repos/{owner}/{repo}/commits/{commit_sha}/pulls", "label-gate.yml")
    forbid_text(text, "context.ref || process.env.GITHUB_REF", "label-gate.yml")


def validate_review_policy(path: Path, declaration: dict[str, Any]) -> None:
    text = path.read_text()
    review_policy = declaration["policy"]["review_policy"]
    require(re.search(r"(?m)^\s*pull_request:\s*$", text) is not None, "review-policy.yml: must trigger on pull_request")
    require(re.search(r"(?m)^\s*pull_request_review:\s*$", text) is not None, "review-policy.yml: must trigger on pull_request_review")
    require(re.search(r"(?m)^\s*merge_group:\s*$", text) is None, "review-policy.yml: merge_group must stay disabled")
    require(re.search(r"(?m)^\s*pull_request_target:\s*$", text) is None, "review-policy.yml: must not trigger on pull_request_target")
    require(re.search(r"(?m)^\s*workflow_dispatch:\s*$", text) is None, "review-policy.yml: workflow_dispatch must stay disabled")
    require_text(text, "edited", "review-policy.yml")
    require_text(text, "contents: read", "review-policy.yml")
    require_text(text, "actions/github-script@v8", "review-policy.yml")
    require_text(text, "GET /repos/{owner}/{repo}/collaborators/{username}/permission", "review-policy.yml")
    require_text(text, "GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews", "review-policy.yml")
    forbid_text(text, "actions/checkout", "review-policy.yml")
    forbid_text(text, "metadata_gate.py", "review-policy.yml")
    forbid_text(text, "createCommitStatus", "review-policy.yml")
    forbid_text(text, "statuses: write", "review-policy.yml")
    forbid_text(text, "GET /repos/{owner}/{repo}/commits/{commit_sha}/pulls", "review-policy.yml")
    forbid_text(text, "context.ref || process.env.GITHUB_REF", "review-policy.yml")
    require(
        extract_js_int(text, "reviewRequiredApprovals", "review-policy.yml") == review_policy["required_approvals"],
        "review-policy.yml: reviewRequiredApprovals must match quality-gates.json",
    )
    require(
        extract_js_string_set(text, "reviewExemptPermissions", "review-policy.yml")
        == set(review_policy["exempt_author_permissions"]),
        "review-policy.yml: reviewExemptPermissions must match quality-gates.json",
    )
    require(
        extract_js_string_set(text, "reviewAllowedPermissions", "review-policy.yml")
        == set(review_policy["allowed_reviewer_permissions"]),
        "review-policy.yml: reviewAllowedPermissions must match quality-gates.json",
    )
    owner_exempt = "key === context.repo.owner.toLowerCase()" in text
    require(
        owner_exempt == bool(review_policy["exempt_repository_owner"]),
        "review-policy.yml: owner exemption must match quality-gates.json",
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
        module = load_module(metadata_script_path)
        validate_quality_gates(declaration_path)
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
