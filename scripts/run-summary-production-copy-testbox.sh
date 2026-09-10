#!/usr/bin/env bash
set -euo pipefail

source_path="${SUMMARY_TESTBOX_PRODUCTION_COPY_SOURCE:?SUMMARY_TESTBOX_PRODUCTION_COPY_SOURCE must be under /srv/codex}"
runner="${SUMMARY_TESTBOX_RUNNER:-/Users/ivan/.codex/skills/shared-testbox-runner/scripts/run-testbox.sh}"
testbox="${TESTBOX:-codex-testbox}"
image="${SUMMARY_TESTBOX_IMAGE:-ghcr.io/ivanli-cn/codex-vibe-monitor:backend-test-7d185d46e5f685101b1c37385211840b67800f32}"
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
source_kib="$(du -sk "$repo_root" | awk '{print $1}')"
[[ "$source_kib" =~ ^[0-9]+$ ]] || {
  printf 'project-reason: workspace source size probe was unavailable\n' >&2
  exit 64
}
source_gib=$(( (source_kib + 1048575) / 1048576 ))
required_free=$((source_gib + gib + 15))
default_stage_wait_secs=$((900 + source_gib * 120))
stage_wait_secs="${SUMMARY_TESTBOX_STAGE_WAIT_SECS:-$default_stage_wait_secs}"
[[ "$stage_wait_secs" =~ ^[1-9][0-9]*$ ]] || {
  printf 'project-reason: stage wait timeout must be a positive integer\n' >&2
  exit 64
}
runner_log="$(mktemp "${TMPDIR:-/tmp}/summary-production-run.XXXXXX")"
runner_pid=""
run_id=""
scratch_path=""
validation_command="${SUMMARY_PRODUCTION_VALIDATION_COMMAND:-}"
validation_command_export=""
if [[ -n "$validation_command" ]]; then
  printf -v validation_command_export '%q' "$validation_command"
fi
cleanup_runner() {
  if [[ -n "$runner_pid" ]] && kill -0 "$runner_pid" 2>/dev/null; then
    kill -TERM "$runner_pid" 2>/dev/null || true
    wait "$runner_pid" 2>/dev/null || true
  fi
  rm -f "$runner_log"
}
trap cleanup_runner EXIT

container_command='set -euo pipefail; while [[ ! -f /codex-scratch/READY ]]; do sleep 1; done; export SUMMARY_PRODUCTION_COPY=/codex-scratch/production-copy; export CARGO_TARGET_DIR=/codex-scratch/target;'
for environment_name in \
  CARGO_NET_OFFLINE \
  SUMMARY_PRODUCTION_RECENT_READY_DEADLINE_SECS \
  SUMMARY_PRODUCTION_HISTORICAL_READY_DEADLINE_SECS \
  SUMMARY_PRODUCTION_RECOVERY_DIAGNOSTICS; do
  environment_value="${!environment_name:-}"
  [[ -n "$environment_value" ]] || continue
  if [[ "$environment_name" == CARGO_NET_OFFLINE && ! "$environment_value" =~ ^(true|false)$ ]]; then
    printf 'project-reason: %s must be true or false\n' "$environment_name" >&2
    exit 64
  fi
  if [[ "$environment_name" != CARGO_NET_OFFLINE && "$environment_name" != SUMMARY_PRODUCTION_RECOVERY_DIAGNOSTICS && ! "$environment_value" =~ ^[0-9]+$ ]]; then
    printf 'project-reason: %s must be numeric\n' "$environment_name" >&2
    exit 64
  fi
  printf -v escaped_environment_value '%q' "$environment_value"
  container_command+=" export ${environment_name}=${escaped_environment_value};"
done
if [[ -n "$validation_command_export" ]]; then
  container_command+=" export SUMMARY_PRODUCTION_VALIDATION_COMMAND=${validation_command_export};"
fi
container_command+=' bash /workspace/scripts/validate-summary-production-fixture.sh'

"$runner" container \
  --repo-root "$repo_root" \
  --testbox "$testbox" \
  --image "$image" \
  --workdir /codex-scratch \
  --required-free-gib "$required_free" \
  --required-mem-gib 4 \
  -- bash -c "$container_command" >"$runner_log" 2>&1 &
runner_pid="$!"

for ((attempt = 0; attempt < 120; attempt += 1)); do
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

for ((attempt = 0; attempt < stage_wait_secs; attempt += 1)); do
  scratch_path="$(ssh -o BatchMode=yes "$testbox" bash -s -- "$(id -un)" "$run_id" <<'REMOTE'
set -euo pipefail
workspace_root="/srv/codex/workspaces/$1"
run_id="$2"
find -P "$workspace_root" -path "*/controls/$run_id/.codex-scratch" -type d -print -quit
REMOTE
  )" || true
  if [[ -n "$scratch_path" && "$scratch_path" == /srv/codex/workspaces/*/controls/*/.codex-scratch ]]; then
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
  cp -a -- "$source_path" "$partial/codex_vibe_monitor.db"
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
grep -E '^(production-copy-bytes=|summary-production-(sqlite|health|startup-phases|window|bootstrap|exactness|overlay|recovery(-telemetry)?|validation)=)' "$runner_log" || true
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
