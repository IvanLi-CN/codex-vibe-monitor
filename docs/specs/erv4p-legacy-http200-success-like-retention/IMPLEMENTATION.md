# 修复 legacy `http_200` success-like retention 漏清理 - Implementation

## Current State

- Canonical spec: `docs/specs/erv4p-legacy-http200-success-like-retention/SPEC.md`
- Implementation summary: 已实现，待 PR / CI 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI 收敛

## 验证

- `cargo test retention_prunes_old_success_invocation_details_and_sweeps_orphans -- --test-threads=1`
- `cargo test retention_prunes_old_legacy_http_200_success_like_invocation_details -- --test-threads=1`
- `cargo test retention_does_not_prune_legacy_http_200_rows_with_error_message -- --test-threads=1`
- `cargo test retention_compresses_cold_raw_payloads_and_updates_paths -- --test-threads=1`
- `cargo check`
