#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixtures_root="$repo_root/.github/scripts/fixtures/quality-gates-contract"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "${tmp_dir}"' EXIT

copy_repo_snapshot() {
  local source_root="$1"
  local dest_root="$2"
  mkdir -p "$dest_root"
  tar \
    --exclude='.git' \
    --exclude='target' \
    --exclude='node_modules' \
    --exclude='web/node_modules' \
    --exclude='web/dist' \
    --exclude='coverage' \
    --exclude='playwright-report' \
    -C "$source_root" \
    -cf - \
    . | tar -C "$dest_root" -xf -
}

python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" \
  --repo-root "$repo_root" \
  --declaration "$repo_root/.github/quality-gates.json" \
  --metadata-script "$repo_root/.github/scripts/metadata_gate.py" \
  --profile final

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" \
  --repo-root "$repo_root" \
  --declaration "$repo_root/.github/quality-gates.json" \
  --metadata-script "$repo_root/.github/scripts/metadata_gate.py" \
  --profile bootstrap >/dev/null 2>"$tmp_dir/profile-mismatch.log"; then
  echo "expected final declaration to reject bootstrap profile validation" >&2
  exit 1
fi

grep -q "implementation_profile='final' does not match workflow profile 'bootstrap'" "$tmp_dir/profile-mismatch.log"

baseline_repo="$tmp_dir/baseline-repo"
copy_repo_snapshot "$repo_root" "$baseline_repo"
cp "$fixtures_root/quality-gates.json" "$baseline_repo/.github/quality-gates.json"
for workflow in ci-pr.yml ci-main.yml release.yml label-gate.yml review-policy.yml; do
  cp "$fixtures_root/$workflow" "$baseline_repo/.github/workflows/$workflow"
done

python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$baseline_repo" --profile final
bash "$repo_root/.github/scripts/test-inline-metadata-workflows.sh"

simplified_topology_repo="$tmp_dir/simplified-topology-repo"
copy_repo_snapshot "$baseline_repo" "$simplified_topology_repo"
for workflow in ci-main.yml release.yml label-gate.yml; do
  cp "$fixtures_root/simplified/$workflow" "$simplified_topology_repo/.github/workflows/$workflow"
done

python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$simplified_topology_repo" --profile final

label_concurrency_repo="$tmp_dir/label-concurrency-repo"
copy_repo_snapshot "$baseline_repo" "$label_concurrency_repo"
python3 - <<'PY' "$label_concurrency_repo"
from pathlib import Path
import sys
repo = Path(sys.argv[1])
path = repo / ".github/workflows/label-gate.yml"
text = path.read_text()
needle = "  group: label-gate-${{ github.event.pull_request.number || github.run_id }}\n"
replacement = "  group: label-gate-static\n"
if needle not in text:
    raise SystemExit("failed to rewrite label-gate concurrency group")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$label_concurrency_repo" >/dev/null 2>"$tmp_dir/label-concurrency.log"; then
  echo "expected label-gate concurrency fixture to fail" >&2
  exit 1
fi

grep -q "label-gate.yml.concurrency.group drifted" "$tmp_dir/label-concurrency.log"

coverage_repo="$tmp_dir/coverage-repo"
copy_repo_snapshot "$baseline_repo" "$coverage_repo"
python3 - <<'PY' "$coverage_repo"
from pathlib import Path
import json
import sys
repo = Path(sys.argv[1])
path = repo / ".github/quality-gates.json"
payload = json.loads(path.read_text())
for workflow in payload["expected_pr_workflows"]:
    if workflow.get("workflow") == "CI PR":
        workflow["jobs"] = [item for item in workflow["jobs"] if item != "Build Artifacts"]
path.write_text(json.dumps(payload, indent=2) + "\n")
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$coverage_repo" --profile final >/dev/null 2>"$tmp_dir/coverage.log"; then
  echo "expected PR coverage fixture to fail" >&2
  exit 1
fi

grep -q "expected_pr_workflows jobs must exactly cover required_checks + informational_checks" "$tmp_dir/coverage.log"

release_dispatch_repo="$tmp_dir/release-dispatch-repo"
copy_repo_snapshot "$baseline_repo" "$release_dispatch_repo"
python3 - <<'PY' "$release_dispatch_repo"
from pathlib import Path
import sys
repo = Path(sys.argv[1])
path = repo / ".github/workflows/release.yml"
text = path.read_text()
needle = "      commit_sha:\n"
replacement = "      sha:\n"
if needle not in text:
    raise SystemExit("failed to rewrite release dispatch input")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$release_dispatch_repo" --profile final >/dev/null 2>"$tmp_dir/release-dispatch.log"; then
  echo "expected release dispatch fixture to fail" >&2
  exit 1
fi

grep -q "workflow_dispatch.inputs.commit_sha" "$tmp_dir/release-dispatch.log"

release_workflow_repo="$tmp_dir/release-workflow-repo"
copy_repo_snapshot "$baseline_repo" "$release_workflow_repo"
python3 - <<'PY' "$release_workflow_repo"
from pathlib import Path
import sys
repo = Path(sys.argv[1])
path = repo / ".github/workflows/release.yml"
text = path.read_text()
needle = '        - CI Main\n'
replacement = '        - Main CI\n'
if needle not in text:
    needle = '      - CI Main\n'
    replacement = '      - Main CI\n'
if needle not in text:
    raise SystemExit("failed to rewrite workflow_run workflow name")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$release_workflow_repo" --profile final >/dev/null 2>"$tmp_dir/release-workflow.log"; then
  echo "expected release workflow_run fixture to fail" >&2
  exit 1
fi

grep -q "workflow_run.workflows drifted" "$tmp_dir/release-workflow.log"

ci_main_repo="$tmp_dir/ci-main-repo"
copy_repo_snapshot "$baseline_repo" "$ci_main_repo"
python3 - <<'PY' "$ci_main_repo"
from pathlib import Path
import sys
repo = Path(sys.argv[1])
path = repo / ".github/workflows/ci-main.yml"
text = path.read_text()
needle = "  group: ci-main-main\n"
replacement = "  group: ci-main-${{ github.sha }}\n"
if needle not in text:
    raise SystemExit("failed to rewrite ci-main concurrency group")
text = text.replace(needle, replacement, 1)
needle = "  cancel-in-progress: false\n"
replacement = "  cancel-in-progress: true\n"
if needle not in text:
    raise SystemExit("failed to rewrite ci-main concurrency cancel")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$ci_main_repo" --profile final >/dev/null 2>"$tmp_dir/ci-main.log"; then
  echo "expected ci-main concurrency fixture to fail" >&2
  exit 1
fi

grep -Eq "ci-main.yml.concurrency.(group drifted|cancel-in-progress must stay false)" "$tmp_dir/ci-main.log"

release_concurrency_repo="$tmp_dir/release-concurrency-repo"
copy_repo_snapshot "$baseline_repo" "$release_concurrency_repo"
python3 - <<'PY' "$release_concurrency_repo"
from pathlib import Path
import sys
repo = Path(sys.argv[1])
path = repo / ".github/workflows/release.yml"
text = path.read_text()
needle = "  group: release-main\n"
replacement = "  group: release-${{ github.event.workflow_run.head_sha }}\n"
if needle not in text:
    raise SystemExit("failed to rewrite release concurrency group")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$release_concurrency_repo" --profile final >/dev/null 2>"$tmp_dir/release-concurrency.log"; then
  echo "expected release concurrency fixture to fail" >&2
  exit 1
fi

grep -q "release.yml.concurrency.group drifted" "$tmp_dir/release-concurrency.log"

metadata_policy_repo="$tmp_dir/metadata-policy-repo"
copy_repo_snapshot "$baseline_repo" "$metadata_policy_repo"
python3 - <<'PY' "$metadata_policy_repo"
from pathlib import Path
import sys
repo = Path(sys.argv[1])
path = repo / ".github/scripts/metadata_gate.py"
text = path.read_text()
needle = "REVIEW_REQUIRED_APPROVALS = 1\n"
replacement = "REVIEW_REQUIRED_APPROVALS = 2\n"
if needle not in text:
    raise SystemExit("failed to rewrite metadata policy")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" \
  --repo-root "$metadata_policy_repo" \
  --declaration "$metadata_policy_repo/.github/quality-gates.json" \
  --metadata-script "$metadata_policy_repo/.github/scripts/metadata_gate.py" \
  --profile final >/dev/null 2>"$tmp_dir/metadata-policy.log"; then
  echo "expected metadata policy fixture to fail" >&2
  exit 1
fi

grep -q "REVIEW_REQUIRED_APPROVALS drifted from quality-gates.json" "$tmp_dir/metadata-policy.log"

echo "test-quality-gates-contract: all checks passed"
