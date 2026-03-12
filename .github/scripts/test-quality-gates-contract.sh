#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 "$repo_root/.github/scripts/check_quality_gates_contract.py"
bash "$repo_root/.github/scripts/test-inline-metadata-workflows.sh"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "${tmp_dir}"' EXIT

label_repo="$tmp_dir/label-repo"
cp -R "$repo_root/." "$label_repo"
python3 - <<'PY' "$label_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/label-gate.yml"
text = path.read_text()
needle = """      - name: Publish trusted Validate PR labels check
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          python3 trusted/.github/scripts/run_trusted_metadata_check.py \\
            --gate label \\
            --check-name "Validate PR labels" \\
            --candidate-root "$PWD/candidate" \\
            --trusted-root "$PWD/trusted"
"""
replacement = """      - name: Publish trusted Validate PR labels check
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          echo "python3 trusted/.github/scripts/run_trusted_metadata_check.py --gate label --check-name Validate PR labels --candidate-root $PWD/candidate --trusted-root $PWD/trusted"
"""
if needle not in text:
    raise SystemExit("failed to rewrite label-gate workflow")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$label_repo" >/dev/null 2>"$tmp_dir/label-gate-bait.log"; then
  echo "expected label-gate trusted contract bait fixture to fail" >&2
  exit 1
fi

grep -q "trusted label publisher must invoke trusted metadata publisher" "$tmp_dir/label-gate-bait.log"

review_repo="$tmp_dir/review-repo"
cp -R "$repo_root/." "$review_repo"
python3 - <<'PY' "$review_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/review-policy.yml"
text = path.read_text()
needle = """      - name: Evaluate review policy
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          python3 "${{ steps.trusted-quality-gates.outputs.metadata_script }}" review
"""
replacement = """      - name: Evaluate review policy
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          echo \"python3 ${{ steps.trusted-quality-gates.outputs.metadata_script }} review\"
"""
if needle not in text:
    raise SystemExit("failed to rewrite review-policy workflow")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$review_repo" >/dev/null 2>"$tmp_dir/review-policy-bait.log"; then
  echo "expected review-policy trusted metadata bait fixture to fail" >&2
  exit 1
fi

grep -q "must invoke trusted metadata gate" "$tmp_dir/review-policy-bait.log"

unsafe_yaml_repo="$tmp_dir/unsafe-yaml-repo"
cp -R "$repo_root/." "$unsafe_yaml_repo"
python3 - <<'PY' "$unsafe_yaml_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/review-policy.yml"
text = path.read_text()
path.write_text(text.replace("name: Review Policy", "name: !ruby/object:Kernel {}", 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$unsafe_yaml_repo" >/dev/null 2>"$tmp_dir/unsafe-yaml.log"; then
  echo "expected unsafe YAML fixture to fail" >&2
  exit 1
fi

grep -q "unable to parse YAML via ruby" "$tmp_dir/unsafe-yaml.log"

echo "test-quality-gates-contract: all checks passed"
