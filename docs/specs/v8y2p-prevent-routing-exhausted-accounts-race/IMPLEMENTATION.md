# 修复额度耗尽账号仍被路由与并发误恢复 - Implementation

## Current State

- Canonical spec: `docs/specs/v8y2p-prevent-routing-exhausted-accounts-race/SPEC.md`
- Implementation summary: 已完成（PR #227）

## Migrated Implementation Notes

## 状态

- Status: 已完成（PR #227）
- Created: 2026-03-25
- Last: 2026-03-25

## 非功能性验收 / 质量门槛（Quality Gates）

- `cargo fmt --check`
- `cargo check`
- `cargo test resolver_short_circuits_when_only_persisted_snapshot_exhausted_accounts_remain -- --test-threads=1`
- `cargo test resolver_skips_persisted_snapshot_exhausted_account_before_routing -- --test-threads=1`
- `cargo test oauth_sync_proactively_quarantines_snapshot_exhausted_account_without_prior_route_failure -- --test-threads=1`
- `cargo test record_pool_route_success_does_not_clear_newer_route_failure_state -- --test-threads=1`
- `cargo test oauth_sync_ignores_stale_input_row_after_newer_quota_hard_stop -- --test-threads=1`
- `cd web && bun run test -- src/components/UpstreamAccountsTable.test.tsx src/pages/account-pool/UpstreamAccounts.test.tsx`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/ppt8w-pool-usage-limit-hard-stop-recovery-gate/SPEC.md`
