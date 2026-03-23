# 号池上游 429 立即切号与终态 429（#h4p2x）

## 状态

- Status: 已完成
- Created: 2026-03-23
- Last: 2026-03-23

## 背景 / 问题陈述

- 号池上游账号命中 `429` 时，现有实现会把它与 `5xx` 一样视为“同账号可重试”错误，导致同一账号在已经触发上游限额或周限后仍被重复命中。
- 当池内没有其它可切账号，或全部候选都处于既有 `429` cooldown 时，对调用方返回的仍可能是泛化 `502`，无法准确表达“整个号池当前被上游限流”。
- 这种行为既浪费 failover 预算，也会把真实的 rate-limit 故障隐藏成 generic no-candidate / upstream failure。

## 目标 / 非目标

### Goals

- 让 pool 路由在任一账号第一次收到上游 `429` 后立刻切到下一个不同账号，不再对同一账号做第 2 次尝试。
- 当没有可切账号，或起手发现所有候选都被既有 `429` cooldown 挡住时，稳定向调用方返回 `HTTP 429` 与清晰错误文案。
- 保持 `5xx`、transport、handshake timeout、first-chunk failure 的同账号重试语义不变。
- 保留“最多 3 个不同账号”的 failover 上限，但如果 3 个不同账号都因 `429` 失败，外部终态仍必须是 `429`。
- 让账号解析与可观测层能明确区分“generic no candidate”和“all candidates are rate-limited upstream”。

### Non-goals

- 不改 generic forward proxy 的 `upstream_429_max_retries` 行为。
- 不改 `/v1/models` 聚合路径。
- 不调整 3 个不同账号的 failover 上限。
- 不改变非 `429` 上游错误的 cooldown 基线与重试策略。

## 范围（Scope）

### In scope

- `src/main.rs`：pool failover 主循环、live replay 恢复链、rate-limited 终态映射与 attempt summary。
- `src/upstream_accounts/mod.rs`：`last_route_failure_kind` 持久化、账号 resolver 的 rate-limited exhaustion 识别、成功/恢复路径的清理逻辑。
- `src/tests/mod.rs`：新增与更新后端回归，覆盖 capture/live/cooldown/budget exhaustion 路径。
- `docs/specs/README.md` 与相关 spec：同步新的 429 failover 约束。

### Out of scope

- generic forward proxy 上游 429 自动重试策略。
- pool 账号排序、tag 路由、sticky key 本身的选路规则。
- pool attempts API / UI 交互模型的大范围重构。

## 需求（Requirements）

### MUST

- 任一 pool 账号第一次收到上游 `429` 后，必须立即记录本次 `http_failure`、写入账号 cooldown，并切到下一个不同账号。
- `429` 不得继续消耗同账号 retry 预算；对应 attempt 的 `same_account_retry_index` 固定为 `1`。
- `500`、transport、handshake timeout、first-chunk failure 仍按现有语义允许同账号重试，并继续递增 `same_account_retry_index`。
- 若已无其它可切账号，或 resolver 起手发现全部候选都处于 `429` cooldown，调用方必须收到 `HTTP 429`，body 继续使用 `{ "error": "<message>" }` 壳。
- `pool_upstream_accounts` 必须持久化最近一次路由失败类型，用于区分“rate-limited cooldown”与 generic cooldown/no-candidate。
- 当 3 个不同账号都因 `429` 失败而耗尽 failover 预算时，attempts 中仍保留 `budget_exhausted_final / max_distinct_accounts_exhausted`，但外部响应状态码必须是 `429`。
- live replay 恢复链在首个账号收到 `429` 后，后续 replay 不能再次优先同一个账号，必须带着已排除账号继续 failover。

### SHOULD

- 成功、sync recovery、人工恢复路径应清理 `last_route_failure_kind`，避免陈旧 `429` 标记污染后续选路。
- 当最终 429 来自“全池已 rate-limited”而不是 generic no-candidate 时，invocation summary 应记录专用 terminal reason，便于排障。

## 功能与行为规格（Functional / Behavior Spec）

### Core flows

- pool 请求进入 failover 主循环后，若当前账号返回 `429`，本轮立即终止对该账号的同账号重试，并 `continue 'account_loop` 切到下一个 distinct account。
- 若 `429` 发生在 live request 的首发阶段，但请求体随后可 replay，恢复逻辑必须从“排除首个 429 账号”的状态继续，而不是重新把该账号放回 preferred path。
- resolver 在 sticky 账号或普通候选均不可选时，若原因是有效期内的 `429` cooldown，则返回 `RateLimited` 终态，而不是 generic `NoCandidate`。
- failover distinct-account 预算达到上限时，若最近一次错误属于 rate-limited，则最终错误状态改写为 `429`，同时保留 budget exhaustion attempt 行。

### Edge cases / errors

- 单账号池首发即 `429` 时，调用方应直接收到 `HTTP 429`，不再等待同账号重试耗尽。
- 所有候选都因既有 `429` cooldown 被挡住时，即使没有真正发出新的上游请求，也要返回清晰的 pool rate-limited 终态。
- `401/403` 的认证/权限错误仍按现有规则落到账号状态更新，不纳入“rate-limited exhaustion”识别。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- | --- |
| `PoolAccountResolution::RateLimited` | Rust enum | internal | Modify | backend | pool routing | 专门表示“可用候选都被 429 cooldown 挡住” |
| `pool_upstream_accounts.last_route_failure_kind` | SQLite column | internal | Add | backend | account resolver / ops | 避免依赖 `last_error` 文本解析 |
| pool `/v1/*` rate-limited terminal | HTTP behavior | external | Modify | backend | pool callers | 状态码统一为 `429`，JSON 壳不变 |
| `pool_upstream_request_attempts.same_account_retry_index` | SQLite behavior | internal | Modify | backend | ops / attempts API | `429` 路径固定为 `1` |

## 验收标准（Acceptance Criteria）

- Given 一个 pool 请求首个账号返回 `429` 且池内还有第二个账号，When 请求完成，Then 下一次真正发往上游的请求必须落到不同账号，且首个账号只被命中一次。
- Given 只有一个账号，When 它第一次返回 `429`，Then 调用方收到 `HTTP 429` 与清晰错误文案，而不是 `502`。
- Given 全部账号都已处于有效 `429` cooldown，When 新请求到达，Then resolver 直接返回 rate-limited 终态，且无需额外上游尝试。
- Given 3 个不同账号连续返回 `429`，When failover 预算耗尽，Then外部状态码为 `429`，同时 attempts 中保留 3 条 `http_failure` 与 1 条 `budget_exhausted_final`。
- Given 上游返回 `500` 或 transport / handshake / first-chunk failure，When 同账号仍有预算，Then 行为保持为同账号重试，不被这次修复改变。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test pool_route_ -- --test-threads=1`
- `cargo check`
- `cargo fmt --check`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/m2f8k-pool-upstream-attempt-observability/SPEC.md`

## 方案概述（Approach, high-level）

- 在 pool failover 主循环中把 `429` 从“同账号 retryable”分支剥离，改成“立即 cooldown + 切号”。
- 在账号状态层新增结构化 `last_route_failure_kind` 字段，用于 resolver 准确识别 rate-limited cooldown exhaustion。
- 在终态映射层复用现有 attempts / summary 结构，仅对状态码、message 与 terminal reason 做 rate-limited-aware 收敛。
