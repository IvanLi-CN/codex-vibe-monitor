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
import re
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/label-gate.yml"
text = path.read_text()
pattern = re.compile(
    r"(      - name: Validate release intent labels\n        uses: actions/github-script@v8\n        with:\n          github-token: \$\{\{ secrets\.GITHUB_TOKEN \}\}\n          script: \|\n)(?:            .*\n)+",
    re.M,
)
replacement = """      - name: Validate release intent labels
        uses: actions/github-script@v8
        with:
          github-token: ${{ secrets.GITHUB_TOKEN }}
          script: |
            core.info('label gate validated 1 pull request(s)');
            // github.rest.issues.get
            // channel:stable
            // type:patch
            // type:skip
"""
rewritten = pattern.sub(replacement, text)
if rewritten == text:
    raise SystemExit("failed to rewrite label-gate workflow")
path.write_text(rewritten)
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$label_repo" >/dev/null 2>"$tmp_dir/label-gate-comment-bypass.log"; then
  echo "expected label-gate comment bypass fixture to fail" >&2
  exit 1
fi

grep -q "label gate must enforce channel labels" "$tmp_dir/label-gate-comment-bypass.log"

review_repo="$tmp_dir/review-repo"
cp -R "$repo_root/." "$review_repo"
python3 - <<'PY' "$review_repo"
from pathlib import Path
import re
import sys

repo = Path(sys.argv[1])
path = repo / ".github/workflows/review-policy.yml"
text = path.read_text()
pattern = re.compile(
    r"(      - name: Evaluate review policy\n        uses: actions/github-script@v8\n        with:\n          github-token: \$\{\{ secrets\.GITHUB_TOKEN \}\}\n          script: \|\n)(?:            .*\n)+",
    re.M,
)
replacement = """      - name: Evaluate review policy
        uses: actions/github-script@v8
        with:
          github-token: ${{ secrets.GITHUB_TOKEN }}
          script: |
            core.info('review gate validated 1 pull request(s)');
            // createCommitStatus
            // GET /repos/{owner}/{repo}/collaborators/{username}/permission
            // GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews
            // const reviewRequiredApprovals = 1;
            // 'admin' 'maintain' 'write'
            // const reviewGateContext = 'Review Policy Gate';
            // Author @${author} has ${authorPermission} permission; approval not required.
"""
rewritten = pattern.sub(replacement, text)
if rewritten == text:
    raise SystemExit("failed to rewrite review-policy workflow")
path.write_text(rewritten)
PY

if python3 "$repo_root/.github/scripts/check_quality_gates_contract.py" --repo-root "$review_repo" >/dev/null 2>"$tmp_dir/review-policy-comment-bypass.log"; then
  echo "expected review-policy comment bypass fixture to fail" >&2
  exit 1
fi

grep -q "required approvals drifted" "$tmp_dir/review-policy-comment-bypass.log"

echo "test-quality-gates-contract: all checks passed"
