#!/usr/bin/env bash
set -euo pipefail

source_path="${SUMMARY_TESTBOX_PRODUCTION_COPY_SOURCE:?SUMMARY_TESTBOX_PRODUCTION_COPY_SOURCE must be under /srv/codex}"
runner="${SUMMARY_TESTBOX_RUNNER:-/Users/ivan/.codex/skills/shared-testbox-runner/scripts/run-testbox.sh}"
testbox="${TESTBOX:-codex-testbox}"
repo_root="$(git rev-parse --show-toplevel)"

case "$source_path" in
  /srv/codex/*) ;;
  *) printf 'project-reason: production copy must be under /srv/codex\n' >&2; exit 64 ;;
esac
[[ "$source_path" != *..* ]] || {
  printf 'project-reason: production copy path contains a parent traversal\n' >&2
  exit 64
}
[[ -x "$runner" ]] || {
  printf 'project-reason: shared-testbox runner is unavailable\n' >&2
  exit 64
}

source_meta="$(ssh -o BatchMode=yes "$testbox" bash -s -- "$source_path" <<'REMOTE'
set -euo pipefail
source_path="$1"
[[ -e "$source_path" && ! -L "$source_path" ]] || exit 10
[[ "$(readlink -f -- "$source_path")" == "$source_path" ]] || exit 11
if find -P "$source_path" \( -type l -o -type b -o -type c -o -type p -o -type s \) -print -quit | grep -q .; then
  exit 12
fi
du -sb -- "$source_path" | awk '{print $1}'
REMOTE
)" || {
  printf 'project-reason: production copy failed canonical source validation\n' >&2
  exit 64
}
[[ "$source_meta" =~ ^[0-9]+$ ]] || {
  printf 'shared-testbox-environment: production copy size probe was unavailable\n' >&2
  exit 75
}

gib=$(( (source_meta + 1073741823) / 1073741824 ))
required_free=$((gib + 15))
runner_log="$(mktemp "${TMPDIR:-/tmp}/summary-production-run.XXXXXX")"
runner_pid=""
run_id=""
scratch_path=""
cleanup_runner() {
  if [[ -n "$runner_pid" ]] && kill -0 "$runner_pid" 2>/dev/null; then
    kill -TERM "$runner_pid" 2>/dev/null || true
    wait "$runner_pid" 2>/dev/null || true
  fi
  rm -f "$runner_log"
}
trap cleanup_runner EXIT

"$runner" container \
  --repo-root "$repo_root" \
  --testbox "$testbox" \
  --image rust:1.96-bookworm \
  --workdir /codex-scratch \
  --required-free-gib "$required_free" \
  --required-mem-gib 4 \
  -- bash -lc 'set -euo pipefail; while [[ ! -f /codex-scratch/READY ]]; do sleep 1; done; export SUMMARY_PRODUCTION_COPY=/codex-scratch/production-copy; export CARGO_TARGET_DIR=/codex-scratch/target; bash /workspace/scripts/validate-summary-production-fixture.sh' >"$runner_log" 2>&1 &
runner_pid="$!"

for _ in {1..120}; do
  run_id="$(sed -n 's/^testbox-run-id=//p' "$runner_log" | head -n 1)"
  [[ -n "$run_id" ]] && break
  if ! kill -0 "$runner_pid" 2>/dev/null; then
    wait "$runner_pid" || true
    printf 'shared-testbox-environment: runner ended before reserving a run\n' >&2
    exit 75
  fi
  sleep 1
done
[[ "$run_id" =~ ^[0-9]{8}_[0-9]{6}_[A-Za-z0-9._-]+$ ]] || {
  printf 'shared-testbox-environment: runner did not expose a valid run id\n' >&2
  exit 75
}

for _ in {1..120}; do
  scratch_path="$(ssh -o BatchMode=yes "$testbox" bash -s -- "$(id -un)" "$run_id" <<'REMOTE'
set -euo pipefail
workspace_root="/srv/codex/workspaces/$1"
run_id="$2"
find -P "$workspace_root" -path "*/runs/$run_id/.codex-scratch" -type d -print -quit
REMOTE
  )" || true
  if [[ -n "$scratch_path" && "$scratch_path" == /srv/codex/workspaces/*/runs/*/.codex-scratch ]]; then
    break
  fi
  if ! kill -0 "$runner_pid" 2>/dev/null; then
    wait "$runner_pid" || true
    printf 'shared-testbox-environment: runner ended before creating managed scratch\n' >&2
    exit 75
  fi
  sleep 1
done
[[ -n "$scratch_path" ]] || {
  printf 'shared-testbox-environment: managed scratch was not created\n' >&2
  exit 75
}

if ! ssh -o BatchMode=yes "$testbox" bash -s -- "$source_path" "$scratch_path" <<'REMOTE'
set -euo pipefail
source_path="$1"
scratch_path="$2"
[[ "$(readlink -f -- "$scratch_path")" == "$scratch_path" ]] || exit 20
[[ ! -e "$scratch_path/production-copy" && ! -L "$scratch_path/production-copy" ]] || exit 21
partial="$scratch_path/.production-copy.partial.$$"
trap 'rm -rf -- "$partial"' EXIT
if [[ -d "$source_path" ]]; then
  mkdir "$partial"
  cp -a -- "$source_path"/. "$partial"/
else
  mkdir "$partial"
  cp -a -- "$source_path" "$partial/data"
fi
if find -P "$partial" \( -type l -o -type b -o -type c -o -type p -o -type s \) -print -quit | grep -q .; then
  exit 22
fi
mv -- "$partial" "$scratch_path/production-copy"
trap - EXIT
touch "$scratch_path/READY"
REMOTE
then
  printf 'shared-testbox-environment: production copy staging failed\n' >&2
  kill -TERM "$runner_pid" 2>/dev/null || true
  wait "$runner_pid" 2>/dev/null || true
  exit 75
fi

set +e
wait "$runner_pid"
status="$?"
set -e
runner_pid=""
if [[ "$status" -eq 0 ]]; then
  printf 'production-copy-bytes=%s\n' "$source_meta"
  printf 'summary-production-validation=passed\n'
  exit 0
fi
if grep -q '^shared-testbox-environment:' "$runner_log"; then
  printf 'shared-testbox-environment: isolated production-copy validation failed\n' >&2
else
  printf 'project-reason: isolated production-copy validation failed\n' >&2
fi
exit "$status"
