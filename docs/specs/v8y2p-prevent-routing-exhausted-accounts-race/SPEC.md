# 修复额度耗尽账号仍被路由与并发误恢复（#v8y2p）

## 状态

- Status: 进行中
- Created: 2026-03-25
- Last: 2026-03-25

## 背景 / 问题陈述

- 线上真实样本 `proxy-566-1774370134065` 显示，三个账号在请求发起前的最新 usage snapshot 都已经是 `100%` exhausted，但 resolver 仍会继续把它们当作可路由候选，导致单次请求连续撞上 3 次必败 `429 quota exhausted`。
- `sync_oauth_account()` 在持久化 fresh snapshot 后，仍使用函数入口时的旧 `row.last_route_failure_kind` 决定是否恢复；一旦 route 侧在 sync 期间写入新的 quota hard-stop，sync 仍可能把账号错误写回 `sync_succeeded`。
- `record_pool_route_success()` 当前无条件清理 `last_route_failure_*` / cooldown / sticky；如果较早开始的 success 在较晚的 quota hard-stop 之后才落库，会把更新更晚的 hard-stop 误清掉。

## 目标 / 非目标

### Goals

- 将“最新 persisted usage snapshot 已明确 exhausted”的账号在 resolver 阶段直接排除出可路由候选，不再靠真实上游 `429` 试出来。
- 当池内所有候选都只是本地已知 exhausted 时，直接汇总成现有 `PoolAccountResolution::RateLimited -> HTTP 429`，不再发起 1 到 3 次必败请求。
- 让 OAuth sync 在持久化 snapshot 后基于最新 row 做最终判定；fresh snapshot 仍 exhausted 时，继续阻断或主动隔离，而不是落成 `sync_succeeded`。
- 给 route success 增加 started-at 时序保护，防止旧 success 清掉更新更晚的 quota hard-stop 与 sticky。
- 复用既有 account action / reason / failure_kind 字段，不新增 schema。

### Non-goals

- 不改普通 `429 rate limit`、`5xx`、transport、timeout、first-chunk failure` 的既有重试语义。
- 不新增外部 API 字段或独立 events endpoint。
- 不重做 sticky tag policy 体系，只修 exhausted 候选排除与 hard-stop 恢复门控。

## 功能与行为规格（Functional / Behavior Spec）

- `AccountRoutingCandidateRow` 必须携带最新 sample 的 `credits_has_credits`、`credits_unlimited`、`credits_balance`；resolver 使用共享 exhaustion helper，语义与 `imported_snapshot_is_exhausted()` 保持一致。
- `resolve_pool_account_for_request()` 对 sticky route 与普通候选都必须先检查 “persisted snapshot exhausted / quota hard-stop / 429 cooldown”；snapshot exhausted 账号视为 `rate_limited candidate`，但绝不真正发上游请求。
- 当所有 routing candidates 都只是 “persisted snapshot exhausted” 或 quota hard-stop 时，resolver 返回 `PoolAccountResolution::RateLimited`；这种短路不应生成伪造的每账号 `http_failure` attempt。
- `sync_oauth_account()` 在 `persist_usage_snapshot()` 之后必须重新读取最新 row：
  - 若最新 row 已有 quota hard-stop，且 fresh snapshot 仍 exhausted，则记录 `sync_recovery_blocked / quota_still_exhausted`，保留原 `last_error` / `last_route_failure_kind`。
  - 若 fresh snapshot exhausted 但当前 row 还没有 quota hard-stop，则主动写 `sync_hard_unavailable / usage_snapshot_exhausted / upstream_usage_snapshot_quota_exhausted`，并把账号隔离出路由集合。
  - 只有 fresh snapshot 明确恢复时，sync 才允许 `sync_succeeded` 并清除可恢复的 hard-stop。
- `record_pool_route_success()` 增加成功请求 `started_at_utc` 参数；只有数据库里不存在“时间晚于该 started_at 的 route failure”时，才允许清理 `last_route_failure_*`、cooldown、sticky，并写 `route_recovered`。
- 若 success 早于最新 route failure，只允许这次请求本身完成；不得清状态、不得重新绑定 sticky、不得写 `route_recovered`。

## 接口契约（Interfaces & Contracts）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- |
| `AccountRoutingCandidateRow` | Rust row | internal | Modify | resolver | 增加 credits exhaustion 所需字段 |
| `upstream_usage_snapshot_is_exhausted_*` helper | Rust helper | internal | Add | resolver / summary / sync | 统一 persisted snapshot exhaustion 语义 |
| `record_pool_route_success(..., started_at_utc, ...)` | Rust fn | internal | Modify | pool live / capture | 防止 stale success 覆盖新 hard-stop |
| `sync_hard_unavailable` | account action | internal | Add | backend / web / ops | 同步主动隔离 exhausted 账号 |
| `usage_snapshot_exhausted` | reason code | internal | Add | backend / web / ops | fresh snapshot 已明确耗尽 |
| `upstream_usage_snapshot_quota_exhausted` | failure kind | internal | Add | backend / web / ops | 非真实 429、由 sync snapshot 主动隔离 |

## 验收标准（Acceptance Criteria）

- Given 账号最新 persisted snapshot 的 primary / secondary / credits 任一维度已 exhausted，When resolver 选择候选，Then 该账号不会真正发起上游请求。
- Given 池内所有候选都只是 persisted snapshot exhausted，When 请求进入 pool routing，Then 直接返回既有 pool-wide `429`，而不是继续切 1 到 3 个 doomed 账号。
- Given OAuth sync 在执行期间该账号先被 route 侧写入 quota hard-stop，When sync 收尾，Then 最新状态仍为 blocked，不能写成 `sync_succeeded`。
- Given fresh snapshot exhausted 但账号此前还没有真实 `429` hard-stop，When sync 完成，Then 账号被主动隔离并记录 `sync_hard_unavailable / usage_snapshot_exhausted`。
- Given 一个较早开始、较晚结束的 success 在更晚的 quota hard-stop 之后才落库，When `record_pool_route_success()` 执行，Then 它不会清空 `last_route_failure_*`、cooldown 或 sticky。

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

## 方案概述（Approach, high-level）

- 把 persisted snapshot exhaustion 从“排序信号”升级为“routing hard exclusion before send”，并复用现有 rate-limited terminal。
- 将 sync 的成功分支拆成 “recovered / blocked / proactive hard unavailable” 三类，全部由 fresh snapshot + 最新 row 决定。
- 给 route success 增加 started-at 时序保护，避免 older success 覆盖 newer hard-stop；summary / UI 继续复用现有 latest action 契约，只补新 action/reason/failure kind 文案。
