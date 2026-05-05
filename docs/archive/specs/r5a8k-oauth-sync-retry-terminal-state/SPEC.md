# OAuth 同步 refresh 后 retry 失败残留 `syncing` 修复（#r5a8k）

## 状态

- Status: 已实现，待 PR / CI 收敛
- Created: 2026-03-25
- Last: 2026-03-25

## Summary

- 修复 `sync_oauth_account()` 在 `401/403 -> refresh token -> 第二次 usage snapshot 仍失败` 时直接把错误冒泡给外层 logger、却没有写回终态的问题。
- 继续沿用现有 `/api/pool/upstream-accounts` summary/detail 契约，不新增字段、不改 schema；只保证后端把账号从 `status=syncing` 收口到已有终态。
- refresh 成功后的 retry 失败统一复用既有 failure classifier：
  - 显式 reauth / invalidated 信号收口到 `needs_reauth`
  - 其余 `401/403` 收口到既有 hard failure
  - 非 auth 失败继续落到既有 retryable / upstream unavailable 语义
- 增补回归测试与 mock-only Storybook 场景，证明列表行和详情头部不再显示 stale `同步中`。

## Scope

- 后端：
  - `src/upstream_accounts/mod.rs`
  - `sync_oauth_account()` retry-after-refresh 分支
  - OAuth retry failure 回归测试夹具
- UI evidence：
  - `web/src/components/UpstreamAccountsPage.story-helpers.tsx`
  - `web/src/components/UpstreamAccountsPage.list.stories.tsx`
- 文档：
  - `docs/specs/README.md`

## Non-goals

- 不做 101 线上数据库手工修复。
- 不新增前端 heuristic 去“猜” stale syncing。
- 不改 API key sync 恢复策略。
- 不新增账号状态枚举、数据库列或 API 字段。

## Acceptance

- Given OAuth 账号首次 usage snapshot 命中 `401/403` 且 refresh token 成功，When 第二次 usage snapshot 仍失败，Then 数据库行不得继续保留 `status=syncing`。
- Given retry 失败包含显式 invalidated / reauth 信号，When 同步结束，Then 账号必须写入 `sync_failed` 最新动作并导出 `syncState=idle` + `displayStatus=needs_reauth`。
- Given retry 失败为非 auth 上游错误，When 同步结束，Then 账号必须写入 `sync_failed` 最新动作并导出 `syncState=idle`，同时保留既有 upstream unavailable / retryable 派生语义。
- Given 现网 canary `1408 / 1409 / 1411`，When 修复版本部署后下一轮 maintenance 执行，Then 它们的 `last_action_at` 必须前进，页面不再显示“同步中”。

## Validation

- `cargo test oauth_sync_retry_after_refresh_settles_to_needs_reauth_without_stale_syncing -- --test-threads=1`
- `cargo test oauth_sync_retry_after_refresh_records_non_auth_terminal_failure_without_stale_syncing -- --test-threads=1`
- `cargo test quota_exhausted_oauth_summary_and_detail_export_as_rate_limited -- --test-threads=1`
- `cd web && bun run test -- src/components/UpstreamAccountsPage.list.stories.tsx`
- Storybook mock 场景截图 + 浏览器 smoke
