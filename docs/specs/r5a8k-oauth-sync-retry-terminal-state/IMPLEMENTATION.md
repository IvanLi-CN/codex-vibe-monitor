# OAuth 同步 refresh 后 retry 失败残留 `syncing` 修复 - Implementation

## Current State

- Canonical spec: `docs/specs/r5a8k-oauth-sync-retry-terminal-state/SPEC.md`
- Implementation summary: 已实现，待 PR / CI 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI 收敛
- Created: 2026-03-25
- Last: 2026-03-25

## Validation

- `cargo test oauth_sync_retry_after_refresh_settles_to_needs_reauth_without_stale_syncing -- --test-threads=1`
- `cargo test oauth_sync_retry_after_refresh_records_non_auth_terminal_failure_without_stale_syncing -- --test-threads=1`
- `cargo test quota_exhausted_oauth_summary_and_detail_export_as_rate_limited -- --test-threads=1`
- `cd web && bun run test -- src/components/UpstreamAccountsPage.list.stories.tsx`
- Storybook mock 场景截图 + 浏览器 smoke
