#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: run-backend-tests.sh [--profile lightweight|stateful-sqlite|archive-file-io]

Profiles:
  lightweight
  stateful-sqlite
  archive-file-io

If --profile is omitted, all three profiles run sequentially.
EOF
}

profile="all"
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

start_epoch="$(date +%s)"

if ! command -v cargo-nextest >/dev/null 2>&1; then
  echo "::error::cargo-nextest is not installed. Install it before running backend tests."
  exit 1
fi

run_profile() {
  local selected_profile="$1"
  local filter_expr=""
  local test_threads=""

  case "$selected_profile" in
    lightweight)
      filter_expr='(test(/^(tests|upstream_accounts::tests)::lightweight::/)) or (not test(/^(tests|upstream_accounts::tests)::/))'
      ;;
    stateful-sqlite)
      filter_expr='test(/^(tests|upstream_accounts::tests)::stateful_sqlite::/)'
      # Stateful tests use isolated in-memory SQLite pools and benefit from bounded I/O concurrency.
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

  local profile_start_epoch
  profile_start_epoch="$(date +%s)"
  echo "backend_test_profile=$selected_profile"
  if [[ -n "$test_threads" ]]; then
    echo "backend_test_profile_test_threads_${selected_profile//-/_}=$test_threads"
    cargo nextest run --locked --all-features --no-fail-fast --test-threads "$test_threads" -E "$filter_expr"
  else
    cargo nextest run --locked --all-features --no-fail-fast -E "$filter_expr"
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
