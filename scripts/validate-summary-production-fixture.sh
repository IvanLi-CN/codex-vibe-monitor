#!/usr/bin/env bash
set -euo pipefail

copy_path="${SUMMARY_PRODUCTION_COPY:?SUMMARY_PRODUCTION_COPY is required}"
[[ -e "$copy_path" && ! -L "$copy_path" ]] || {
  printf 'project-reason: staged production copy is missing or symbolic\n' >&2
  exit 64
}
[[ "$(readlink -f -- "$copy_path")" == "$copy_path" ]] || {
  printf 'project-reason: staged production copy is not canonical\n' >&2
  exit 64
}
if find -P "$copy_path" \( -type l -o -type b -o -type c -o -type p -o -type s \) -print -quit | grep -q .; then
  printf 'project-reason: staged production copy contains unsupported filesystem entries\n' >&2
  exit 64
fi

copy_bytes="$(du -sb -- "$copy_path" | awk '{print $1}')"
[[ "$copy_bytes" =~ ^[0-9]+$ ]] || {
  printf 'project-reason: staged production copy size is unavailable\n' >&2
  exit 64
}
printf 'production-copy-bytes=%s\n' "$copy_bytes"

# The default acceptance oracle is the production-scale stateful regression. A testbox caller
# may provide an approved command for a real database copy; it receives only the staged path.
if [[ -n "${SUMMARY_PRODUCTION_VALIDATION_COMMAND:-}" ]]; then
  bash -lc "$SUMMARY_PRODUCTION_VALIDATION_COMMAND"
else
  cargo test --locked --manifest-path /workspace/Cargo.toml \
    --target-dir "${CARGO_TARGET_DIR:-/codex-scratch/target}" \
    summary_representative_scale_acceptance -- --nocapture
fi
printf 'summary-production-validation=passed\n'
