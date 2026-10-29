#!/usr/bin/env bash
set -euo pipefail

# Explicitly audited test names; never expand this to an entire profile without
# reviewing the added tests for pressure, load, and performance workloads.
selectors=scripts/postgres-acceptance/non-pressure-backend-tests.txt
for profile in lightweight stateful-sqlite archive-file-io; do
  case "$profile" in
    lightweight) namespace='::lightweight::' ;;
    stateful-sqlite) namespace='::stateful_sqlite::' ;;
    archive-file-io) namespace='::archive_file_io::' ;;
  esac
  filter=''
  while IFS= read -r name; do
    [[ "$name" == *"$namespace"* ]] || continue
    [[ "$name" =~ ^[a-zA-Z0-9_:]+$ ]] || exit 1
    [[ -z "$filter" ]] || filter+=' | '
    filter+="test(=$name)"
  done < "$selectors"
  [[ -n "$filter" ]]
  bash .github/scripts/run-backend-tests.sh --profile "$profile" --test-filter "$filter"
done
