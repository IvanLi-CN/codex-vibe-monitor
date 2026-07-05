#!/usr/bin/env bash
set -euo pipefail

start_epoch="$(date +%s)"

if ! command -v cargo-nextest >/dev/null 2>&1; then
  echo "::error::cargo-nextest is not installed. Install it before running backend tests."
  exit 1
fi

cargo nextest run --locked --all-features --no-fail-fast

end_epoch="$(date +%s)"
echo "backend_test_total_seconds=$((end_epoch - start_epoch))"
