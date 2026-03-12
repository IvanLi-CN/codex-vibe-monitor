#!/usr/bin/env python3
from __future__ import annotations

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


def validate_quality_gates(path: Path) -> None:
    payload = json.loads(path.read_text())
    branch_policy = payload["policy"]["branch_protection"]
    require(branch_policy.get("require_merge_queue") is False, "quality-gates.json: require_merge_queue must be false")
    required_checks = set(payload.get("required_checks", []))
    require(required_checks == REQUIRED_CHECKS, f"quality-gates.json: required_checks drifted: {sorted(required_checks)}")

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


def validate_review_policy(path: Path) -> None:
    text = path.read_text()
    require(re.search(r"(?m)^\s*pull_request:\s*$", text) is not None, "review-policy.yml: must trigger on pull_request")
    require(re.search(r"(?m)^\s*pull_request_review:\s*$", text) is not None, "review-policy.yml: must trigger on pull_request_review")
    require(re.search(r"(?m)^\s*merge_group:\s*$", text) is None, "review-policy.yml: merge_group must stay disabled")
    require(re.search(r"(?m)^\s*pull_request_target:\s*$", text) is None, "review-policy.yml: must not trigger on pull_request_target")
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
    repo_root = Path(__file__).resolve().parents[2]
    scripts_dir = repo_root / ".github" / "scripts"

    try:
        module = load_module(scripts_dir / "metadata_gate.py")
        validate_quality_gates(repo_root / ".github" / "quality-gates.json")
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
