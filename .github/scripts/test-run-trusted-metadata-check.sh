#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

python3 - <<'PY' "$repo_root/.github/scripts/run_trusted_metadata_check.py"
from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from types import SimpleNamespace

script_path = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("run_trusted_metadata_check", script_path)
module = importlib.util.module_from_spec(spec)
assert spec is not None and spec.loader is not None
sys.modules[spec.name] = module
spec.loader.exec_module(module)


class FakeClient:
    def __init__(self) -> None:
        self.created = []
        self.updated = []

    def create_check_run(self, **kwargs):
        self.created.append(kwargs)
        return 123

    def update_check_run(self, **kwargs):
        self.updated.append(kwargs)


def make_args() -> object:
    return SimpleNamespace(
        gate="label",
        check_name="Validate PR labels",
        candidate_root="/tmp/candidate",
        trusted_root="/tmp/trusted",
        repo="IvanLi-CN/codex-vibe-monitor",
        api_root="https://api.github.com",
        token="token",
        event_path="/tmp/event.json",
    )


context = module.GateContext(
    owner="IvanLi-CN",
    repo="codex-vibe-monitor",
    head_sha="abc123",
    details_url="https://github.com/IvanLi-CN/codex-vibe-monitor/actions/runs/42",
)


def success_runner(command):
    target = command[1]
    if target.endswith("check_quality_gates_contract.py"):
        return module.CommandResult("contract", 0, "contract ok", "")
    if target.endswith("metadata_gate.py"):
        return module.CommandResult("metadata", 0, "labels ok", "")
    raise AssertionError(f"unexpected command: {command}")


success_client = FakeClient()
exit_code = module.execute_gate(make_args(), context, success_client, runner=success_runner)
assert exit_code == 0, f"expected success exit code, got {exit_code}"
assert success_client.created[0]["name"] == "Validate PR labels"
assert success_client.created[0]["head_sha"] == "abc123"
assert success_client.updated[0]["conclusion"] == "success"
assert "Contract" in success_client.updated[0]["summary"]
assert "Labels" in success_client.updated[0]["summary"]
assert "contract ok" in success_client.updated[0]["text"]
assert "labels ok" in success_client.updated[0]["text"]


def failing_runner(command):
    target = command[1]
    if target.endswith("check_quality_gates_contract.py"):
        return module.CommandResult("contract", 1, "", "contract drift")
    if target.endswith("metadata_gate.py"):
        return module.CommandResult("metadata", 0, "labels ok", "")
    raise AssertionError(f"unexpected command: {command}")


failure_client = FakeClient()
exit_code = module.execute_gate(make_args(), context, failure_client, runner=failing_runner)
assert exit_code == 1, f"expected failure exit code, got {exit_code}"
assert failure_client.updated[0]["conclusion"] == "failure"
assert "contract drift" in failure_client.updated[0]["text"]

print("test-run-trusted-metadata-check: all checks passed")
PY
