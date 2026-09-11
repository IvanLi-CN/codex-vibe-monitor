#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source_snapshot_root="$(cd "$repo_root" && pwd -P)"

usage() {
  cat <<'EOF'
Usage: run-backend-tests.sh [--profile lightweight|stateful-sqlite|archive-file-io] [--archive-file PATH] [--test-filter EXPR] [--partition hash:N/M]

Profiles:
  lightweight
  stateful-sqlite
  archive-file-io

If --profile is omitted, all three profiles run sequentially.

When --archive-file is set, run profiles from an existing cargo-nextest archive
instead of building test binaries in this invocation.

When --test-filter is set, replace the profile's default nextest filter while
retaining the profile's schema-template and workspace contract.

When --partition is set, pass one deterministic cargo-nextest hash partition to
the selected profile. The value must be hash:N/M with 1 <= N <= M.
EOF
}

profile="all"
archive_file=""
test_filter_override=""
partition=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      if [[ $# -lt 2 ]]; then
        echo "::error::--profile requires a value." >&2
        usage >&2
        exit 1
      fi
      profile="$2"
      shift 2
      ;;
    --archive-file)
      if [[ $# -lt 2 ]]; then
        echo "::error::--archive-file requires a path." >&2
        usage >&2
        exit 1
      fi
      archive_file="$2"
      shift 2
      ;;
    --test-filter)
      if [[ $# -lt 2 ]]; then
        echo "::error::--test-filter requires a value." >&2
        usage >&2
        exit 1
      fi
      test_filter_override="$2"
      shift 2
      ;;
    --partition)
      if [[ $# -lt 2 ]]; then
        echo "::error::--partition requires a value." >&2
        usage >&2
        exit 64
      fi
      partition="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "::error::unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -n "$partition" ]]; then
  if [[ ! "$partition" =~ ^hash:([0-9]+)/([0-9]+)$ ]]; then
    echo "::error::--partition must use hash:N/M." >&2
    exit 64
  fi
  partition_index="${BASH_REMATCH[1]}"
  partition_count="${BASH_REMATCH[2]}"
  if (( partition_index < 1 || partition_count < 1 || partition_index > partition_count )); then
    echo "::error::--partition requires 1 <= N <= M." >&2
    exit 64
  fi
fi

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

path_is_within() {
  local candidate="$1"
  local parent="$2"
  [[ "$candidate" == "$parent" || "$candidate" == "$parent"/* ]]
}

path_overlaps() {
  local left="$1"
  local right="$2"
  path_is_within "$left" "$right" || path_is_within "$right" "$left"
}

canonical_dir_path() {
  local variable_name="$1"
  local raw_path="$2"
  if [[ "$raw_path" != /* ]] || path_has_parent_component "$raw_path"; then
    echo "::error::$variable_name must be a normalized absolute path without parent components." >&2
    exit 64
  fi
  local probe="$raw_path"
  while [[ ! -e "$probe" ]]; do
    [[ "$probe" != "/" ]] || break
    probe="${probe%/*}"
    [[ -n "$probe" ]] || probe="/"
  done
  [[ -d "$probe" ]] || {
    echo "::error::$variable_name parent could not be canonicalized." >&2
    exit 64
  }
  local canonical_probe
  canonical_probe="$(cd "$probe" && pwd -P)" || {
    echo "::error::$variable_name could not be canonicalized." >&2
    exit 64
  }
  local suffix="${raw_path#"$probe"}"
  local canonical_path="${canonical_probe%/}${suffix}"
  while [[ "$canonical_path" != "/" && "$canonical_path" == */ ]]; do
    canonical_path="${canonical_path%/}"
  done
  if [[ "$canonical_path" == / ]]; then
    echo "::error::$variable_name must resolve to a non-root directory." >&2
    exit 64
  fi
  printf '%s\n' "$canonical_path"
}

ensure_writable_dir() {
  local variable_name="$1"
  local path="$2"
  mkdir -p "$path" || {
    echo "::error::could not create $variable_name directory." >&2
    exit 64
  }
  if [[ ! -d "$path" || ! -w "$path" ]]; then
    echo "::error::$variable_name must resolve to a writable directory." >&2
    exit 64
  fi
}

workspace_input="${BACKEND_TEST_WORKSPACE:-/tmp/codex-vibe-monitor-backend-test}"
backend_test_workspace="$(canonical_dir_path BACKEND_TEST_WORKSPACE "$workspace_input")"

cargo_home_input="${CARGO_HOME:-}"
cargo_target_input="${CARGO_TARGET_DIR:-}"
cargo_home_external=false
cargo_target_external=false
if [[ -z "$cargo_home_input" ]]; then
  cargo_home="$backend_test_workspace/cargo-home"
  cache_mode="ephemeral"
else
  cargo_home="$(canonical_dir_path CARGO_HOME "$cargo_home_input")"
  cache_mode="external"
  cargo_home_external=true
fi
if [[ -z "$cargo_target_input" ]]; then
  cargo_target_dir="$backend_test_workspace/target"
else
  cargo_target_dir="$(canonical_dir_path CARGO_TARGET_DIR "$cargo_target_input")"
  cache_mode="external"
  cargo_target_external=true
fi
if path_overlaps "$backend_test_workspace" "$source_snapshot_root"; then
  echo "::error::BACKEND_TEST_WORKSPACE must be separate from the source snapshot." >&2
  exit 64
fi
if { [[ "$cargo_home_external" == true ]] && path_overlaps "$cargo_home" "$backend_test_workspace"; } \
  || { [[ "$cargo_target_external" == true ]] && path_overlaps "$cargo_target_dir" "$backend_test_workspace"; } \
  || { [[ "$cargo_home_external" == true ]] && path_overlaps "$cargo_home" "$source_snapshot_root"; } \
  || { [[ "$cargo_target_external" == true ]] && path_overlaps "$cargo_target_dir" "$source_snapshot_root"; }; then
  echo "::error::externally provided Cargo directories must be outside BACKEND_TEST_WORKSPACE." >&2
  exit 64
fi
if path_overlaps "$cargo_home" "$cargo_target_dir"; then
  echo "::error::CARGO_HOME and CARGO_TARGET_DIR must be separate directories." >&2
  exit 64
fi
ensure_writable_dir BACKEND_TEST_WORKSPACE "$backend_test_workspace"
ensure_writable_dir CARGO_HOME "$cargo_home"
ensure_writable_dir CARGO_TARGET_DIR "$cargo_target_dir"
export CARGO_HOME="$cargo_home"
export CARGO_TARGET_DIR="$cargo_target_dir"

offline_mode="online"
case "${CARGO_NET_OFFLINE:-}" in
  1|true|yes) offline_mode="offline" ;;
esac
echo "backend_test_cache_mode=$cache_mode"
echo "backend_test_network_mode=$offline_mode"
if command -v rustc >/dev/null 2>&1; then
  echo "backend_test_rustc_version=$(rustc --version)"
else
  echo "backend_test_rustc_version=unavailable"
fi

start_epoch="$(date +%s)"
schema_template_dir=""

cleanup_schema_template() {
  if [[ -n "$schema_template_dir" && -d "$schema_template_dir" ]]; then
    rm -rf "$schema_template_dir"
  fi
  schema_template_dir=""
}
trap cleanup_schema_template EXIT

# The pool routing/live-first test profiles now exercise async paths that exceed the
# default Rust thread stack on CI workers. Raise the per-thread minimum for the
# backend test binary unless the caller already set a stronger value.
if [[ -z "${RUST_MIN_STACK:-}" ]]; then
  export RUST_MIN_STACK=$((8 * 1024 * 1024))
fi
echo "backend_test_rust_min_stack_bytes=$RUST_MIN_STACK"

if ! command -v cargo-nextest >/dev/null 2>&1; then
  echo "::error::cargo-nextest is not installed. Install it before running backend tests."
  exit 1
fi

prepare_schema_template() {
  local selected_profile="$1"
  cleanup_schema_template
  schema_template_dir="$(mktemp -d "$backend_test_workspace/${selected_profile}-schema.XXXXXX")"
  local template_path="$schema_template_dir/current-schema.db"
  case "$selected_profile" in
    stateful-sqlite)
      export CODEX_VIBE_MONITOR_STATEFUL_SCHEMA_TEMPLATE_PATH="$template_path"
      echo "backend_test_stateful_schema_template=prepared"
      ;;
    archive-file-io)
      export CODEX_VIBE_MONITOR_ARCHIVE_SCHEMA_TEMPLATE_PATH="$template_path"
      echo "backend_test_archive_schema_template=prepared"
      ;;
    *)
      echo "::error::schema templates are unsupported for profile: $selected_profile" >&2
      exit 1
      ;;
  esac

  local template_filter='test(=tests::prepare_current_schema_template_for_stateful_profile)'
  if [[ -n "$archive_file" ]]; then
    cargo nextest run --archive-file "$archive_file" --no-fail-fast -E "$template_filter"
  else
    cargo nextest run --locked --all-features --no-fail-fast -E "$template_filter"
  fi
}

run_profile() {
  local selected_profile="$1"
  local filter_expr=""
  local test_threads=""

  case "$selected_profile" in
    lightweight)
      filter_expr='(test(/^(tests|upstream_accounts::tests)::lightweight::/)) or (not test(/^(tests|upstream_accounts::tests)::/))'
      # Keep SQLite-backed lightweight tests serialized to avoid connection-pool
      # contention on shared CI workers.
      test_threads="1"
      ;;
    stateful-sqlite)
      filter_expr='test(/^(tests|upstream_accounts::tests)::stateful_sqlite::/)'
      # The 4/6/8 hot-run matrix selected the lowest tier within 10% of the fastest mean.
      test_threads="6"
      ;;
    archive-file-io)
      filter_expr='test(/^(tests|upstream_accounts::tests)::archive_file_io::/)'
      ;;
    *)
      echo "::error::unsupported backend test profile: $selected_profile" >&2
      usage >&2
      exit 1
      ;;
  esac

  if [[ -n "$test_filter_override" ]]; then
    filter_expr="$test_filter_override"
  fi

  # Only the selected profile may consume its private current-schema template.
  # Caller-provided values must not leak fixture behavior across profiles.
  if [[ "$selected_profile" != "stateful-sqlite" ]]; then
    unset CODEX_VIBE_MONITOR_STATEFUL_SCHEMA_TEMPLATE_PATH
  fi
  if [[ "$selected_profile" != "archive-file-io" ]]; then
    unset CODEX_VIBE_MONITOR_ARCHIVE_SCHEMA_TEMPLATE_PATH
  fi

  local profile_start_epoch
  profile_start_epoch="$(date +%s)"
  echo "backend_test_profile=$selected_profile"
  if [[ "$selected_profile" == "stateful-sqlite" || "$selected_profile" == "archive-file-io" ]]; then
    prepare_schema_template "$selected_profile"
  fi
  nextest_args=(nextest run)
  if [[ -n "$archive_file" ]]; then
    nextest_args+=(--archive-file "$archive_file" --no-fail-fast)
  else
    nextest_args+=(--locked --all-features --no-fail-fast)
  fi
  if [[ -n "$test_threads" ]]; then
    echo "backend_test_profile_test_threads_${selected_profile//-/_}=$test_threads"
    nextest_args+=(--test-threads "$test_threads")
  fi
  if [[ -n "$partition" ]]; then
    nextest_args+=(--partition "$partition")
  fi
  nextest_args+=(-E "$filter_expr")
  cargo "${nextest_args[@]}"
  if [[ "$selected_profile" == "stateful-sqlite" ]]; then
    unset CODEX_VIBE_MONITOR_STATEFUL_SCHEMA_TEMPLATE_PATH
  fi
  if [[ "$selected_profile" == "archive-file-io" ]]; then
    unset CODEX_VIBE_MONITOR_ARCHIVE_SCHEMA_TEMPLATE_PATH
  fi
  local profile_end_epoch
  profile_end_epoch="$(date +%s)"
  echo "backend_test_profile_seconds_${selected_profile//-/_}=$((profile_end_epoch - profile_start_epoch))"
}

if [[ "$profile" == "all" ]]; then
  run_profile lightweight
  run_profile stateful-sqlite
  run_profile archive-file-io
else
  run_profile "$profile"
fi

end_epoch="$(date +%s)"
echo "backend_test_total_seconds=$((end_epoch - start_epoch))"
