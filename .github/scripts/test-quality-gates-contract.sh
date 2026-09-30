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
for workflow in ci-pr.yml ci-main.yml release.yml release-snapshot-pr.yml label-gate.yml review-policy.yml; do
  cp "$fixtures_root/$workflow" "$baseline_repo/.github/workflows/$workflow"
done

python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$baseline_repo" --profile final
bash "$repo_root/.github/scripts/test-inline-metadata-workflows.sh"

continue_error_repo="$tmp_dir/continue-error-repo"
copy_repo_snapshot "$repo_root" "$continue_error_repo"

expect_continue_on_error_rejected() {
  local name="$1"
  local workflow="$2"
  local needle="$3"
  local replacement="$4"
  local path="$continue_error_repo/$workflow"

  python3 - "$path" "$needle" "$replacement" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
needle, replacement = sys.argv[2:4]
if needle not in text:
    raise SystemExit("failed to locate continue-on-error fixture insertion point")
path.write_text(text.replace(needle, replacement, 1))
PY

  if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" \
    --repo-root "$continue_error_repo" --profile final >/dev/null 2>"$tmp_dir/$name.log"; then
    echo "expected $name continue-on-error fixture to fail" >&2
    exit 1
  fi
  grep -q "continue-on-error must not ignore failures" "$tmp_dir/$name.log"

  python3 - "$path" "$replacement" "$needle" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
replacement, needle = sys.argv[2:4]
if replacement not in text:
    raise SystemExit("failed to restore continue-on-error fixture")
path.write_text(text.replace(replacement, needle, 1))
PY
}

expect_continue_on_error_rejected \
  pr-shard-job \
  .github/workflows/ci-pr.yml \
  $'  backend-tests-stateful-sqlite-shard-1:\n' \
  $'  backend-tests-stateful-sqlite-shard-1:\n    continue-on-error: true\n'
expect_continue_on_error_rejected \
  pr-shard-step \
  .github/workflows/ci-pr.yml \
  $'      - name: Run stateful SQLite backend profile\n' \
  $'      - name: Run stateful SQLite backend profile\n        continue-on-error: true\n'
expect_continue_on_error_rejected \
  pr-stateful-aggregate-step \
  .github/workflows/ci-pr.yml \
  $'      - name: Require both Stateful SQLite shards\n' \
  $'      - name: Require both Stateful SQLite shards\n        continue-on-error: true\n'
expect_continue_on_error_rejected \
  pr-representative-job \
  .github/workflows/ci-pr.yml \
  $'  backend-tests-representative-scale:\n' \
  $'  backend-tests-representative-scale:\n    continue-on-error: true\n'
expect_continue_on_error_rejected \
  pr-representative-step \
  .github/workflows/ci-pr.yml \
  $'      - name: Run deterministic representative-scale acceptance\n' \
  $'      - name: Run deterministic representative-scale acceptance\n        continue-on-error: true\n'
expect_continue_on_error_rejected \
  main-shard-step \
  .github/workflows/ci-main.yml \
  $'      - name: Run stateful SQLite backend profile\n' \
  $'      - name: Run stateful SQLite backend profile\n        continue-on-error: true\n'
expect_continue_on_error_rejected \
  main-stateful-aggregate-step \
  .github/workflows/ci-main.yml \
  $'      - name: Require both Stateful SQLite shards\n' \
  $'      - name: Require both Stateful SQLite shards\n        continue-on-error: true\n'
expect_continue_on_error_rejected \
  pr-lightweight-consumer-job \
  .github/workflows/ci-pr.yml \
  $'  backend-tests-lightweight:\n' \
  $'  backend-tests-lightweight:\n    continue-on-error: true\n'
expect_continue_on_error_rejected \
  pr-archive-file-io-consumer-step \
  .github/workflows/ci-pr.yml \
  $'      - name: Download backend test archive\n' \
  $'      - name: Download backend test archive\n        continue-on-error: true\n'
expect_continue_on_error_rejected \
  main-lightweight-consumer-job \
  .github/workflows/ci-main.yml \
  $'  backend-tests-lightweight:\n' \
  $'  backend-tests-lightweight:\n    continue-on-error: true\n'
expect_continue_on_error_rejected \
  main-archive-file-io-consumer-step \
  .github/workflows/ci-main.yml \
  $'      - name: Run archive / file I/O backend profile\n' \
  $'      - name: Run archive / file I/O backend profile\n        continue-on-error: true\n'

ci_pr_workflow="$baseline_repo/.github/workflows/ci-pr.yml"
python3 - <<'PY' "$ci_pr_workflow"
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
needle = "--partition hash:1/2"
replacement = "--partition hash:2/2"
if needle not in text:
    raise SystemExit("failed to locate PR Stateful SQLite partition")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$baseline_repo" --profile final >/dev/null 2>"$tmp_dir/ci-pr-shard.log"; then
  echo "expected incorrect PR Stateful SQLite partition fixture to fail" >&2
  exit 1
fi

grep -q "must run partition hash:1/2" "$tmp_dir/ci-pr-shard.log"
cp "$fixtures_root/ci-pr.yml" "$ci_pr_workflow"

python3 - <<'PY' "$ci_pr_workflow"
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
needle = "--partition hash:1/2"
replacement = "--partition hash:1/20"
if needle not in text:
    raise SystemExit("failed to locate PR Stateful SQLite partition for prefix regression")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$baseline_repo" --profile final >/dev/null 2>"$tmp_dir/ci-pr-shard-prefix.log"; then
  echo "expected PR Stateful SQLite partition prefix fixture to fail" >&2
  exit 1
fi

grep -q "must run partition hash:1/2" "$tmp_dir/ci-pr-shard-prefix.log"
cp "$fixtures_root/ci-pr.yml" "$ci_pr_workflow"

python3 - <<'PY' "$ci_pr_workflow"
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
needle = '          test "${SHARD_TWO_RESULT}" = success\n'
replacement = "          : <<'EOF'\n          test \"${SHARD_TWO_RESULT}\" = success\n          EOF\n"
if needle not in text:
    raise SystemExit("failed to locate CI PR Stateful SQLite shard success assertion")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$baseline_repo" --profile final >/dev/null 2>"$tmp_dir/ci-pr-shard-aggregate.log"; then
  echo "expected CI PR aggregate with a non-executable shard 2 success assertion to fail" >&2
  exit 1
fi

grep -q "ci-pr.yml Stateful SQLite aggregate must preserve the exact fail-closed script" "$tmp_dir/ci-pr-shard-aggregate.log"
cp "$fixtures_root/ci-pr.yml" "$ci_pr_workflow"

ci_main_workflow="$baseline_repo/.github/workflows/ci-main.yml"
python3 - <<'PY' "$ci_main_workflow"
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
needle = '          SHARD_TWO_RESULT: ${{ needs.backend-tests-stateful-sqlite-shard-2.result }}\n'
replacement = '          SHARD_TWO_RESULT: ${{ needs.backend-tests-stateful-sqlite-shard-1.result }}\n'
if needle not in text:
    raise SystemExit("failed to locate CI Main Stateful SQLite shard result mapping")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$baseline_repo" --profile final >/dev/null 2>"$tmp_dir/ci-main-shard-mapping.log"; then
  echo "expected CI Main aggregate with a duplicated shard result mapping to fail" >&2
  exit 1
fi

grep -q "ci-main.yml Stateful SQLite aggregate must read both matching shard results" "$tmp_dir/ci-main-shard-mapping.log"
cp "$fixtures_root/ci-main.yml" "$ci_main_workflow"

python3 - <<'PY' "$ci_main_workflow"
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
needle = "--partition hash:1/2"
replacement = "--partition hash:1/20"
if needle not in text:
    raise SystemExit("failed to locate CI Main Stateful SQLite partition for prefix regression")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$baseline_repo" --profile final >/dev/null 2>"$tmp_dir/ci-main-shard-prefix.log"; then
  echo "expected CI Main Stateful SQLite partition prefix fixture to fail" >&2
  exit 1
fi

grep -q "ci-main.yml.jobs.backend-tests-stateful-sqlite-shard-1 must run partition hash:1/2" "$tmp_dir/ci-main-shard-prefix.log"
cp "$fixtures_root/ci-main.yml" "$ci_main_workflow"

python3 - <<'PY' "$ci_main_workflow"
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
needle = '          test "${SHARD_TWO_RESULT}" = success\n'
replacement = "          : <<'EOF'\n          test \"${SHARD_TWO_RESULT}\" = success\n          EOF\n"
if needle not in text:
    raise SystemExit("failed to locate CI Main Stateful SQLite shard success assertion")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$baseline_repo" --profile final >/dev/null 2>"$tmp_dir/ci-main-shard-aggregate.log"; then
  echo "expected CI Main aggregate with a non-executable shard 2 success assertion to fail" >&2
  exit 1
fi

grep -q "ci-main.yml Stateful SQLite aggregate must preserve the exact fail-closed script" "$tmp_dir/ci-main-shard-aggregate.log"
cp "$fixtures_root/ci-main.yml" "$ci_main_workflow"

python3 - <<'PY' "$ci_pr_workflow"
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
needle = "run: bunx playwright install chromium"
replacement = "run: bunx playwright install --with-deps chromium"
if needle not in text:
    raise SystemExit("failed to locate Storybook Playwright install")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$baseline_repo" --profile final >/dev/null 2>"$tmp_dir/storybook-install.log"; then
  echo "expected Storybook system dependency install fixture to fail" >&2
  exit 1
fi

grep -q "must install Chromium without system dependencies" "$tmp_dir/storybook-install.log"
cp "$fixtures_root/ci-pr.yml" "$ci_pr_workflow"

upgraded_baseline_repo="$tmp_dir/upgraded-baseline-repo"
copy_repo_snapshot "$baseline_repo" "$upgraded_baseline_repo"
python3 - <<'PY' "$upgraded_baseline_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
for path in (repo / ".github/workflows").glob("*.yml"):
    text = path.read_text()
    path.write_text(text.replace("actions/checkout@v4", "actions/checkout@v7"))

release_path = repo / ".github/workflows/release.yml"
release_text = release_path.read_text()
needle = "  ci-main-gate:\n    name: CI Main Gate\n    runs-on: ubuntu-latest\n"
replacement = "  ci-main-gate:\n    name: CI Main Gate\n    runs-on: ubuntu-24.04\n"
if needle in release_text:
    release_path.write_text(release_text.replace(needle, replacement, 1))
elif replacement not in release_text:
    raise SystemExit("failed to align release ci-main-gate runner")
PY

python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$upgraded_baseline_repo" --profile final

checkout_action_repo="$tmp_dir/checkout-action-repo"
copy_repo_snapshot "$upgraded_baseline_repo" "$checkout_action_repo"
python3 - <<'PY' "$checkout_action_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/label-gate.yml"
text = path.read_text()
needle = "uses: actions/checkout@v7"
replacement = "uses: actions/checkout@v6"
if needle not in text:
    raise SystemExit("failed to rewrite label-gate checkout action")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$checkout_action_repo" --profile final >/dev/null 2>"$tmp_dir/checkout-action.log"; then
  echo "expected unsupported checkout action fixture to fail" >&2
  exit 1
fi

grep -q "must stay 'actions/checkout@v7'" "$tmp_dir/checkout-action.log"

ci_main_gate_runner_repo="$tmp_dir/ci-main-gate-runner-repo"
copy_repo_snapshot "$upgraded_baseline_repo" "$ci_main_gate_runner_repo"
python3 - <<'PY' "$ci_main_gate_runner_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/release.yml"
text = path.read_text()
needle = "  ci-main-gate:\n    name: CI Main Gate\n    runs-on: ubuntu-24.04\n"
replacement = "  ci-main-gate:\n    name: CI Main Gate\n    runs-on: ubuntu-22.04\n"
if needle not in text:
    raise SystemExit("failed to rewrite release ci-main-gate runner")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$ci_main_gate_runner_repo" --profile final >/dev/null 2>"$tmp_dir/ci-main-gate-runner.log"; then
  echo "expected unsupported ci-main-gate runner fixture to fail" >&2
  exit 1
fi

grep -q "release.yml.jobs.ci-main-gate.runs-on drifted" "$tmp_dir/ci-main-gate-runner.log"

simplified_topology_repo="$tmp_dir/simplified-topology-repo"
copy_repo_snapshot "$baseline_repo" "$simplified_topology_repo"
for workflow in ci-main.yml release.yml release-snapshot-pr.yml label-gate.yml; do
  cp "$fixtures_root/simplified/$workflow" "$simplified_topology_repo/.github/workflows/$workflow"
done

python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$simplified_topology_repo" --profile final

release_comment_permissions_repo="$tmp_dir/release-comment-permissions-repo"
copy_repo_snapshot "$baseline_repo" "$release_comment_permissions_repo"
python3 - <<'PY' "$release_comment_permissions_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/release.yml"
text = path.read_text()
needle = "      packages: write\n"
replacement = "      issues: write\n      packages: write\n      pull-requests: write\n"
if needle not in text:
    raise SystemExit("failed to locate release-publish package permission")
prefix, suffix = text.rsplit(needle, 1)
path.write_text(prefix + replacement + suffix)
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$release_comment_permissions_repo" --profile final >/dev/null 2>"$tmp_dir/release-comment-permissions.log"; then
  echo "expected source-PR comment permissions fixture to fail" >&2
  exit 1
fi

grep -q "permissions must exclude source-PR comment permissions" "$tmp_dir/release-comment-permissions.log"

release_comment_step_repo="$tmp_dir/release-comment-step-repo"
copy_repo_snapshot "$baseline_repo" "$release_comment_step_repo"
python3 - <<'PY' "$release_comment_step_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/release.yml"
text = path.read_text()
needle = "      - name: Resolve next pending release target\n"
replacement = "      - name: Upsert PR release version comment\n        run: echo 'legacy source-PR comment'\n\n" + needle
if needle not in text:
    raise SystemExit("failed to locate release queue step")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$release_comment_step_repo" --profile final >/dev/null 2>"$tmp_dir/release-comment-step.log"; then
  echo "expected source-PR comment step fixture to fail" >&2
  exit 1
fi

grep -q "source-PR release comment step must stay removed" "$tmp_dir/release-comment-step.log"

release_notes_repo="$tmp_dir/release-notes-repo"
copy_repo_snapshot "$baseline_repo" "$release_notes_repo"
python3 - <<'PY' "$release_notes_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/release.yml"
text = path.read_text()
needle = "              generate_release_notes: true,\n"
if needle not in text:
    raise SystemExit("failed to locate generated release notes option")
path.write_text(text.replace(needle, "", 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$release_notes_repo" --profile final >/dev/null 2>"$tmp_dir/release-notes.log"; then
  echo "expected missing generated release notes option fixture to fail" >&2
  exit 1
fi

grep -q "createRelease must set generate_release_notes: true" "$tmp_dir/release-notes.log"

release_body_repo="$tmp_dir/release-body-repo"
copy_repo_snapshot "$baseline_repo" "$release_body_repo"
python3 - <<'PY' "$release_body_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/release.yml"
text = path.read_text()
needle = "              generate_release_notes: true,\n"
replacement = needle + "              body: 'legacy metadata',\n"
if needle not in text:
    raise SystemExit("failed to locate generated release notes option")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$release_body_repo" --profile final >/dev/null 2>"$tmp_dir/release-body.log"; then
  echo "expected custom release body fixture to fail" >&2
  exit 1
fi

grep -q "createRelease must not set body" "$tmp_dir/release-body.log"

label_concurrency_repo="$tmp_dir/label-concurrency-repo"
copy_repo_snapshot "$baseline_repo" "$label_concurrency_repo"
python3 - <<'PY' "$label_concurrency_repo"
from pathlib import Path
import sys
repo = Path(sys.argv[1])
path = repo / ".github/workflows/label-gate.yml"
text = path.read_text()
needle = "github.event_name == 'pull_request' && github.event.action == 'edited'"
replacement = "github.event.action == 'edited'"
if needle not in text:
    raise SystemExit("failed to rewrite label-gate metadata selector")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$label_concurrency_repo" >/dev/null 2>"$tmp_dir/label-concurrency.log"; then
  echo "expected label-gate concurrency fixture to fail" >&2
  exit 1
fi

grep -q "label-gate.yml.concurrency.group drifted" "$tmp_dir/label-concurrency.log"

label_cancellation_repo="$tmp_dir/label-cancellation-repo"
copy_repo_snapshot "$baseline_repo" "$label_cancellation_repo"
python3 - <<'PY' "$label_cancellation_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/label-gate.yml"
text = path.read_text()
needle = "  cancel-in-progress: false\n"
replacement = "  cancel-in-progress: true\n"
if needle not in text:
    raise SystemExit("failed to locate label-gate cancellation policy")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$label_cancellation_repo" >/dev/null 2>"$tmp_dir/label-cancellation.log"; then
  echo "expected label-gate cancellation fixture to fail" >&2
  exit 1
fi

grep -q "label-gate.yml.concurrency.cancel-in-progress must stay false" "$tmp_dir/label-cancellation.log"

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

grep -q "expected_pr_workflows jobs must exactly cover required_checks" "$tmp_dir/coverage.log"

archive_cache_repo="$tmp_dir/archive-cache-repo"
copy_repo_snapshot "$baseline_repo" "$archive_cache_repo"
python3 - <<'PY' "$archive_cache_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/ci-pr.yml"
text = path.read_text()
needle = "if: ${{ steps.build-backend-test-archive.outcome == 'success' && steps.cargo-test-cache.outputs.cache-hit != 'true' }}"
replacement = "if: ${{ always() && steps.cargo-test-cache.outputs.cache-hit != 'true' }}"
if needle not in text:
    raise SystemExit("failed to rewrite archive target-cache condition")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$archive_cache_repo" --profile final >/dev/null 2>"$tmp_dir/archive-cache.log"; then
  echo "expected archive target-cache condition fixture to fail" >&2
  exit 1
fi

grep -q "target cache must save only after a successful archive build" "$tmp_dir/archive-cache.log"

archive_consumer_repo="$tmp_dir/archive-consumer-repo"
copy_repo_snapshot "$baseline_repo" "$archive_consumer_repo"
python3 - <<'PY' "$archive_consumer_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/ci-pr.yml"
text = path.read_text()
needle = '''  backend-tests-lightweight:
    needs: backend-test-archive
    if: always()
    name: Backend Tests (Lightweight)
    runs-on: ubuntu-24.04
    timeout-minutes: 20
    steps:
      - name: Verify backend test archive producer
        env:
          PRODUCER_RESULT: ${{ needs.backend-test-archive.result }}
        run: test "$PRODUCER_RESULT" = success

'''
replacement = needle.replace(
    '''      - name: Verify backend test archive producer
        env:
          PRODUCER_RESULT: ${{ needs.backend-test-archive.result }}
        run: test "$PRODUCER_RESULT" = success

''',
    "",
    1,
)
if needle not in text:
    raise SystemExit("failed to locate PR lightweight producer-result guard")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$archive_consumer_repo" --profile final >/dev/null 2>"$tmp_dir/archive-consumer.log"; then
  echo "expected missing backend archive producer-result guard fixture to fail" >&2
  exit 1
fi

grep -q "missing step 'Verify backend test archive producer'" "$tmp_dir/archive-consumer.log"

main_archive_consumer_repo="$tmp_dir/main-archive-consumer-repo"
copy_repo_snapshot "$baseline_repo" "$main_archive_consumer_repo"
python3 - <<'PY' "$main_archive_consumer_repo"
from pathlib import Path
import sys

path = Path(sys.argv[1]) / ".github/workflows/ci-main.yml"
text = path.read_text()
needle = '''      - name: Verify backend test archive producer
        env:
          PRODUCER_RESULT: ${{ needs.backend-test-archive.result }}
        run: test "$PRODUCER_RESULT" = success

'''
if needle not in text:
    raise SystemExit("failed to locate Main archive consumer producer-result guard")
path.write_text(text.replace(needle, "", 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$main_archive_consumer_repo" --profile final >/dev/null 2>"$tmp_dir/main-archive-consumer.log"; then
  echo "expected missing Main backend archive producer-result guard fixture to fail" >&2
  exit 1
fi

grep -q "missing step 'Verify backend test archive producer'" "$tmp_dir/main-archive-consumer.log"

archive_profile_repo="$tmp_dir/archive-profile-repo"
copy_repo_snapshot "$baseline_repo" "$archive_profile_repo"
python3 - <<'PY' "$archive_profile_repo"
from pathlib import Path
import sys

path = Path(sys.argv[1]) / ".github/workflows/ci-pr.yml"
text = path.read_text()
needle = "run: bash .github/scripts/run-backend-tests.sh --profile lightweight --archive-file \"$RUNNER_TEMP/backend-tests.tar.zst\""
replacement = "run: bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite --archive-file \"$RUNNER_TEMP/backend-tests.tar.zst\""
if needle not in text:
    raise SystemExit("failed to locate PR lightweight backend replay command")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$archive_profile_repo" --profile final >/dev/null 2>"$tmp_dir/archive-profile.log"; then
  echo "expected incorrect backend archive profile fixture to fail" >&2
  exit 1
fi

grep -q "backend-tests-lightweight must replay the expected backend profile from the workflow archive" "$tmp_dir/archive-profile.log"

archive_download_path_repo="$tmp_dir/archive-download-path-repo"
copy_repo_snapshot "$baseline_repo" "$archive_download_path_repo"
python3 - <<'PY' "$archive_download_path_repo"
from pathlib import Path
import sys

path = Path(sys.argv[1]) / ".github/workflows/ci-main.yml"
text = path.read_text()
start = text.index("  backend-tests-archive-file-io:\n")
end = text.index("\n  release-snapshot:", start)
job = text[start:end]
needle = "          path: ${{ runner.temp }}"
if needle not in job:
    raise SystemExit("failed to locate Main archive/file-I/O download path")
path.write_text(text[:start] + job.replace(needle, "          path: ${{ github.workspace }}", 1) + text[end:])
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$archive_download_path_repo" --profile final >/dev/null 2>"$tmp_dir/archive-download-path.log"; then
  echo "expected incorrect backend archive download path fixture to fail" >&2
  exit 1
fi

grep -q "backend-tests-archive-file-io must download the backend test archive to runner.temp" "$tmp_dir/archive-download-path.log"

archive_cache_save_repo="$tmp_dir/archive-cache-save-repo"
copy_repo_snapshot "$baseline_repo" "$archive_cache_save_repo"
python3 - <<'PY' "$archive_cache_save_repo"
from pathlib import Path
import sys

path = Path(sys.argv[1]) / ".github/workflows/ci-pr.yml"
text = path.read_text()
needle = '''      - name: Save Cargo test artifacts
        if: ${{ steps.build-backend-test-archive.outcome == 'success' && steps.cargo-test-cache.outputs.cache-hit != 'true' }}
        continue-on-error: true
'''
replacement = needle.replace("        continue-on-error: true\n", "", 1)
if needle not in text:
    raise SystemExit("failed to locate backend archive cache-save policy")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$archive_cache_save_repo" --profile final >/dev/null 2>"$tmp_dir/archive-cache-save.log"; then
  echo "expected blocking backend cache-save fixture to fail" >&2
  exit 1
fi

grep -q "cache writes best-effort" "$tmp_dir/archive-cache-save.log"

clippy_cache_save_repo="$tmp_dir/clippy-cache-save-repo"
copy_repo_snapshot "$baseline_repo" "$clippy_cache_save_repo"
python3 - <<'PY' "$clippy_cache_save_repo"
from pathlib import Path
import sys

path = Path(sys.argv[1]) / ".github/workflows/ci-pr.yml"
text = path.read_text()
needle = '''      - name: Save Cargo Clippy artifacts
        if: ${{ steps.rust-source-quality.outcome == 'success' && steps.cargo-clippy-cache.outputs.cache-hit != 'true' }}
        continue-on-error: true
'''
replacement = needle.replace("        continue-on-error: true\n", "", 1)
if needle not in text:
    raise SystemExit("failed to locate Clippy cache-save policy")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$clippy_cache_save_repo" --profile final >/dev/null 2>"$tmp_dir/clippy-cache-save.log"; then
  echo "expected blocking Clippy cache-save fixture to fail" >&2
  exit 1
fi

grep -q "must save Clippy artifacts best-effort" "$tmp_dir/clippy-cache-save.log"

archive_name_repo="$tmp_dir/archive-name-repo"
copy_repo_snapshot "$baseline_repo" "$archive_name_repo"
python3 - <<'PY' "$archive_name_repo"
from pathlib import Path
import sys

path = Path(sys.argv[1]) / ".github/workflows/ci-pr.yml"
text = path.read_text()
needle = '''      - name: Upload backend test archive
        uses: actions/upload-artifact@v7
        with:
          name: backend-test-archive-${{ github.run_id }}
          path: ${{ runner.temp }}/backend-tests.tar.zst
          if-no-files-found: error
          overwrite: true
'''
replacement = needle.replace(
    "backend-test-archive-${{ github.run_id }}",
    "backend-test-archive-${{ github.run_id }}-${{ github.run_attempt }}",
    1,
)
if needle not in text:
    raise SystemExit("failed to locate PR run-scoped archive upload")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$archive_name_repo" --profile final >/dev/null 2>"$tmp_dir/archive-name.log"; then
  echo "expected attempt-scoped PR archive artifact fixture to fail" >&2
  exit 1
fi

grep -q "artifact must be run-scoped and replace the prior attempt" "$tmp_dir/archive-name.log"

smoke_guard_repo="$tmp_dir/smoke-guard-repo"
copy_repo_snapshot "$baseline_repo" "$smoke_guard_repo"
python3 - <<'PY' "$smoke_guard_repo"
from pathlib import Path
import sys

path = Path(sys.argv[1]) / ".github/workflows/ci-pr.yml"
text = path.read_text()
needle = '''      - name: Verify smoke artifact producer
        env:
          PRODUCER_RESULT: ${{ needs.build-pr-smoke-artifacts.result }}
        run: test "$PRODUCER_RESULT" = success
'''
replacement = needle.replace('run: test "$PRODUCER_RESULT" = success', "run: true", 1)
if needle not in text:
    raise SystemExit("failed to locate PR smoke producer-result command")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$smoke_guard_repo" --profile final >/dev/null 2>"$tmp_dir/smoke-guard.log"; then
  echo "expected inert PR smoke producer guard fixture to fail" >&2
  exit 1
fi

grep -q "ci-pr.yml.jobs.build must fail when the PR smoke artifact producer fails" "$tmp_dir/smoke-guard.log"

e2e_guard_repo="$tmp_dir/e2e-guard-repo"
copy_repo_snapshot "$baseline_repo" "$e2e_guard_repo"
python3 - <<'PY' "$e2e_guard_repo"
from pathlib import Path
import sys

path = Path(sys.argv[1]) / ".github/workflows/ci-pr.yml"
text = path.read_text()
needle = '''      - name: Verify E2E test producer
        env:
          PRODUCER_RESULT: ${{ needs.records-overlay-e2e-producer.result }}
        run: test "$PRODUCER_RESULT" = success
'''
replacement = needle.replace('run: test "$PRODUCER_RESULT" = success', "run: true", 1)
if needle not in text:
    raise SystemExit("failed to locate E2E producer-result command")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$e2e_guard_repo" --profile final >/dev/null 2>"$tmp_dir/e2e-guard.log"; then
  echo "expected inert E2E producer guard fixture to fail" >&2
  exit 1
fi

grep -q "ci-pr.yml.jobs.records-overlay-e2e must fail when the E2E test producer fails" "$tmp_dir/e2e-guard.log"

smoke_producer_repo="$tmp_dir/smoke-producer-repo"
copy_repo_snapshot "$baseline_repo" "$smoke_producer_repo"
python3 - <<'PY' "$smoke_producer_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/ci-pr.yml"
text = path.read_text()
needle = "    needs: build-pr-smoke-artifacts\n"
replacement = "    needs: backend-test-archive\n"
if needle not in text:
    raise SystemExit("failed to rewrite PR smoke artifact dependency")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$smoke_producer_repo" --profile final >/dev/null 2>"$tmp_dir/smoke-producer.log"; then
  echo "expected PR smoke artifact dependency fixture to fail" >&2
  exit 1
fi

grep -q "ci-pr.yml.jobs.build.needs must use the PR smoke artifact producer" "$tmp_dir/smoke-producer.log"

e2e_producer_repo="$tmp_dir/e2e-producer-repo"
copy_repo_snapshot "$baseline_repo" "$e2e_producer_repo"
python3 - <<'PY' "$e2e_producer_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/ci-pr.yml"
text = path.read_text()
needle = "    needs: records-overlay-e2e-producer\n"
replacement = "    needs: backend-test-archive\n"
if needle not in text:
    raise SystemExit("failed to rewrite E2E test producer dependency")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$e2e_producer_repo" --profile final >/dev/null 2>"$tmp_dir/e2e-producer.log"; then
  echo "expected E2E test producer dependency fixture to fail" >&2
  exit 1
fi

grep -q "ci-pr.yml.jobs.records-overlay-e2e.needs must use the E2E test producer" "$tmp_dir/e2e-producer.log"

for workflow_file in ci-pr.yml ci-main.yml; do
lint_target_repo="$tmp_dir/lint-target-${workflow_file%.yml}-repo"
copy_repo_snapshot "$baseline_repo" "$lint_target_repo"
python3 - <<'PY' "$lint_target_repo" "$workflow_file"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows" / sys.argv[2]
text = path.read_text()
needle = "            ~/.cargo/git\n"
if needle not in text:
    raise SystemExit("failed to locate lint cache paths")
path.write_text(text.replace(needle, f"{needle}            target\n", 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$lint_target_repo" --profile final >/dev/null 2>"$tmp_dir/lint-target-${workflow_file%.yml}.log"; then
  echo "expected $workflow_file lint target-cache fixture to fail" >&2
  exit 1
fi

grep -q "legacy cache must not restore Cargo target artifacts" "$tmp_dir/lint-target-${workflow_file%.yml}.log"
done

e2e_spec_repo="$tmp_dir/e2e-spec-repo"
copy_repo_snapshot "$baseline_repo" "$e2e_spec_repo"
python3 - <<'PY' "$e2e_spec_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/ci-pr.yml"
text = path.read_text()
needle = "demo-runtime.spec.ts"
if needle not in text:
    raise SystemExit("failed to locate Web Demo Playwright spec")
path.write_text(text.replace(needle, "demo-runtime-removed.spec.ts", 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$e2e_spec_repo" --profile final >/dev/null 2>"$tmp_dir/e2e-spec.log"; then
  echo "expected missing Web Demo Playwright spec fixture to fail" >&2
  exit 1
fi

grep -q "must run all required Playwright regression specs" "$tmp_dir/e2e-spec.log"

e2e_parallel_repo="$tmp_dir/e2e-parallel-repo"
copy_repo_snapshot "$baseline_repo" "$e2e_parallel_repo"
python3 - <<'PY' "$e2e_parallel_repo"
from pathlib import Path
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/ci-pr.yml"
text = path.read_text()
needle = "dashboard-render-performance.spec.ts || performance_status=$?"
replacement = "dashboard-render-performance.spec.ts &"
if needle not in text:
    raise SystemExit("failed to locate serial dashboard performance command")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$e2e_parallel_repo" --profile final >/dev/null 2>"$tmp_dir/e2e-parallel.log"; then
  echo "expected parallel E2E producer fixture to fail" >&2
  exit 1
fi

grep -q "must run all four Playwright specs serially" "$tmp_dir/e2e-parallel.log"

informational_repo="$tmp_dir/informational-repo"
copy_repo_snapshot "$baseline_repo" "$informational_repo"
python3 - <<'PY' "$informational_repo"
from pathlib import Path
import json
import sys
repo = Path(sys.argv[1])
path = repo / ".github/quality-gates.json"
payload = json.loads(path.read_text())
payload["informational_checks"] = ["Docs Preview"]
path.write_text(json.dumps(payload, indent=2) + "\n")
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$informational_repo" --profile final >/dev/null 2>"$tmp_dir/informational.log"; then
  echo "expected informational checks fixture to fail" >&2
  exit 1
fi

grep -q "informational_checks must stay empty" "$tmp_dir/informational.log"

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

release_ci_gate_repo="$tmp_dir/release-ci-gate-repo"
copy_repo_snapshot "$baseline_repo" "$release_ci_gate_repo"
python3 - <<'PY' "$release_ci_gate_repo"
from pathlib import Path
import sys
repo = Path(sys.argv[1])
path = repo / ".github/workflows/release.yml"
text = path.read_text()
needle = "${{ github.event_name == 'workflow_run' && github.event.workflow_run.conclusion != 'success' }}"
replacement = "${{ github.event_name == 'workflow_run' }}"
if needle not in text:
    raise SystemExit("failed to rewrite release CI Main Gate condition")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$release_ci_gate_repo" --profile final >/dev/null 2>"$tmp_dir/release-ci-gate.log"; then
  echo "expected release CI Main Gate fixture to fail" >&2
  exit 1
fi

grep -q "release.yml.jobs.ci-main-gate.if must stay" "$tmp_dir/release-ci-gate.log"

release_arm_retry_repo="$tmp_dir/release-arm-retry-repo"
copy_repo_snapshot "$baseline_repo" "$release_arm_retry_repo"
python3 - <<'PY' "$release_arm_retry_repo"
from pathlib import Path
import sys
repo = Path(sys.argv[1])
path = repo / ".github/workflows/release.yml"
text = path.read_text()
needle = "        run: ./.github/scripts/build-smoke-image-with-retry.sh\n"
replacement = "        run: docker buildx build --platform \"$BUILD_PLATFORM\" .\n"
if needle not in text:
    raise SystemExit("failed to rewrite arm64 retry helper step")
path.write_text(text.replace(needle, replacement, 1))
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$release_arm_retry_repo" --profile final >/dev/null 2>"$tmp_dir/release-arm-retry.log"; then
  echo "expected arm64 retry helper fixture to fail" >&2
  exit 1
fi

grep -q "arm64 smoke build must use the retry helper" "$tmp_dir/release-arm-retry.log"

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
