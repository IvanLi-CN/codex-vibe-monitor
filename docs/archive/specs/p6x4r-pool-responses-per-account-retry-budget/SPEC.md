# 号池 `/v1/responses*` 临时失败改为“每个当前账号先重试再切号”（#p6x4r）

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-15
- Last: 2026-04-15
- Note: fast-track / pool `/v1/responses*` temporary failure family only / no HTTP API or SQLite schema change

## 背景 / 问题陈述

- 现有 pool `/v1/responses*` 对 same-account retry budget 有一条额外收口：只有首个真实 distinct account 能拿到完整的同账号临时失败重试预算，后续切到的新账号会被硬降成单次尝试。
- 这会把第二个或第三个账号上的可恢复临时失败放大成过早切号，尤其在 retryable `5xx`、transport timeout、handshake timeout、first-chunk failure、pre-forward `response.failed` 等场景下，`poolAttemptCount` 会显得偏激进，而实际成功率偏低。
- 线上调用 `proxy-5188-1776160635332` 暴露的需求不是改写 `429` / quota / auth 语义，而是让 `/v1/responses*` 的当前账号在已经真实发起 upstream dispatch 之后，也能先吃完现有 temporary-failure retry budget，再决定是否切换到下一个账号。

## 目标 / 非目标

### Goals

- 让 pool `/v1/responses*` 的 same-account retry budget 适用于每个真实发起过 upstream dispatch 的当前账号，而不是只对首个 distinct account 生效。
- 保持 `poolAttemptCount`、`poolDistinctAccountCount`、`same_account_retry_index` 的现有可解释性，不新增响应字段或持久化 shape。
- 保持 `/v1/responses*` 的 `300s` total-timeout guardrail、timeout-shaped failover、preflight 不偷吃 distinct-account 预算等既有护栏。
- 明确不改变 plain `429`、quota-exhausted `429`、`401`、`402`、`403` 的既有语义。

### Non-goals

- 不修改 generic reverse proxy、`/v1/models`、非 pool 路径或 UI 可视化。
- 不新增数据库列、HTTP API 字段、设置项或 group metadata。
- 不把分组级 `429` retry override 与这次 temporary-failure retry 扩展混在一起。

## 范围（Scope）

### In scope

- `src/proxy/stream_gate.rs`：`pool_same_account_attempt_budget(...)` 对 `/v1/responses*` 的预算策略。
- `src/proxy/failover.rs`：temporary-failure family 的 per-account same-account retry 判定与 retry 链路。
- `src/tests/slices/proxy_retry_headers_and_model_settings.rs`、`src/tests/slices/invocation_failure_recovery_{a,b}.rs`：budget helper、follow-up account retry、plain `429` non-regression、group `429` override、responses total-timeout 回归。
- `docs/specs/README.md` 与当前 spec 的状态同步。

### Out of scope

- `h4p2x` 已冻结的 plain `429` 立即切号语义。
- `k8a4r` 已冻结的 group-level `upstream429RetryEnabled / upstream429MaxRetries` 语义。
- `t9m3p` 已冻结的 `/v1/responses*` `300s` total timeout / `pool_total_timeout_exhausted` 终态。
- `gkser` 已冻结的 preflight 不计入真实 distinct-account dispatch 预算语义。

## 需求（Requirements）

### MUST

- `pool_same_account_attempt_budget(...)` 在 `/v1/responses*` 上不得再把 follow-up distinct account 的预算硬降为 `1`；每个当前账号都应继承初始 same-account retry budget（至少 `1`）。
- same-account retry 只能在“当前账号已经真实发起 upstream dispatch”且“当前失败属于 temporary-failure family”时继续消耗预算。
- `/v1/responses*` 的 temporary-failure family 至少包括：transport failure、send/handshake timeout、retryable `5xx`、first-chunk failure、pre-forward `response.failed`；其中 timeout-shaped failures 仍必须继续服从既有 timeout-route-failover 护栏。
- plain `429` 仍必须立即切号；quota-exhausted `429` / `401` / `402` / `403` 仍维持既有 hard-stop / cooldown 语义。
- `/v1/responses*` 的 `300s` total-timeout budget 必须继续跨所有账号与同账号重试累计；预算耗尽时终态仍是 `pool_total_timeout_exhausted`。
- preflight 失败不得偷吃 `poolDistinctAccountCount`，也不得提前消耗 follow-up account 的 same-account retry budget。

### SHOULD

- follow-up account 在临时失败后继续 same-account retry 时，应继续复用现有 observability 字段，而不是引入新的 attempt 分类。
- 回归测试应直接验证第二个账号 timeout/`5xx` 后还能继续 retry，以及 plain `429` / group `429` override 不回归。

## 功能与行为规格（Functional / Behavior Spec）

### Core flows

- `/v1/responses` 与 `/v1/responses/compact` 在 pool routing 下共享同一条“per-account temporary retry budget + total timeout”策略。
- 当首个账号临时失败并切到第二个账号后，第二个账号首次真实 dispatch 若再次命中 temporary failure，应继续走当前 same-account retry 分支，直到本账号预算耗尽，再切换第三个账号。
- `same_account_retry_index` 继续按实际同账号尝试次数递增；切换到新账号时重新从 `1` 开始。
- `poolDistinctAccountCount` 只统计真实 dispatch 过的账号；总 attempt 统计仍只反映真实 attempt 行，不把 preflight-only 失败误算成 distinct-account 消耗。

### Non-regression boundaries

- `h4p2x`: plain `429` 第一次命中时仍立即切换到下一个账号；若全池因 plain `429` 耗尽，终态继续保持既有 `429` 语义。
- `k8a4r`: group-level upstream `429` retry 仍只作用于 `429`，不被这次 `/v1/responses*` temporary failure 扩展覆盖或重置。
- `t9m3p`: timeout-shaped failures 仍按既有规则立即切入 timeout-route failover，并继续受同一个 `300s` total timeout budget 裁剪。
- `gkser`: preflight 未真实 dispatch 的账号不进入 distinct-account 与 same-account budget 统计。

## 验收标准（Acceptance Criteria）

- Given 首账号临时失败并切到第二账号，When 第二账号返回 retryable `5xx` / first-chunk failure / pre-forward `response.failed`，Then 第二账号必须还能继续 same-account retry，而不是立刻切第三账号。
- Given follow-up account 连续临时失败直到预算耗尽，When 还有下一 distinct account 可用，Then 代理在本账号预算耗尽后再切到下一账号，且 attempt rows 中 `same_account_retry_index` 对应该账号连续递增。
- Given plain `429`，When 请求发生在 pool `/v1/responses*`，Then 行为仍是立即切号，不受这次 per-account retry 扩展影响。
- Given group-level `upstream429RetryEnabled=true`，When 同账号先经历 retryable `5xx` 再命中 `429`，Then `5xx` 仍走 `/v1/responses*` temporary failure budget，`429` 仍走分组级 override，不混淆预算来源。
- Given `/v1/responses*` 命中 timeout-shaped failure，When timeout-route failover 生效，Then 仍必须立即切到其它 route key 或直接返回既有 timeout terminal，而不是先耗尽当前 route 的 same-account retry budget。
- Given `/v1/responses*` 多账号/多次重试累计触达 `300s`，When 总预算耗尽，Then 终态仍为 `pool_total_timeout_exhausted`，且不会因新的 per-account retry 规则继续拉长。

## 验证

- `cargo fmt --check`
- `cargo check --tests`
- `cargo test pool_same_account_attempt_budget_keeps_follow_up_accounts_retryable_for_responses_family -- --nocapture`
- `cargo test pool_route_responses_compact_retries_follow_up_accounts_before_switching -- --nocapture`
- `cargo test capture_target_pool_route_stops_after_three_distinct_accounts -- --nocapture`
- `cargo test capture_target_pool_route_timeout_switches_to_alternate_upstream_route -- --nocapture`
- `cargo test capture_target_pool_route_timeout_returns_no_alternate_when_only_same_route_remains -- --nocapture`
- `cargo test capture_target_pool_route_timeout_surfaces_blocked_policy_terminal -- --nocapture`
- `cargo test capture_target_pool_route_timeout_ignores_broken_same_route_groups -- --nocapture`
- `cargo test pool_route_existing_sticky_owner_preserves_last_failure_after_cutout_alternate_fails -- --nocapture`
- `cargo test pool_route_existing_sticky_owner_preserves_last_failure_after_distinct_budget_exhausts -- --nocapture`
- `cargo test pool_route_does_not_use_pool_wide_429_message_when_budget_exhaustion_is_mixed -- --nocapture`
- `cargo test pool_route_group_upstream_429_retry_keeps_separate_budget_from_server_errors -- --nocapture`
- `cargo test pool_route_live_request_switches_accounts_immediately_after_upstream_429 -- --nocapture`
- `cargo test pool_openai_v1_responses_failover_reapplies_account_fast_mode_from_original_body -- --nocapture`
- `cargo test pool_openai_v1_responses_compact_total_timeout_caps_same_account_retry_before_first_byte -- --nocapture`
- `cargo test pool_route_compact_502_returns_cvm_id_and_attempt_observations -- --nocapture`

## 参考

- `docs/specs/h4p2x-pool-upstream-429-immediate-failover/SPEC.md`
- `docs/specs/k8a4r-pool-group-upstream-429-retry/SPEC.md`
- `docs/specs/t9m3p-pool-responses-timeout-guardrails/SPEC.md`
- `docs/specs/gkser-oauth-responses-large-body-passthrough/SPEC.md`
