#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Sequence

API_VERSION = "2022-11-28"
MAX_OUTPUT_TEXT = 60000


class TrustedGateError(RuntimeError):
    pass


@dataclass(frozen=True)
class GateContext:
    owner: str
    repo: str
    head_sha: str
    details_url: str | None


@dataclass(frozen=True)
class CommandResult:
    name: str
    returncode: int
    stdout: str
    stderr: str

    @property
    def ok(self) -> bool:
        return self.returncode == 0


class GitHubChecksClient:
    def __init__(self, owner: str, repo: str, api_root: str, token: str) -> None:
        self.owner = owner
        self.repo = repo
        self.api_root = api_root.rstrip("/")
        self.token = token

    def request_json(self, method: str, path: str, body: dict[str, Any] | None = None) -> Any:
        url = self.api_root + path
        headers = {
            "Accept": "application/vnd.github+json",
            "Content-Type": "application/json",
            "User-Agent": "codex-vibe-monitor-trusted-metadata-check/1.0",
            "X-GitHub-Api-Version": API_VERSION,
        }
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        data = None if body is None else json.dumps(body).encode("utf-8")
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                if response.length == 0:
                    return None
                return json.load(response)
        except urllib.error.HTTPError as exc:
            detail = exc.read().decode("utf-8", errors="replace")
            raise TrustedGateError(f"GitHub API request failed ({exc.code}): {detail or exc.reason}") from exc
        except urllib.error.URLError as exc:
            raise TrustedGateError(f"GitHub API request failed: {exc.reason}") from exc

    def create_check_run(
        self,
        *,
        name: str,
        head_sha: str,
        started_at: str,
        details_url: str | None,
        summary: str,
        external_id: str,
    ) -> int:
        payload: dict[str, Any] = {
            "name": name,
            "head_sha": head_sha,
            "status": "in_progress",
            "started_at": started_at,
            "external_id": external_id,
            "output": {
                "title": name,
                "summary": summary,
            },
        }
        if details_url:
            payload["details_url"] = details_url
        response = self.request_json("POST", f"/repos/{self.owner}/{self.repo}/check-runs", payload)
        if not isinstance(response, dict) or not isinstance(response.get("id"), int):
            raise TrustedGateError("GitHub API returned an invalid check-run creation payload")
        return int(response["id"])

    def update_check_run(
        self,
        *,
        check_run_id: int,
        title: str,
        conclusion: str,
        completed_at: str,
        summary: str,
        text: str,
    ) -> None:
        payload = {
            "status": "completed",
            "conclusion": conclusion,
            "completed_at": completed_at,
            "output": {
                "title": title,
                "summary": summary,
                "text": text,
            },
        }
        self.request_json("PATCH", f"/repos/{self.owner}/{self.repo}/check-runs/{check_run_id}", payload)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Publish trusted metadata gate results as a GitHub check run.")
    parser.add_argument("--gate", choices=("label",), required=True)
    parser.add_argument("--check-name", required=True)
    parser.add_argument("--candidate-root", required=True)
    parser.add_argument("--trusted-root", required=True)
    parser.add_argument("--api-root", default=os.environ.get("GITHUB_API_URL", "https://api.github.com"))
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", ""))
    parser.add_argument("--token", default=os.environ.get("GITHUB_TOKEN", ""))
    parser.add_argument("--event-path", default=os.environ.get("GITHUB_EVENT_PATH", ""))
    return parser.parse_args()


def split_repo(repo: str) -> tuple[str, str]:
    owner, sep, name = repo.partition("/")
    if not sep or not owner or not name:
        raise TrustedGateError("Repository must be in owner/name form")
    return owner, name


def load_event_payload(path: str) -> dict[str, Any]:
    event_path = Path(path)
    if not path or not event_path.is_file():
        raise TrustedGateError("Missing GitHub event payload")
    try:
        payload = json.loads(event_path.read_text())
    except json.JSONDecodeError as exc:
        raise TrustedGateError(f"Failed to parse GitHub event payload: {exc}") from exc
    if not isinstance(payload, dict):
        raise TrustedGateError("GitHub event payload must be a JSON object")
    return payload


def build_context(args: argparse.Namespace) -> GateContext:
    owner, repo = split_repo(args.repo)
    payload = load_event_payload(args.event_path)
    pull_request = payload.get("pull_request")
    if not isinstance(pull_request, dict):
        raise TrustedGateError("Trusted metadata check requires a pull_request payload")
    head = pull_request.get("head")
    if not isinstance(head, dict):
        raise TrustedGateError("pull_request.head is missing")
    head_sha = head.get("sha")
    if not isinstance(head_sha, str) or not head_sha:
        raise TrustedGateError("pull_request.head.sha is missing")
    run_id = os.environ.get("GITHUB_RUN_ID", "")
    server_url = os.environ.get("GITHUB_SERVER_URL", "https://github.com").rstrip("/")
    details_url = f"{server_url}/{args.repo}/actions/runs/{run_id}" if run_id else None
    return GateContext(owner=owner, repo=repo, head_sha=head_sha, details_url=details_url)


def iso_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def run_command(command: Sequence[str], *, cwd: Path | None = None) -> CommandResult:
    result = subprocess.run(
        list(command),
        cwd=str(cwd) if cwd is not None else None,
        capture_output=True,
        text=True,
        check=False,
    )
    return CommandResult(
        name=Path(command[1]).name if len(command) > 1 else command[0],
        returncode=result.returncode,
        stdout=result.stdout.strip(),
        stderr=result.stderr.strip(),
    )


def summarize_command(label: str, result: CommandResult) -> str:
    state = "pass" if result.ok else "fail"
    return f"- {label}: {state}"


def truncate_text(text: str) -> str:
    if len(text) <= MAX_OUTPUT_TEXT:
        return text
    overflow = len(text) - MAX_OUTPUT_TEXT
    return text[:MAX_OUTPUT_TEXT] + f"\n\n...[truncated {overflow} chars]"


def build_output(check_name: str, results: list[tuple[str, CommandResult]]) -> tuple[str, str, str]:
    summary_lines = [f"Trusted gate: `{check_name}`", ""]
    text_sections: list[str] = []
    for label, result in results:
        summary_lines.append(summarize_command(label, result))
        body_parts = [f"{label} exit={result.returncode}"]
        if result.stdout:
            body_parts.append(f"stdout:\n{result.stdout}")
        if result.stderr:
            body_parts.append(f"stderr:\n{result.stderr}")
        text_sections.append("\n\n".join(body_parts))
    success = all(result.ok for _, result in results)
    summary = "\n".join(summary_lines)
    text = truncate_text("\n\n---\n\n".join(text_sections))
    title = check_name if success else f"{check_name} failed"
    return title, summary, text


def external_id(gate: str) -> str:
    run_id = os.environ.get("GITHUB_RUN_ID", "local")
    attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "1")
    return f"trusted-metadata:{gate}:{run_id}:{attempt}"


def run_label_gate(
    args: argparse.Namespace,
    runner: Callable[[Sequence[str]], CommandResult],
) -> list[tuple[str, CommandResult]]:
    trusted_root = Path(args.trusted_root).resolve()
    candidate_root = Path(args.candidate_root).resolve()
    contract_script = trusted_root / ".github/scripts/check_quality_gates_contract.py"
    metadata_script = trusted_root / ".github/scripts/metadata_gate.py"
    declaration = candidate_root / ".github/quality-gates.json"
    results = [
        (
            "Contract",
            runner(
                [
                    sys.executable,
                    str(contract_script),
                    "--repo-root",
                    str(candidate_root),
                    "--declaration",
                    str(declaration),
                    "--metadata-script",
                    str(metadata_script),
                ]
            ),
        ),
        (
            "Labels",
            runner(
                [
                    sys.executable,
                    str(metadata_script),
                    "label",
                ]
            ),
        ),
    ]
    return results


def execute_gate(
    args: argparse.Namespace,
    context: GateContext,
    client: GitHubChecksClient,
    *,
    runner: Callable[[Sequence[str]], CommandResult] = run_command,
) -> int:
    started_at = iso_now()
    check_run_id = client.create_check_run(
        name=args.check_name,
        head_sha=context.head_sha,
        started_at=started_at,
        details_url=context.details_url,
        summary="Trusted metadata evaluation is in progress.",
        external_id=external_id(args.gate),
    )
    try:
        if args.gate != "label":
            raise TrustedGateError(f"Unsupported gate: {args.gate}")
        results = run_label_gate(args, runner)
        title, summary, text = build_output(args.check_name, results)
        conclusion = "success" if all(result.ok for _, result in results) else "failure"
        client.update_check_run(
            check_run_id=check_run_id,
            title=title,
            conclusion=conclusion,
            completed_at=iso_now(),
            summary=summary,
            text=text,
        )
        print(summary)
        return 0 if conclusion == "success" else 1
    except Exception as exc:
        error_text = truncate_text(str(exc))
        client.update_check_run(
            check_run_id=check_run_id,
            title=f"{args.check_name} failed",
            conclusion="failure",
            completed_at=iso_now(),
            summary="Trusted metadata evaluation failed before completion.",
            text=error_text,
        )
        raise


def main() -> int:
    args = parse_args()
    try:
        context = build_context(args)
        owner, repo = split_repo(args.repo)
        client = GitHubChecksClient(owner, repo, args.api_root, args.token)
        return execute_gate(args, context, client)
    except TrustedGateError as exc:
        print(f"trusted-metadata-check[{args.gate}]: {exc}", file=sys.stderr)
        return 1
    except Exception as exc:  # pragma: no cover - defensive CLI surface
        print(f"trusted-metadata-check[{args.gate}]: unexpected failure: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
