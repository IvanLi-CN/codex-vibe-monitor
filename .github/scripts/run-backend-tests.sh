#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: run-backend-tests.sh [--profile lightweight|stateful-sqlite|archive-file-io] [--archive-file PATH] [--test-filter EXPR]

Profiles:
  lightweight
  stateful-sqlite
  archive-file-io

If --profile is omitted, all three profiles run sequentially.

When --archive-file is set, run profiles from an existing cargo-nextest archive
instead of building test binaries in this invocation.

When --test-filter is set, replace the profile's default nextest filter while
retaining the profile's schema-template and workspace contract.
EOF
}

profile="all"
archive_file=""
test_filter_override=""
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

backend_test_workspace="${BACKEND_TEST_WORKSPACE:-/tmp/codex-vibe-monitor-backend-test}"
if [[ "$backend_test_workspace" != /tmp/* || "$backend_test_workspace" == *..* ]]; then
  echo "::error::BACKEND_TEST_WORKSPACE must be a path under /tmp without '..'." >&2
  exit 64
fi
mkdir -p "$backend_test_workspace"
if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  export CARGO_TARGET_DIR="$backend_test_workspace/target"
fi
if [[ "$CARGO_TARGET_DIR" != "$backend_test_workspace"/* || "$CARGO_TARGET_DIR" == *..* ]]; then
  echo "::error::CARGO_TARGET_DIR must be inside BACKEND_TEST_WORKSPACE." >&2
  exit 64
fi
mkdir -p "$CARGO_TARGET_DIR"

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
      echo "backend_test_stateful_schema_template=$template_path"
      ;;
    archive-file-io)
      export CODEX_VIBE_MONITOR_ARCHIVE_SCHEMA_TEMPLATE_PATH="$template_path"
      echo "backend_test_archive_schema_template=$template_path"
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
  if [[ -n "$test_threads" ]]; then
    echo "backend_test_profile_test_threads_${selected_profile//-/_}=$test_threads"
    if [[ -n "$archive_file" ]]; then
      cargo nextest run --archive-file "$archive_file" --no-fail-fast --test-threads "$test_threads" -E "$filter_expr"
    else
      cargo nextest run --locked --all-features --no-fail-fast --test-threads "$test_threads" -E "$filter_expr"
    fi
  else
    if [[ -n "$archive_file" ]]; then
      cargo nextest run --archive-file "$archive_file" --no-fail-fast -E "$filter_expr"
    else
      cargo nextest run --locked --all-features --no-fail-fast -E "$filter_expr"
    fi
  fi
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
