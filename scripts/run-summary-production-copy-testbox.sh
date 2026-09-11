#!/usr/bin/env bash
set -euo pipefail

path_has_parent_component() {
  local value="$1"
  local -a components
  local component
  IFS=/ read -r -a components <<<"$value"
  for component in "${components[@]}"; do
    if [[ "$component" == .. || "$component" == . ]]; then
      return 0
    fi
  done
  return 1
}

path_is_normalized_absolute() {
  local value="$1"
  [[ "$value" == /* && "$value" != *//* ]]
}

agent_id="${CODEX_THREAD_ID:-}"
[[ "$agent_id" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] || {
  printf 'project-reason: CODEX_THREAD_ID must be a safe non-empty agent identifier\n' >&2
  exit 64
}
testbox="${TESTBOX:-codex-testbox}"
[[ "$testbox" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] || {
  printf 'project-reason: testbox host contains unsupported characters\n' >&2
  exit 64
}

candidate_sha="${SUMMARY_TESTBOX_COMMIT_SHA:-$(git rev-parse HEAD)}"
image="${SUMMARY_TESTBOX_IMAGE:-ghcr.io/ivanli-cn/codex-vibe-monitor:backend-test-${candidate_sha}}"
repo_root="$(cd "$(git rev-parse --show-toplevel)" && pwd -P)"
agent_dir="/srv/codex/agents/$agent_id"
workspace_path="$agent_dir/workspace"
run_id="$(date -u +%Y%m%d_%H%M%S)_summary_production_${candidate_sha}_$$"
run_path="$agent_dir/runs/$run_id"
source_path="${SUMMARY_TESTBOX_PRODUCTION_COPY_SOURCE:?SUMMARY_TESTBOX_PRODUCTION_COPY_SOURCE must be inside the current Agent Directory}"
while [[ "$source_path" != "/" && "$source_path" == */ ]]; do
  source_path="${source_path%/}"
done
path_is_normalized_absolute "$source_path" && ! path_has_parent_component "$source_path" || {
  printf 'project-reason: production copy path must be a normalized absolute path without parent traversal\n' >&2
  exit 64
}
[[ "$source_path" =~ ^/srv/codex/agents/[A-Za-z0-9._/-]+$ ]] || {
  printf 'project-reason: production copy path contains unsupported characters\n' >&2
  exit 64
}
case "$source_path" in
  "$agent_dir"/*) ;;
  *)
    printf 'project-reason: production copy must be inside the current Agent Directory\n' >&2
    exit 64
    ;;
esac
case "$source_path" in
  "$workspace_path"|"$workspace_path"/*)
    printf 'project-reason: production copy must not overlap the synced worktree\n' >&2
    exit 64
    ;;
esac

for environment_name in CARGO_HOME CARGO_TARGET_DIR; do
  environment_value="${!environment_name:-}"
  [[ -n "$environment_value" ]] || continue
  path_is_normalized_absolute "$environment_value" && ! path_has_parent_component "$environment_value" || {
    printf 'project-reason: %s must be a normalized absolute path without parent traversal\n' "$environment_name" >&2
    exit 64
  }
  case "$environment_value" in
    /srv/codex/*) ;;
    *)
      printf 'project-reason: %s must stay under /srv/codex on the shared testbox\n' "$environment_name" >&2
      exit 64
      ;;
  esac
  [[ "$environment_value" =~ ^/srv/codex/[A-Za-z0-9._/-]+$ ]] || {
    printf 'project-reason: %s contains unsupported path characters\n' "$environment_name" >&2
    exit 64
  }
done

for environment_name in \
  SUMMARY_PRODUCTION_RECENT_READY_DEADLINE_SECS \
  SUMMARY_PRODUCTION_HISTORICAL_READY_DEADLINE_SECS; do
  environment_value="${!environment_name:-}"
  [[ -z "$environment_value" || "$environment_value" =~ ^[0-9]+$ ]] || {
    printf 'project-reason: %s must be numeric\n' "$environment_name" >&2
    exit 64
  }
done

printf '== testbox agent ==\n%s\n' "$agent_dir"
printf '== testbox run ==\n%s\n' "$run_path"
printf '== image ==\n%s\n' "$image"

source_meta=""
set +e
source_meta="$(ssh -o BatchMode=yes -o ConnectTimeout=5 "$testbox" bash -s -- "$agent_id" "$source_path" <<'REMOTE'
set -euo pipefail
agent_id="$1"
source_path="$2"
base=/srv/codex/agents
agent_dir="$base/$agent_id"
[[ "$agent_id" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]
[[ -d "$base" && ! -L "$base" ]]
if [[ -e "$agent_dir" || -L "$agent_dir" ]]; then
  [[ -d "$agent_dir" && ! -L "$agent_dir" ]]
else
  mkdir -- "$agent_dir"
fi
[[ "$(readlink -f -- "$agent_dir")" == "$agent_dir" ]]
[[ "$source_path" == "$agent_dir"/* ]]
[[ -e "$source_path" && ! -L "$source_path" ]] || exit 10
[[ "$(readlink -f -- "$source_path")" == "$source_path" ]] || exit 11
if find -P "$source_path" \( -type l -o -type b -o -type c -o -type p -o -type s \) -print -quit | grep -q .; then
  exit 12
fi
du -sb -- "$source_path" | awk '{print $1}'
REMOTE
)"
source_validation_status="$?"
set -e
case "$source_validation_status" in
  10|11|12)
    printf 'project-reason: production copy failed canonical source validation\n' >&2
    exit 64
    ;;
  0) ;;
  *)
    printf 'shared-testbox-environment: production copy source validation was unavailable\n' >&2
    exit 75
    ;;
esac
[[ "$source_meta" =~ ^[0-9]+$ ]] || {
  printf 'shared-testbox-environment: production copy size probe was unavailable\n' >&2
  exit 75
}

ssh -o BatchMode=yes -o ConnectTimeout=5 "$testbox" bash -s -- "$agent_id" "$workspace_path" <<'REMOTE'
set -euo pipefail
agent_id="$1"
workspace_path="$2"
base=/srv/codex/agents
agent_dir="$base/$agent_id"
[[ -d "$base" && ! -L "$base" ]]
[[ -d "$agent_dir" && ! -L "$agent_dir" ]]
[[ "$(readlink -f -- "$agent_dir")" == "$agent_dir" ]]
if [[ -e "$workspace_path" || -L "$workspace_path" ]]; then
  [[ -d "$workspace_path" && ! -L "$workspace_path" ]]
else
  mkdir -- "$workspace_path"
fi
[[ "$(readlink -f -- "$workspace_path")" == "$workspace_path" ]]
find "$workspace_path" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
REMOTE
if git -C "$repo_root" ls-files -s | awk '$1 == "160000" { found=1 } END { exit !found }'; then
  printf 'project-reason: submodules must be transferred directly; synced worktree only supports ordinary worktrees\n' >&2
  exit 64
fi
git -C "$repo_root" ls-files -co --exclude-standard -z |
  while IFS= read -r -d '' path; do
    if [[ -e "$repo_root/$path" || -L "$repo_root/$path" ]]; then
      printf '%s\0' "$path"
    fi
  done |
  rsync -az --from0 --files-from=- "$repo_root/" "$testbox:$workspace_path/"

runner_log="$(mktemp "${TMPDIR:-/tmp}/summary-production-run.XXXXXX")"
cleanup_remote() {
  ssh -o BatchMode=yes -o ConnectTimeout=5 "$testbox" bash -s -- "$agent_id" "$run_path" <<'REMOTE' >/dev/null 2>&1 || true
set -euo pipefail
agent_id="$1"
run_path="$2"
base=/srv/codex/agents
agent_dir="$base/$agent_id"
run_parent="$agent_dir/runs"
[[ "$agent_id" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]
[[ -d "$base" && ! -L "$base" ]]
[[ -d "$agent_dir" && ! -L "$agent_dir" ]]
[[ "$(readlink -f -- "$agent_dir")" == "$agent_dir" ]]
[[ -d "$run_parent" && ! -L "$run_parent" ]]
[[ "$(readlink -f -- "$run_parent")" == "$run_parent" ]]
case "$run_path" in
  "$run_parent"/summary_production_*) rm -rf -- "$run_path" ;;
esac
REMOTE
  rm -f "$runner_log"
}
trap cleanup_remote EXIT

set +e
ssh -o BatchMode=yes -o ConnectTimeout=5 "$testbox" env \
  SOURCE_PATH="$source_path" \
  IMAGE="$image" \
  CARGO_HOME_INPUT="${CARGO_HOME:-}" \
  CARGO_TARGET_INPUT="${CARGO_TARGET_DIR:-}" \
  CARGO_NET_OFFLINE_INPUT="${CARGO_NET_OFFLINE:-}" \
  RECENT_READY_DEADLINE="${SUMMARY_PRODUCTION_RECENT_READY_DEADLINE_SECS:-}" \
  HISTORICAL_READY_DEADLINE="${SUMMARY_PRODUCTION_HISTORICAL_READY_DEADLINE_SECS:-}" \
  RECOVERY_DIAGNOSTICS="${SUMMARY_PRODUCTION_RECOVERY_DIAGNOSTICS:-}" \
  bash -s -- "$agent_id" "$workspace_path" "$run_path" <<'REMOTE' >"$runner_log" 2>&1
set -euo pipefail
agent_id="$1"
workspace_path="$2"
run_path="$3"
base=/srv/codex/agents
agent_dir="$base/$agent_id"
run_parent="$agent_dir/runs"
[[ "$agent_id" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]
[[ -d "$base" && ! -L "$base" ]]
[[ -d "$agent_dir" && ! -L "$agent_dir" ]]
[[ "$(readlink -f -- "$agent_dir")" == "$agent_dir" ]]
if [[ -e "$run_parent" || -L "$run_parent" ]]; then
  [[ -d "$run_parent" && ! -L "$run_parent" ]]
else
  mkdir -- "$run_parent"
fi
[[ "$(readlink -f -- "$run_parent")" == "$run_parent" ]]
case "$run_path" in
  "$run_parent"/*) ;;
  *) exit 64 ;;
esac
if [[ -e "$workspace_path" || -L "$workspace_path" ]]; then
  [[ -d "$workspace_path" && ! -L "$workspace_path" ]]
else
  mkdir -- "$workspace_path"
fi
[[ "$(readlink -f -- "$workspace_path")" == "$workspace_path" ]]
if [[ -e "$run_path" || -L "$run_path" ]]; then
  [[ -d "$run_path" && ! -L "$run_path" ]]
else
  mkdir -- "$run_path"
fi
[[ "$(readlink -f -- "$run_path")" == "$run_path" ]]
chmod 0777 "$run_path"

if [[ -e "$run_path/production-copy" || -L "$run_path/production-copy" ]]; then
  echo 'production copy run directory is not empty' >&2
  exit 64
fi
partial="$run_path/.production-copy.partial.$$"
trap 'rm -rf -- "$partial"' EXIT
if [[ -d "$SOURCE_PATH" ]]; then
  mkdir -- "$partial"
  cp -a -- "$SOURCE_PATH"/. "$partial"/
elif [[ -f "$SOURCE_PATH" ]]; then
  mkdir -- "$partial"
  cp -a -- "$SOURCE_PATH" "$partial/codex_vibe_monitor.db"
else
  echo 'production copy source disappeared before staging' >&2
  exit 64
fi
if find -P "$partial" \( -type l -o -type b -o -type c -o -type p -o -type s \) -print -quit | grep -q .; then
  echo 'production copy contains unsupported filesystem entries' >&2
  exit 64
fi
mv -- "$partial" "$run_path/production-copy"
trap - EXIT
touch "$run_path/READY"

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo 'shared-testbox-environment: requested validation image is unavailable' >&2
  exit 75
fi
docker_args=(
  run --rm --cap-drop=ALL --user 65534:65534
  --entrypoint /bin/bash
  -v "$workspace_path:/workspace:ro"
  -v "$run_path:/codex-scratch:rw"
  -w /workspace
)
for environment_name in \
  CARGO_HOME_INPUT \
  CARGO_TARGET_INPUT \
  CARGO_NET_OFFLINE_INPUT \
  RECENT_READY_DEADLINE \
  HISTORICAL_READY_DEADLINE \
  RECOVERY_DIAGNOSTICS; do
  environment_value="${!environment_name:-}"
  [[ -n "$environment_value" ]] || continue
  container_name="$environment_name"
  case "$environment_name" in
    CARGO_HOME_INPUT) container_name=CARGO_HOME ;;
    CARGO_TARGET_INPUT) container_name=CARGO_TARGET_DIR ;;
    CARGO_NET_OFFLINE_INPUT) container_name=CARGO_NET_OFFLINE ;;
    RECENT_READY_DEADLINE) container_name=SUMMARY_PRODUCTION_RECENT_READY_DEADLINE_SECS ;;
    HISTORICAL_READY_DEADLINE) container_name=SUMMARY_PRODUCTION_HISTORICAL_READY_DEADLINE_SECS ;;
    RECOVERY_DIAGNOSTICS) container_name=SUMMARY_PRODUCTION_RECOVERY_DIAGNOSTICS ;;
  esac
  docker_args+=(-e "$container_name=$environment_value")
  case "$environment_name" in
    CARGO_HOME_INPUT|CARGO_TARGET_INPUT)
      mkdir -p -- "$environment_value"
      docker_args+=(-v "$environment_value:$environment_value:rw")
      ;;
  esac
done
docker_args+=("$IMAGE" -c 'export SUMMARY_PRODUCTION_COPY=/codex-scratch/production-copy; bash /workspace/scripts/validate-summary-production-fixture.sh')
"${docker_args[@]}"
REMOTE
remote_status="$?"
set -e

grep -E '^(production-copy-bytes=|summary-production-(cache-mode|network-mode|sqlite|health|startup-phases|window|bootstrap|exactness|overlay|recovery(-telemetry)?|validation)=)' "$runner_log" || true
if [[ "$remote_status" -eq 0 ]]; then
  printf 'production-copy-bytes=%s\n' "$source_meta"
  printf 'summary-production-validation=passed\n'
  exit 0
fi
if grep -q '^shared-testbox-environment:' "$runner_log"; then
  printf 'shared-testbox-environment: isolated production-copy validation failed\n' >&2
else
  printf 'project-reason: isolated production-copy validation failed\n' >&2
fi
exit "$remote_status"
