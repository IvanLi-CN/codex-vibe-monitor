#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

API_VERSION = "2022-11-28"
ALLOWED_INTENT_LABELS = frozenset(
    {
        "type:docs",
        "type:skip",
        "type:patch",
        "type:minor",
        "type:major",
    }
)
ALLOWED_CHANNEL_LABELS = frozenset({"channel:stable", "channel:rc"})
REVIEW_REQUIRED_APPROVALS = 1
REVIEW_EXEMPT_PERMISSIONS = frozenset({"admin", "maintain"})
REVIEW_ALLOWED_PERMISSIONS = frozenset({"write", "maintain", "admin"})


class GateError(RuntimeError):
    pass


class GitHubApiError(RuntimeError):
    def __init__(self, status: int, message: str) -> None:
        super().__init__(message)
        self.status = status


@dataclass(frozen=True)
class GateContext:
    gate: str
    owner: str
    repo: str
    api_root: str
    token: str
    event_name: str
    event_payload: dict[str, Any]
    manual_pull_number: int | None


class GitHubClient:
    def __init__(self, owner: str, repo: str, api_root: str, token: str) -> None:
        self.owner = owner
        self.repo = repo
        self.api_root = api_root.rstrip("/")
        self.token = token

    def request_json(self, path: str, query: dict[str, Any] | None = None) -> Any:
        url = self.api_root + path
        if query:
            url += "?" + urllib.parse.urlencode(query)
        headers = {
            "Accept": "application/vnd.github+json",
            "User-Agent": "codex-vibe-monitor-metadata-gate/1.0",
            "X-GitHub-Api-Version": API_VERSION,
        }
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        request = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.load(response)
        except urllib.error.HTTPError as exc:
            detail = exc.read().decode("utf-8", errors="replace")
            raise GitHubApiError(exc.code, detail or exc.reason) from exc
        except urllib.error.URLError as exc:
            raise GateError(f"GitHub API request failed: {exc.reason}") from exc

    def paginate(self, path: str, query: dict[str, Any] | None = None) -> list[dict[str, Any]]:
        items: list[dict[str, Any]] = []
        page = 1
        while True:
            payload = self.request_json(path, {**(query or {}), "per_page": 100, "page": page})
            if not isinstance(payload, list):
                raise GateError(f"Expected list payload from {path}, got {type(payload).__name__}")
            items.extend(item for item in payload if isinstance(item, dict))
            if len(payload) < 100:
                break
            page += 1
        return items


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Evaluate codex-vibe-monitor metadata gates from trusted sources.")
    parser.add_argument("gate", choices=("label", "review"))
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", ""))
    parser.add_argument("--api-root", default=os.environ.get("GITHUB_API_URL", "https://api.github.com"))
    parser.add_argument("--token", default=os.environ.get("GITHUB_TOKEN", ""))
    parser.add_argument("--event-name", default=os.environ.get("GITHUB_EVENT_NAME", ""))
    parser.add_argument("--event-path", default=os.environ.get("GITHUB_EVENT_PATH", ""))
    parser.add_argument("--pull-number", type=int, default=None)
    return parser.parse_args()


def parse_optional_int(value: str) -> int | None:
    if not value:
        return None
    try:
        parsed = int(value)
    except ValueError:
        return None
    return parsed if parsed > 0 else None


def load_event_payload(path: str) -> dict[str, Any]:
    if not path:
        return {}
    event_path = Path(path)
    if not event_path.is_file():
        return {}
    try:
        payload = json.loads(event_path.read_text())
    except json.JSONDecodeError as exc:
        raise GateError(f"Failed to parse event payload JSON: {exc}") from exc
    if not isinstance(payload, dict):
        raise GateError("GitHub event payload must be a JSON object")
    return payload


def split_repo(full_name: str) -> tuple[str, str]:
    owner, sep, repo = full_name.partition("/")
    if not sep or not owner or not repo:
        raise GateError("Repository must be in owner/name form")
    return owner, repo


def build_context(args: argparse.Namespace) -> GateContext:
    owner, repo = split_repo(args.repo)
    manual_pull_number = args.pull_number
    if manual_pull_number is None:
        manual_pull_number = parse_optional_int(os.environ.get("MANUAL_PULL_NUMBER", ""))
    if manual_pull_number is None:
        manual_pull_number = parse_optional_int(os.environ.get("INPUT_PULL_NUMBER", ""))
    payload = load_event_payload(args.event_path)
    return GateContext(
        gate=args.gate,
        owner=owner,
        repo=repo,
        api_root=args.api_root,
        token=args.token,
        event_name=args.event_name,
        event_payload=payload,
        manual_pull_number=manual_pull_number,
    )


def get_payload_value(payload: dict[str, Any], *path: str) -> Any:
    current: Any = payload
    for key in path:
        if not isinstance(current, dict):
            return None
        current = current.get(key)
    return current


def resolve_pull_numbers(context: GateContext, client: GitHubClient) -> list[int]:
    if context.event_name == "merge_group":
        raise GateError("merge_group is unsupported for PR-only metadata gates")

    payload_pull_number = get_payload_value(context.event_payload, "pull_request", "number")
    if isinstance(payload_pull_number, int) and payload_pull_number > 0:
        return [payload_pull_number]
    if context.manual_pull_number is not None:
        return [context.manual_pull_number]
    raise GateError(f"Missing valid pull request number for {context.gate} gate evaluation")


def describe_labels(labels: list[str]) -> str:
    if not labels:
        return "(none)"
    return ", ".join(sorted(set(labels)))


def evaluate_labels_from_names(labels: list[str]) -> tuple[bool, str]:
    type_labels = sorted({label for label in labels if label.startswith("type:")})
    channel_labels = sorted({label for label in labels if label.startswith("channel:")})
    unknown_type_labels = [label for label in type_labels if label not in ALLOWED_INTENT_LABELS]
    unknown_channel_labels = [label for label in channel_labels if label not in ALLOWED_CHANNEL_LABELS]
    selected_type_labels = [label for label in type_labels if label in ALLOWED_INTENT_LABELS]
    selected_channel_labels = [label for label in channel_labels if label in ALLOWED_CHANNEL_LABELS]
    problems: list[str] = []
    if unknown_type_labels:
        problems.append(f"Unknown type label(s): {', '.join(unknown_type_labels)}")
    if unknown_channel_labels:
        problems.append(f"Unknown channel label(s): {', '.join(unknown_channel_labels)}")
    if len(selected_type_labels) != 1:
        problems.append(f"Expected exactly 1 type:* label, got {len(selected_type_labels)}")
    if len(selected_channel_labels) != 1:
        problems.append(f"Expected exactly 1 channel:* label, got {len(selected_channel_labels)}")
    if problems:
        return False, f"{'; '.join(problems)} | labels={describe_labels(labels)}"
    return True, f"Labels OK: {selected_type_labels[0]} + {selected_channel_labels[0]}"


def fetch_issue_labels(client: GitHubClient, pull_number: int) -> list[str]:
    payload = client.request_json(f"/repos/{client.owner}/{client.repo}/issues/{pull_number}")
    if not isinstance(payload, dict):
        raise GateError(f"Issue payload for PR #{pull_number} must be an object")
    labels = payload.get("labels") or []
    if not isinstance(labels, list):
        raise GateError(f"Labels payload for PR #{pull_number} must be a list")
    names = [str(label.get("name")) for label in labels if isinstance(label, dict) and label.get("name")]
    return sorted(set(names))



def write_step_summary(lines: list[str]) -> None:
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY", "")
    if not summary_path:
        return
    with open(summary_path, "a", encoding="utf-8") as handle:
        handle.write("\n".join(lines) + "\n")


def run_label_gate(context: GateContext, client: GitHubClient) -> int:
    pull_numbers = resolve_pull_numbers(context, client)
    results: list[dict[str, Any]] = []
    for pull_number in pull_numbers:
        labels = fetch_issue_labels(client, pull_number)
        passed, description = evaluate_labels_from_names(labels)
        results.append(
            {
                "pull_number": pull_number,
                "passed": passed,
                "labels": labels,
                "description": description,
            }
        )

    summary_lines = [
        "## PR label gate",
        f"- Scope: `{context.event_name}`",
        "",
    ]
    for result in results:
        summary_lines.append(
            f"- PR #{result['pull_number']}: {'pass' if result['passed'] else 'fail'} - {result['description']}"
        )
    write_step_summary(summary_lines)

    failed = [result for result in results if not result["passed"]]
    if failed:
        print(
            " | ".join(
                f"PR #{result['pull_number']}: {result['description']}" for result in failed
            ),
            file=sys.stderr,
        )
        return 1

    print(f"metadata-gate[label]: validated {len(results)} pull request(s)")
    return 0


def get_permission(client: GitHubClient, owner: str, username: str) -> str:
    if not username:
        return "none"
    if username.lower() == owner.lower():
        return "admin"
    path = (
        f"/repos/{client.owner}/{client.repo}/collaborators/"
        f"{urllib.parse.quote(username, safe='')}/permission"
    )
    try:
        payload = client.request_json(path)
    except GitHubApiError as exc:
        if exc.status == 404:
            return "none"
        raise
    if not isinstance(payload, dict):
        raise GateError(f"Permission payload for @{username} must be an object")
    permission = payload.get("permission")
    return str(permission) if permission else "none"


def list_approvals(client: GitHubClient, owner: str, pull_number: int, author: str) -> list[str]:
    reviews = client.paginate(f"/repos/{client.owner}/{client.repo}/pulls/{pull_number}/reviews")
    reviews.sort(key=lambda review: str(review.get("submitted_at") or ""))
    latest_by_user: dict[str, dict[str, Any]] = {}
    decision_states = {"APPROVED", "CHANGES_REQUESTED", "DISMISSED"}
    for review in reviews:
        user = review.get("user") or {}
        reviewer = user.get("login") if isinstance(user, dict) else None
        state = review.get("state")
        if not isinstance(reviewer, str) or not reviewer or state == "PENDING":
            continue
        if state in decision_states:
            latest_by_user[reviewer] = review

    approvals: list[str] = []
    for reviewer, review in sorted(latest_by_user.items()):
        if review.get("state") != "APPROVED" or reviewer == author:
            continue
        permission = get_permission(client, owner, reviewer)
        if permission not in REVIEW_ALLOWED_PERMISSIONS:
            continue
        approvals.append(f"@{reviewer} ({permission})")
    return approvals


def run_review_gate(context: GateContext, client: GitHubClient) -> int:
    pull_numbers = resolve_pull_numbers(context, client)
    results: list[dict[str, Any]] = []
    for pull_number in pull_numbers:
        payload = client.request_json(f"/repos/{client.owner}/{client.repo}/pulls/{pull_number}")
        if not isinstance(payload, dict):
            raise GateError(f"Pull request payload for PR #{pull_number} must be an object")
        user = payload.get("user") or {}
        author = user.get("login") if isinstance(user, dict) else None
        if not isinstance(author, str) or not author:
            raise GateError(f"PR #{pull_number} is missing an author")
        author_permission = get_permission(client, context.owner, author)
        if author_permission in REVIEW_EXEMPT_PERMISSIONS:
            results.append(
                {
                    "pull_number": pull_number,
                    "author": author,
                    "author_permission": author_permission,
                    "approvals": [],
                    "passed": True,
                    "description": f"Author @{author} has {author_permission} permission; approval not required.",
                }
            )
            continue

        approvals = list_approvals(client, context.owner, pull_number, author)
        passed = len(approvals) >= REVIEW_REQUIRED_APPROVALS
        results.append(
            {
                "pull_number": pull_number,
                "author": author,
                "author_permission": author_permission,
                "approvals": approvals,
                "passed": passed,
                "description": (
                    f"Approval satisfied by {', '.join(approvals)}."
                    if passed
                    else (
                        f"Author @{author} has {author_permission} permission; at least {REVIEW_REQUIRED_APPROVALS} approval(s) "
                        f"from write/maintain/admin reviewer(s) required."
                    )
                ),
            }
        )

    summary_lines = [
        "## Review policy gate",
        f"- Scope: `{context.event_name}`",
        "",
    ]
    for result in results:
        summary_lines.extend(
            [
                f"- PR #{result['pull_number']}: {'pass' if result['passed'] else 'fail'}",
                f"  - Author: @{result['author']} ({result['author_permission']})",
                f"  - {result['description']}",
            ]
        )
    write_step_summary(summary_lines)

    failed = [result for result in results if not result["passed"]]
    if failed:
        print(
            " | ".join(
                f"PR #{result['pull_number']}: {result['description']}" for result in failed
            ),
            file=sys.stderr,
        )
        return 1

    print(f"metadata-gate[review]: validated {len(results)} pull request(s)")
    return 0


def main() -> int:
    args = parse_args()
    try:
        context = build_context(args)
        client = GitHubClient(context.owner, context.repo, context.api_root, context.token)
        if context.gate == "label":
            return run_label_gate(context, client)
        return run_review_gate(context, client)
    except (GateError, GitHubApiError) as exc:
        print(f"metadata-gate[{args.gate}]: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
