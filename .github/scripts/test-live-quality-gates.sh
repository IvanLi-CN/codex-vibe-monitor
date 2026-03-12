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
