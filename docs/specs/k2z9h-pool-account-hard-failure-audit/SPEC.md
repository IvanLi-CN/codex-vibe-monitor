# 号池硬失效账号淘汰与账号动作审计可视化（#k2z9h）

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-23
- Last: 2026-03-23

## 背景 / 问题陈述

- 号池在上游返回 `401`、`402`、`403` 与“额度耗尽型 `429`”时，仍缺少统一的硬失效收敛；部分路径会把账号继续保留在可用集合中，或只留下模糊 `last_error`。
- 账号列表与详情页当前没有结构化的“最近动作来源 / 原因 / HTTP 状态 / invoke id”视图，运营只能靠 `lastError` 猜测账号为什么被踢出或为什么恢复。
- 账号同步、路由调用、OAuth 导入、账号编辑这些动作缺少统一事件审计，导致数据库与 UI 都无法稳定回答“谁把账号标坏了、为什么、什么时候恢复过”。

## 目标 / 非目标

### Goals

- 将 `401`、`402`、`403` 与 quota / billing / plan / weekly-cap 语义的 `429` 统一归为“硬失效”，首次命中即踢出可用集合。
- 保持普通 `429 rate limit` 的“立即切号 + cooldown”语义，以及 `5xx` / transport / timeout / first-chunk failure 的现有重试语义。
- 为账号新增事件表与最新动作摘要列，统一记录 `action`、`source`、`reason_code`、`reason_message`、`http_status`、`failure_kind`、`invoke_id`、`sticky_key`。
- 在账号池列表与详情抽屉中直接展示最近动作来源 / 原因，并提供最近事件时间线。

### Non-goals

- 不更改 generic forward proxy 的 `upstream_429_max_retries`。
- 不重做 `/v1/models` 聚合路径。
- 不引入新的独立 account events API endpoint。

## 范围（Scope）

### In scope

- `src/main.rs`：pool 路由失败分类、HTTP `402` / quota `429` failover、attempt failure kind 对齐。
- `src/upstream_accounts/mod.rs`：账号硬失效 / soft failure 状态更新、事件持久化、resolver 识别 quota-exhausted 账号、detail API recent actions。
- `web/src/lib/api.ts`、`web/src/components/UpstreamAccountsTable.tsx`、`web/src/pages/account-pool/UpstreamAccounts.tsx`：最新动作摘要与 recent events UI。
- SQLite schema：`pool_upstream_account_events` 与 `pool_upstream_accounts.last_action_*`。

### Out of scope

- 删除账号后的历史恢复与独立审计页。
- 非账号池页面的大范围 observability 改版。

## 功能与行为规格（Functional / Behavior Spec）

- `401` / `402` / `403` 与 quota-exhausted `429` 命中时，账号立刻标记为 `error` 或 `needs_reauth`，并从后续 pool routing 候选中移除。
- 普通 `429` 继续只打 cooldown，但 recent action / event 必须明确记录来源为 `call`、原因是 `upstream_http_429_rate_limit`。
- 同步链路中，硬失效错误把账号标为不可用；普通 `429`、`5xx`、transport 只更新 `last_error` 与 recent action，不应永久踢号。
- 列表页 `Sync / Call` 列新增“最近动作摘要”；详情页展示 latest action 卡片与 recent events 列表。

## 接口契约（Interfaces & Contracts）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- |
| `pool_upstream_account_events` | SQLite table | internal | Add | backend / ops / web | 账号动作审计时间线 |
| `pool_upstream_accounts.last_action_*` | SQLite columns | internal | Add | backend / web | 列表页最新动作摘要 |
| `UpstreamAccountSummary` | Rust + TS type | internal | Modify | web | 新增 latest action 摘要字段 |
| `UpstreamAccountDetail.recentActions` | Rust + TS type | internal | Modify | web | 固定返回最近 20 条动作 |

## 验收标准（Acceptance Criteria）

- Given 上游首次返回 `402`，When pool 请求继续 failover，Then 当前账号立刻标为 `error`，并切到下一个账号。
- Given 上游返回带 quota / billing 语义的 `429`，When 账号被标记，Then 该账号不再作为 active candidate 参与后续路由，同时 UI 可直接看到来源、原因与最近事件。
- Given 手动同步命中普通 `429` 或 `5xx`，When 同步结束，Then 账号保留可用状态，但 recent action 明确记录同步失败来源与原因。
- Given 用户打开账号详情抽屉，When 该账号已有事件，Then 页面展示 latest action 卡片与 recent events 列表；若没有事件，则展示清晰空态。

## 非功能性验收 / 质量门槛（Quality Gates）

- `cargo fmt --check`
- `cargo check`
- `cargo test pool_route_ -- --test-threads=1`
- `cd web && bun run test -- src/components/UpstreamAccountsTable.test.tsx src/pages/account-pool/UpstreamAccounts.test.tsx`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/h4p2x-pool-upstream-429-immediate-failover/SPEC.md`

## 方案概述（Approach, high-level）

- 用统一的 HTTP failure classifier 收敛 route / sync 两条链路，区分 `HardUnavailable / RateLimited / Retryable`。
- 新增账号动作事件表与主表 latest action 摘要列，让数据库和 UI 都直接读结构化动作，而不是继续解析 `last_error`。
- 将 quota-exhausted `429` 视为硬失效，但 resolver 仍需识别其 rate-limited 语义，避免后续请求退化成 generic unavailable。
