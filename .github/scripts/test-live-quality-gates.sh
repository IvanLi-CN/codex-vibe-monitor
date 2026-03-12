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

python3 "$script" \
  --mode require \
  --repo IvanLi-CN/codex-vibe-monitor \
  --declaration "$declaration" \
  --rules-file "$fixtures_dir/rules-main-unexpected-merge-queue.json" \
  --branch main >/dev/null

echo "test-live-quality-gates: all checks passed"
