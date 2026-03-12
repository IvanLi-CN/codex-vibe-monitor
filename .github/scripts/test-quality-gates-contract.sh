#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 "$repo_root/.github/scripts/check_quality_gates_contract.py"
bash "$repo_root/.github/scripts/test-inline-metadata-workflows.sh"

echo "test-quality-gates-contract: all checks passed"
