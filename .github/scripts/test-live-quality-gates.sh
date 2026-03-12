#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script="$repo_root/.github/scripts/check_live_quality_gates.py"
declaration="$repo_root/.github/quality-gates.json"
fixtures_dir="$repo_root/.github/scripts/fixtures/quality-gates"

python3 "$script" \
  --mode require \
  --repo IvanLi-CN/codex-vibe-monitor \
  --declaration "$declaration" \
  --rules-file "$fixtures_dir/rules-main-ok.json" \
  --branch main >/dev/null

if python3 "$script" \
  --mode require \
  --repo IvanLi-CN/codex-vibe-monitor \
  --declaration "$declaration" \
  --rules-file "$fixtures_dir/rules-main-unexpected-merge-queue.json" \
  --branch main >/dev/null 2>"$fixtures_dir/.unexpected-merge-queue.log"; then
  echo "expected unexpected merge_queue fixture to fail" >&2
  exit 1
fi

grep -q "unexpected merge_queue rule" "$fixtures_dir/.unexpected-merge-queue.log"
rm -f "$fixtures_dir/.unexpected-merge-queue.log"

if python3 "$script" \
  --mode require \
  --repo IvanLi-CN/codex-vibe-monitor \
  --declaration "$declaration" \
  --rules-file "$fixtures_dir/rules-main-weak-branch-protection.json" \
  --branch main >/dev/null 2>"$fixtures_dir/.weak-branch-protection.log"; then
  echo "expected weak branch protection fixture to fail" >&2
  exit 1
fi

grep -q "missing deletion rule" "$fixtures_dir/.weak-branch-protection.log"
grep -q "missing non_fast_forward rule" "$fixtures_dir/.weak-branch-protection.log"
rm -f "$fixtures_dir/.weak-branch-protection.log"

if python3 "$script" \
  --mode require \
  --repo IvanLi-CN/codex-vibe-monitor \
  --declaration "$declaration" \
  --rules-file "$fixtures_dir/rules-main-status-check-policy-drift.json" \
  --branch main >/dev/null 2>"$fixtures_dir/.status-check-policy-drift.log"; then
  echo "expected status-check policy drift fixture to fail" >&2
  exit 1
fi

grep -q "strict_required_status_checks_policy" "$fixtures_dir/.status-check-policy-drift.log"
grep -q "required_status_check integrations drift" "$fixtures_dir/.status-check-policy-drift.log"
rm -f "$fixtures_dir/.status-check-policy-drift.log"

if python3 "$script" \
  --mode require \
  --repo IvanLi-CN/codex-vibe-monitor \
  --declaration "$declaration" \
  --rules-file "$fixtures_dir/rules-main-review-policy-legacy-source.json" \
  --branch main >/dev/null 2>"$fixtures_dir/.legacy-source.log"; then
  echo "expected legacy review-policy source fixture to fail" >&2
  exit 1
fi

grep -q "Review Policy Gate: expected one of" "$fixtures_dir/.legacy-source.log"
rm -f "$fixtures_dir/.legacy-source.log"

bypass_rules="$fixtures_dir/.rules-main-bypass-actors.json"
python3 - <<'PY' "$fixtures_dir/rules-main-ok.json" "$bypass_rules"
import json
import sys
from pathlib import Path

source = Path(sys.argv[1])
target = Path(sys.argv[2])
payload = json.loads(source.read_text())
payload[0]["bypass_actors"] = [
    {
        "actor_id": 1,
        "actor_type": "RepositoryRole",
        "bypass_mode": "always"
    }
]
target.write_text(json.dumps(payload, indent=2) + "\n")
PY

if python3 "$script" \
  --mode require \
  --repo IvanLi-CN/codex-vibe-monitor \
  --declaration "$declaration" \
  --rules-file "$bypass_rules" \
  --branch main >/dev/null 2>"$fixtures_dir/.bypass-waiver.log"; then
  echo "expected bypass actors fixture to fail" >&2
  exit 1
fi

grep -q "bypass actors must stay empty" "$fixtures_dir/.bypass-waiver.log"
rm -f "$fixtures_dir/.bypass-waiver.log" "$bypass_rules"

legacy_rules="$fixtures_dir/.rules-main-legacy-flat.json"
python3 - <<'PY' "$fixtures_dir/rules-main-ok.json" "$legacy_rules"
import json
import sys
from pathlib import Path

source = Path(sys.argv[1])
target = Path(sys.argv[2])
payload = json.loads(source.read_text())
target.write_text(json.dumps(payload[0]["rules"], indent=2) + "\n")
PY

if python3 "$script" \
  --mode require \
  --repo IvanLi-CN/codex-vibe-monitor \
  --declaration "$declaration" \
  --rules-file "$legacy_rules" \
  --branch main >/dev/null 2>"$fixtures_dir/.legacy-bypass.log"; then
  echo "expected legacy flat rules fixture to require explicit waiver" >&2
  exit 1
fi

grep -q "bypass actor verification unavailable without explicit waiver" "$fixtures_dir/.legacy-bypass.log"
rm -f "$fixtures_dir/.legacy-bypass.log" "$legacy_rules"

linear_history_rules="$fixtures_dir/.rules-main-linear-history.json"
python3 - <<'PY' "$fixtures_dir/rules-main-ok.json" "$linear_history_rules"
import json
import sys
from pathlib import Path

source = Path(sys.argv[1])
target = Path(sys.argv[2])
payload = json.loads(source.read_text())
payload[0]["rules"].append({"type": "required_linear_history", "parameters": {}})
target.write_text(json.dumps(payload, indent=2) + "\n")
PY

if python3 "$script" \
  --mode require \
  --repo IvanLi-CN/codex-vibe-monitor \
  --declaration "$declaration" \
  --rules-file "$linear_history_rules" \
  --branch main >/dev/null 2>"$fixtures_dir/.linear-history.log"; then
  echo "expected required linear history drift to fail" >&2
  exit 1
fi

grep -q "merge commits must remain allowed" "$fixtures_dir/.linear-history.log"
rm -f "$fixtures_dir/.linear-history.log" "$linear_history_rules"

required_reviewers_rules="$fixtures_dir/.rules-main-required-reviewers.json"
python3 - <<'PY' "$fixtures_dir/rules-main-ok.json" "$required_reviewers_rules"
import json
import sys
from pathlib import Path

source = Path(sys.argv[1])
target = Path(sys.argv[2])
payload = json.loads(source.read_text())
for rule in payload[0]["rules"]:
    if rule.get("type") != "pull_request":
        continue
    params = rule.setdefault("parameters", {})
    params["required_reviewers"] = [
        {
            "reviewer_id": 123456,
            "reviewer_type": "Team",
        }
    ]
    break
target.write_text(json.dumps(payload, indent=2) + "\n")
PY

if python3 "$script" \
  --mode require \
  --repo IvanLi-CN/codex-vibe-monitor \
  --declaration "$declaration" \
  --rules-file "$required_reviewers_rules" \
  --branch main >/dev/null 2>"$fixtures_dir/.required-reviewers.log"; then
  echo "expected required reviewers drift to fail" >&2
  exit 1
fi

grep -q "required_reviewers must stay empty" "$fixtures_dir/.required-reviewers.log"
rm -f "$fixtures_dir/.required-reviewers.log" "$required_reviewers_rules"

python3 - <<'PY' "$script"
import importlib.util
import json
import sys
import urllib.parse
import urllib.request
from pathlib import Path

script_path = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("check_live_quality_gates", script_path)
module = importlib.util.module_from_spec(spec)
assert spec is not None and spec.loader is not None
spec.loader.exec_module(module)

calls = []

class FakeResponse:
    def __init__(self, payload):
        self.payload = payload

    def read(self, *_args, **_kwargs):
        return json.dumps(self.payload).encode("utf-8")

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        return False


def fake_urlopen(request, timeout=30):
    parsed = urllib.parse.urlsplit(request.full_url)
    params = urllib.parse.parse_qs(parsed.query)
    page = int(params["page"][0])
    calls.append(page)
    if page == 1:
        payload = [{"type": f"rule-{index}"} for index in range(100)]
    elif page == 2:
        payload = [{"type": "rule-100"}]
    else:
        raise AssertionError(f"unexpected page {page}")
    return FakeResponse(payload)


original_urlopen = urllib.request.urlopen
urllib.request.urlopen = fake_urlopen
try:
    payload = module.fetch_branch_rules("https://api.github.com", "IvanLi-CN", "codex-vibe-monitor", "main")
finally:
    urllib.request.urlopen = original_urlopen

assert calls == [1, 2], f"expected pagination through page 2, got {calls}"
assert isinstance(payload, list) and len(payload) == 101, f"expected 101 accumulated rules, got {len(payload)}"
PY

echo "test-live-quality-gates: all checks passed"
