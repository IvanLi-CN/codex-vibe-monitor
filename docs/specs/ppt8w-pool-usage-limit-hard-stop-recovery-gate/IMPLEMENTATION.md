# 号池 usage-limit 429 硬失效与恢复门控补洞 - Implementation

## Current State

- Canonical spec: `docs/specs/ppt8w-pool-usage-limit-hard-stop-recovery-gate/SPEC.md`
- Implementation summary: 已完成（5/5）

## Migrated Implementation Notes

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-24
- Last: 2026-03-24

## 非功能性验收 / 质量门槛（Quality Gates）

- `cargo fmt --check`
- `cargo check`
- `cargo test usage_limit_reached -- --test-threads=1`
- `cargo test oauth_sync_keeps_quota_exhausted_accounts_blocked_until_snapshot_recovers -- --test-threads=1`
- `cargo test oauth_sync_reactivates_quota_exhausted_account_once_snapshot_recovers -- --test-threads=1`
- `cargo test sync_api_key_account_keeps_hard_unavailable_accounts_blocked -- --test-threads=1`
- `cargo test updating_api_key_reactivates_manually_recoverable_account -- --test-threads=1`
- `cargo test resolver_keeps_quota_exhausted_accounts_in_rate_limited_terminal_state_after_sync_block -- --test-threads=1`
- `cd web && bun run test -- src/pages/account-pool/UpstreamAccounts.test.tsx`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/k2z9h-pool-account-hard-failure-audit/SPEC.md`
