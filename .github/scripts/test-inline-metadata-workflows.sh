#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 - <<'PY' "$repo_root/.github/scripts/metadata_gate.py"
from __future__ import annotations

import importlib.util
import os
import sys
import tempfile
from pathlib import Path

script_path = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("metadata_gate", script_path)
module = importlib.util.module_from_spec(spec)
assert spec is not None and spec.loader is not None
sys.modules[spec.name] = module
spec.loader.exec_module(module)


class FakeClient:
    def __init__(self) -> None:
        self.owner = "IvanLi-CN"
        self.repo = "codex-vibe-monitor"
        self.issue_labels = {
            57: ["type:minor", "channel:rc"],
            58: ["type:patch", "type:minor", "channel:stable"],
            59: ["type:patch"],
        }
        self.pulls = {
            57: "bob",
            58: "bob",
            59: "bob",
            60: "bob",
            61: "maintainer",
            62: "IvanLi-CN",
            63: "bob",
        }
        self.permissions = {
            "bob": "write",
            "reviewer": "write",
            "maintainer": "maintain",
        }
        self.reviews = {
            57: [
                {
                    "user": {"login": "reviewer"},
                    "state": "APPROVED",
                    "submitted_at": "2026-03-12T00:00:00Z",
                }
            ],
            58: [],
            59: [
                {
                    "user": {"login": "reviewer"},
                    "state": "APPROVED",
                    "submitted_at": "2026-03-12T00:00:00Z",
                },
                {
                    "user": {"login": "reviewer"},
                    "state": "CHANGES_REQUESTED",
                    "submitted_at": "2026-03-12T00:05:00Z",
                },
            ],
            60: [
                {
                    "user": {"login": "reviewer"},
                    "state": "APPROVED",
                    "submitted_at": "2026-03-12T00:00:00Z",
                },
                {
                    "user": {"login": "reviewer"},
                    "state": "DISMISSED",
                    "submitted_at": "2026-03-12T00:05:00Z",
                },
            ],
            63: [
                {
                    "user": {"login": "reviewer"},
                    "state": "APPROVED",
                    "submitted_at": "2026-03-12T00:00:00Z",
                },
                {
                    "user": {"login": "reviewer"},
                    "state": "COMMENTED",
                    "submitted_at": "2026-03-12T00:05:00Z",
                },
            ],
        }

    def request_json(self, path: str, query=None):
        del query
        if path.startswith("/repos/IvanLi-CN/codex-vibe-monitor/issues/"):
            pull_number = int(path.rsplit("/", 1)[-1])
            return {"labels": [{"name": label} for label in self.issue_labels[pull_number]]}
        if path.startswith("/repos/IvanLi-CN/codex-vibe-monitor/pulls/"):
            pull_number = int(path.rsplit("/", 1)[-1])
            return {
                "user": {"login": self.pulls[pull_number]},
                "head": {"sha": f"sha-{pull_number}"},
            }
        if "/collaborators/" in path and path.endswith("/permission"):
            username = path.split("/collaborators/", 1)[1].rsplit("/", 1)[0]
            return {"permission": self.permissions.get(username, "none")}
        raise AssertionError(f"unexpected request_json path: {path}")

    def paginate(self, path: str, query=None):
        del query
        if path.startswith("/repos/IvanLi-CN/codex-vibe-monitor/pulls/") and path.endswith("/reviews"):
            pull_number = int(path.split("/pulls/", 1)[1].split("/", 1)[0])
            return self.reviews.get(pull_number, [])
        raise AssertionError(f"unexpected paginate path: {path}")


def make_context(gate: str, event_name: str, pull_number: int) -> object:
    return module.GateContext(
        gate=gate,
        owner="IvanLi-CN",
        repo="codex-vibe-monitor",
        api_root="https://api.github.com",
        token="",
        event_name=event_name,
        event_payload={"pull_request": {"number": pull_number}},
        manual_pull_number=None,
    )


def run_with_summary(fn, *args, **kwargs):
    with tempfile.NamedTemporaryFile(delete=False) as handle:
        summary_path = handle.name
    try:
        os.environ["GITHUB_STEP_SUMMARY"] = summary_path
        exit_code = fn(*args, **kwargs)
        summary = Path(summary_path).read_text()
        return exit_code, summary
    finally:
        os.environ.pop("GITHUB_STEP_SUMMARY", None)
        Path(summary_path).unlink(missing_ok=True)


client = FakeClient()

exit_code, summary = run_with_summary(
    module.run_label_gate,
    make_context("label", "pull_request_target", 57),
    client,
)
assert exit_code == 0, f"expected label gate success, got {exit_code}"
assert "PR #57: pass - Labels OK: type:minor + channel:rc" in summary

for pull_number in (58, 59):
    exit_code, summary = run_with_summary(
        module.run_label_gate,
        make_context("label", "pull_request_target", pull_number),
        client,
    )
    assert exit_code == 1, f"expected label gate failure for PR #{pull_number}, got {exit_code}"
    assert f"PR #{pull_number}: fail" in summary

exit_code, summary = run_with_summary(
    module.run_review_gate,
    make_context("review", "pull_request_review", 57),
    client,
)
assert exit_code == 0, f"expected review gate success, got {exit_code}"
assert "Approval satisfied by @reviewer (write)." in summary

for pull_number in (58, 59, 60):
    exit_code, summary = run_with_summary(
        module.run_review_gate,
        make_context("review", "pull_request_review", pull_number),
        client,
    )
    assert exit_code == 1, f"expected review gate failure for PR #{pull_number}, got {exit_code}"
    assert f"PR #{pull_number}: fail" in summary

for pull_number in (61, 62):
    exit_code, summary = run_with_summary(
        module.run_review_gate,
        make_context("review", "pull_request_review", pull_number),
        client,
    )
    assert exit_code == 0, f"expected review gate exemption for PR #{pull_number}, got {exit_code}"
    assert "approval not required" in summary

exit_code, summary = run_with_summary(
    module.run_review_gate,
    make_context("review", "pull_request_review", 63),
    client,
)
assert exit_code == 0, f"expected approval to survive a later comment, got {exit_code}"
assert "Approval satisfied by @reviewer (write)." in summary

print("test-inline-metadata-workflows: all checks passed")
PY
