# 号池 usage-limit 429 硬失效与恢复门控补洞（#ppt8w）

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-24
- Last: 2026-03-24

## 背景 / 问题陈述

- 线上真实样本显示，部分上游 `429` 响应体使用 `"The usage limit has been reached"` 文案；现有 quota matcher 没有命中该短语，导致账号只进入短 cooldown，而不会被标记为 quota-exhausted hard stop。
- `oauth_codex` 维护同步在拿到 fresh usage snapshot 后，会无条件写 `sync_succeeded` 并恢复 `active`；当窗口仍处于 exhausted 状态时，这会把本应继续阻断的账号提前放回池中。
- `api_key_codex` 的同步路径不具备远端额度验证能力，但此前也会无条件写 `sync_succeeded`；一旦账号此前因 `402` / hard `429` 被踢出，同步会错误地把它重新拉回可用集合。
- resolver 在枚举候选时只看 `status=active`，因此被 hard-stop 标成 `error` 的 quota-exhausted 账号不会参与 “all accounts are rate limited” 汇总，可能退化成 generic unavailable。

## 目标 / 非目标

### Goals

- 将 `"The usage limit has been reached"` / `"usage limit reached"` 统一纳入 quota-exhausted `429` 分类，保持首次命中即 hard unavailable。
- 将 `oauth_codex` 的恢复改为“凭 fresh usage snapshot 恢复”：窗口仍 exhausted 时保持不可用并写明确动作；只有窗口恢复后才回到 `active`。
- 将 `api_key_codex` 的 hard-unavailable 恢复改为人工路径：同步不再自动恢复，只有显式账号修复动作才清掉 hard stop。
- 保持 quota-exhausted 账号即使已是 `error` 也能参与 pool-level `RateLimited` 汇总，继续对调用方返回既有终态 `429`。
- 复用现有 `pool_upstream_account_events` 与 `last_action_*` 摘要列，不新增 schema，只补 action/reason 语义。

### Non-goals

- 不改 generic forward proxy 的 `upstream_429_max_retries` 配置。
- 不改 `/v1/models` 聚合逻辑。
- 不新增独立 account events API、独立恢复按钮或新的数据库表/列。

## 功能与行为规格（Functional / Behavior Spec）

- `classify_pool_account_http_failure()` 继续作为 route/sync 共享入口；`429` 响应体命中 `"the usage limit has been reached"`、`"usage limit has been reached"`、`"usage limit reached"` 时，必须落到 `HardUnavailable + upstream_http_429_quota_exhausted + FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED`。
- `oauth_codex` 同步在成功拿到 snapshot 并持久化后：
  - 若账号此前是 quota-exhausted hard stop，且 fresh snapshot 仍 exhausted，则维持原 `status` / `last_error` / `last_route_failure_kind`，只更新 `last_synced_at`，并记录 `action=sync_recovery_blocked`、`reason_code=quota_still_exhausted`。
  - 若 fresh snapshot 已恢复，则清空 hard stop 状态并写 `sync_succeeded`。
- `api_key_codex` 同步在账号此前处于 hard-unavailable（auth / `402` / quota-exhausted `429`）时，不再写 `sync_succeeded`；仅更新 `last_synced_at`，并记录 `action=sync_recovery_blocked`、`reason_code=recovery_unconfirmed_manual_required`。
- `api_key_codex` 的显式人工恢复路径复用现有账号更新接口：当用户更新 API key 或显式重新启用账号时，若账号当前处于可人工恢复的 hard stop，则清空 `status/error/last_route_failure_*` 并返回 `active`。
- quota-exhausted 账号即使 `status=error`，resolver 仍必须把它计入 “rate limited candidate”；当池内只剩这类账号时，返回 `PoolAccountResolution::RateLimited`。

## 接口契约（Interfaces & Contracts）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- |
| `upstream_error_indicates_quota_exhausted()` | Rust helper | internal | Modify | route / sync classifier | 增补 usage-limit 短语 |
| `sync_recovery_blocked` | account action | internal | Add | backend / web / ops | 标识同步已执行但恢复仍被阻断 |
| `quota_still_exhausted` | reason code | internal | Add | backend / web / ops | OAuth fresh snapshot 仍 exhausted |
| `recovery_unconfirmed_manual_required` | reason code | internal | Add | backend / web / ops | API key sync 无法确认恢复，只能人工恢复 |

## 验收标准（Acceptance Criteria）

- Given 上游返回 `429: The usage limit has been reached`，When route classifier 处理该错误，Then 账号立刻写入 quota-exhausted hard stop，而不是 `route_cooldown_started`。
- Given `oauth_codex` 账号此前因 quota-exhausted `429` 被踢出，When 维护同步拿到仍 exhausted 的 fresh snapshot，Then 账号保持不可用，最新动作为 `sync_recovery_blocked / quota_still_exhausted`。
- Given 同一账号后续 fresh snapshot 已恢复，When 同步成功，Then 账号清空 hard stop 并重新回到 `active`。
- Given `api_key_codex` 账号此前因 hard-unavailable 被踢出，When 执行同步，Then 同步不会自动恢复该账号；只有账号更新/重新启用这类显式人工动作才能让它重新入池。
- Given 池内所有候选都处于 quota-exhausted hard stop，When resolver 计算 pool candidate，Then 结果仍为 `RateLimited` 而不是 generic unavailable。

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

## 方案概述（Approach, high-level）

- 在共享 `429` 分类器里补齐 usage-limit 文案，复用既有 hard-unavailable 行为，不新开分支体系。
- 将 sync 成功拆成“真正恢复成功”和“恢复仍被阻断”两种收口路径，前者清掉 hard stop，后者只记 action/timestamp，不改 last_error 上下文。
- 让 resolver 评估候选时包含 `enabled=1` 的全部 Codex 账号，再用现有分类逻辑区分 active / rate-limited / unavailable，确保 hard-stop quota 账号不会从汇总里消失。

## 后续补洞

- persisted usage snapshot exhausted 仍可能在 route 前被当成可选候选，且 stale success / stale sync 仍会覆盖更新更晚的 quota hard-stop；对应 follow-up 由 `docs/specs/v8y2p-prevent-routing-exhausted-accounts-race/SPEC.md` 承接。
