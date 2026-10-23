#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

run_timed() {
  local label="$1"
  shift
  local started_at
  local finished_at
  local status

  started_at="$(date +%s)"
  if "$@"; then
    status=0
  else
    status=$?
  fi
  finished_at="$(date +%s)"
  echo "rust_source_quality_seconds_${label}=$((finished_at - started_at))"
  return "$status"
}

run_timed fmt cargo fmt --all -- --check
# Clippy checks the same all-target/all-feature graph while performing Cargo's type-check.
run_timed clippy cargo clippy --locked --all-targets --all-features -- -D warnings
run_timed policy python3 .github/scripts/check_rust_source_quality.py \
  --repo-root "$repo_root" \
  --policy .github/rust-source-quality-policy.json
