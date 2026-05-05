# raw 保真降本与历史维护追平 follow-up - Implementation

## Current State

- Canonical spec: `docs/specs/vw93e-raw-born-gzip-rollup-followup/SPEC.md`
- Implementation summary: 已实现，待 PR / CI / review-proof 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-13
- Last: 2026-04-17

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo check`
- `cargo test born_gzip -- --test-threads=1`
- `cargo test materialize_historical_rollups_marks_batches_and_prune_removes_files -- --test-threads=1`
- `cargo test fetch_usage_snapshot_skips_browser_user_agent_retry_for_upstream_rejected_402 -- --test-threads=1`
- `cargo test maintenance_plan_is_not_due_during_upstream_rejected_cooldown -- --test-threads=1`
- `cargo test record_pool_route_http_failure_marks_402_as_hard_error_and_records_reason -- --test-threads=1`
- `cargo test sync_triggered_402_summary_and_detail_export_as_upstream_rejected -- --test-threads=1`
- `scripts/shared-testbox-raw-smoke`

## 文档更新（Docs to Update）

- `README.md`
- `docs/deployment.md`
- `docs/specs/README.md`
- `docs/specs/vw93e-raw-born-gzip-rollup-followup/SPEC.md`
