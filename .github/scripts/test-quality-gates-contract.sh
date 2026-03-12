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

label_concurrency_repo="$tmp_dir/label-concurrency-repo"
cp -R "$repo_root/." "$label_concurrency_repo"
python3 - <<'PY' "$label_concurrency_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/label-gate.yml"
text = path.read_text()
needle = "  group: label-gate-${{ github.event_name }}-${{ github.event.pull_request.number || github.run_id }}\n"
replacement = "  group: label-gate-${{ github.event.pull_request.number || github.run_id }}\n"
if needle not in text:
    raise SystemExit("failed to rewrite label-gate concurrency group")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$label_concurrency_repo" >/dev/null 2>"$tmp_dir/label-concurrency.log"; then
  echo "expected label-gate concurrency fixture to fail" >&2
  exit 1
fi

grep -q "concurrency.group must isolate pull_request and pull_request_target runs" "$tmp_dir/label-concurrency.log"

dynamic_contract_repo="$tmp_dir/dynamic-contract-repo"
cp -R "$repo_root/." "$dynamic_contract_repo"
python3 - <<'PY' "$dynamic_contract_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
contract_path = repo / ".github/quality-gates.json"
contract_text = contract_path.read_text()
contract_text = contract_text.replace('"Validate PR labels"', '"Release Labels Gate"')
contract_path.write_text(contract_text)

workflow_path = repo / ".github/workflows/label-gate.yml"
workflow_text = workflow_path.read_text()
workflow_text = workflow_text.replace('--check-name "Validate PR labels"', '--check-name "Release Labels Gate"')
workflow_path.write_text(workflow_text)
PY

python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$dynamic_contract_repo" >/dev/null

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

review_if_repo="$tmp_dir/review-if-repo"
cp -R "$repo_root/." "$review_if_repo"
python3 - <<'PY' "$review_if_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/review-policy.yml"
text = path.read_text()
needle = """      - name: Evaluate review policy
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
"""
replacement = """      - name: Evaluate review policy
        if: ${{ false }}
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
"""
if needle not in text:
    raise SystemExit("failed to inject review-policy if guard")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$review_if_repo" >/dev/null 2>"$tmp_dir/review-policy-if.log"; then
  echo "expected review-policy if:false fixture to fail" >&2
  exit 1
fi

grep -q "Evaluate review policy'].if must stay unset" "$tmp_dir/review-policy-if.log"

ci_if_repo="$tmp_dir/ci-if-repo"
cp -R "$repo_root/." "$ci_if_repo"
python3 - <<'PY' "$ci_if_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/ci.yml"
text = path.read_text()
needle = """      - name: Quality-gates live rules check
        env:
          QUALITY_GATES_LIVE_RULES_MODE: require
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
"""
replacement = """      - name: Quality-gates live rules check
        if: ${{ false }}
        env:
          QUALITY_GATES_LIVE_RULES_MODE: require
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
"""
if needle not in text:
    raise SystemExit("failed to inject ci if guard")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$ci_if_repo" >/dev/null 2>"$tmp_dir/ci-if.log"; then
  echo "expected ci if:false fixture to fail" >&2
  exit 1
fi

grep -q "Quality-gates live rules check'].if must stay unset" "$tmp_dir/ci-if.log"

ci_live_contract_repo="$tmp_dir/ci-live-contract-repo"
cp -R "$repo_root/." "$ci_live_contract_repo"
python3 - <<'PY' "$ci_live_contract_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/ci.yml"
text = path.read_text()
needle = '--declaration ".github/quality-gates.json"'
replacement = '--declaration "${{ steps.trusted-quality-gates.outputs.declaration }}"'
if needle not in text:
    raise SystemExit("failed to rewrite ci live declaration source")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$ci_live_contract_repo" >/dev/null 2>"$tmp_dir/ci-live-contract.log"; then
  echo "expected ci live declaration source fixture to fail" >&2
  exit 1
fi

grep -q "live rules step must use the trusted checker against the candidate declaration" "$tmp_dir/ci-live-contract.log"

ci_merge_group_repo="$tmp_dir/ci-merge-group-repo"
cp -R "$repo_root/." "$ci_merge_group_repo"
python3 - <<'PY' "$ci_merge_group_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/ci.yml"
text = path.read_text()
needle = """          elif [ \"${{ github.event_name }}\" = \"merge_group\" ]; then
            queue_prefix=\"refs/heads/gh-readonly-queue/\"
            if [[ \"${GITHUB_REF}\" != \"${queue_prefix}\"* ]]; then
              echo \"::error::merge_group ref ${GITHUB_REF} must use the gh-readonly-queue/<base_branch>/... format.\"
              exit 1
            fi
            queue_ref=\"${GITHUB_REF#${queue_prefix}}\"
            if [[ \"${queue_ref}\" != */pr-* ]]; then
              echo \"::error::Unable to derive the merge_group base branch from ${GITHUB_REF}.\"
              exit 1
            fi
            base_branch=\"${queue_ref%%/pr-*}\"
            if [ -z \"${base_branch}\" ]; then
              echo \"::error::Unable to derive the merge_group base branch from ${GITHUB_REF}.\"
              exit 1
            fi
"""
if needle not in text:
    raise SystemExit("failed to rewrite ci merge_group branch resolution")
path.write_text(text.replace(needle, "", 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$ci_merge_group_repo" >/dev/null 2>"$tmp_dir/ci-merge-group.log"; then
  echo "expected ci merge_group trusted-source fixture to fail" >&2
  exit 1
fi

grep -q "merge_group trusted-source branch handling drifted" "$tmp_dir/ci-merge-group.log"

metadata_repo="$tmp_dir/metadata-repo"
cp -R "$repo_root/." "$metadata_repo"
python3 - <<'PY' "$metadata_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/scripts/metadata_gate.py"
text = path.read_text()
needle = "REVIEW_REQUIRED_APPROVALS = 1"
replacement = "REVIEW_REQUIRED_APPROVALS = 0"
if needle not in text:
    raise SystemExit("failed to rewrite metadata approval constant")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" \
  --repo-root "$metadata_repo" \
  --metadata-script "$metadata_repo/.github/scripts/metadata_gate.py" >/dev/null 2>"$tmp_dir/metadata-policy.log"; then
  echo "expected metadata policy drift fixture to fail" >&2
  exit 1
fi

grep -q "REVIEW_REQUIRED_APPROVALS drifted from quality-gates.json" "$tmp_dir/metadata-policy.log"

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
