#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
python3 .github/scripts/check_rust_source_quality.py \
  --repo-root "$repo_root" \
  --policy .github/rust-source-quality-policy.json
