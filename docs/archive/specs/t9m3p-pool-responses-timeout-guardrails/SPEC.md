# 号池 `/v1/responses*` 超时护栏收口为 `180s / 300s`（#t9m3p）

## 状态

- Status: 已实现
- Created: 2026-03-23
- Last: 2026-03-25

## 背景 / 问题陈述

- 101 线上 `routeMode=pool` 的 `/v1/responses` 长尾请求已经不只是“长对话本身慢”，而是内部 failover 链路会把多次超时串起来，导致调用方体感接近“卡住”。
- 现有号池对 `/v1/responses` 的单次首包预算默认是 `120s`，发生 timeout-shaped failure 后会继续切换账号 / route；当连续两到三跳都不理想时，总耗时会膨胀到 `5-12` 分钟。
- 线上样本已经验证问题链路来自 pool 账号本身，而不是 forward proxy；因此这次需要先给 pool `/v1/responses*` 加明确的时间护栏，而不是继续放任内部重试无限拉长。
- 本轮目标不是重做 sticky / prompt-cache affinity，而是先把“单跳稍微放宽、整链路必须收敛、预算耗尽就明确报错”的行为固定下来。

## 目标 / 非目标

### Goals

- 将 pool `/v1/responses` 的单次首包默认预算从 `120s` 调整为 `180s`。
- 为 pool `/v1/responses*` 引入统一的整条 failover 链路总预算 `300s`，超过后立刻停止内部重试。
- 当总预算耗尽时，统一返回 `HTTP 504`，并把 `failureKind` / `poolAttemptTerminalReason` 记为 `pool_total_timeout_exhausted`。
- 保持现有 pool 尝试计数与 distinct-account 统计语义可用，让线上观察面能直接区分“无可用路由”与“总预算耗尽”。

### Non-goals

- 不修改 sticky route 的删除时机。
- 不重做 prompt-cache affinity、账号排序或 failover 策略本身。
- 不改变非 `/v1/responses*` 的 pool 路径、后台任务或 forward-proxy 路径默认超时语义。

## 范围（Scope）

### In scope

- `src/main.rs` 中 pool `/v1/responses*` 的 attempt timeout、total timeout、终态状态码与失败原因。
- `src/tests/mod.rs` 中覆盖 `/v1/responses` 与 `/v1/responses/compact` 的 `180s / 300s / 504` 回归。
- `docs/specs/README.md` 与当前 spec 的状态同步。

### Out of scope

- sticky cut-out / affinity-first 策略修复。
- forward proxy 节点选择、账号候选排序和 backfill SQL 调优。
- 新增 API 字段或数据库 schema 变更。

## 需求（Requirements）

### MUST

- pool `/v1/responses` 的单次首包默认预算必须是 `180s`。
- pool `/v1/responses/compact` 必须继续使用 compact 专用单次预算 `180s`。
- pool `/v1/responses*` 必须共享单一的整链路总预算 `300s`，该预算跨账号切换、route failover 与 replay 重试累计消耗。
- 每次 `send`、首 chunk 等待、错误体读取都必须使用 `min(单次预算, 剩余总预算)`。
- 一旦总预算耗尽，后续不得继续尝试下一账号 / route，必须立即返回 `HTTP 504`。
- 总预算耗尽时，`failureKind` 与 `poolAttemptTerminalReason` 必须持久化为 `pool_total_timeout_exhausted`。

### SHOULD

- `/v1/responses/compact` 应与 `/v1/responses` 共用同一条 timeout-failover 护栏，而不是继续保留单独的“可以无限拖长”的语义。
- 回归测试应直接验证“第二跳只拿剩余预算”和“第三跳根本不会被尝试”。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- `POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS` 默认值改为 `180`。
- 新增 `POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS`，默认值 `300`。
- `/v1/responses` 与 `/v1/responses/compact` 共用 timeout-failover policy：
  - 单跳预算分别取普通 responses attempt timeout 或 compact handshake timeout；
  - 总预算统一从首次 pool upstream 尝试开始计时；
  - 每次实际超时 budget 取 `min(单跳预算, 剩余总预算)`。
- 若某一跳已经把剩余总预算耗尽，则最终错误统一提升为 `HTTP 504 + pool_total_timeout_exhausted`，而不是再落回 `502 no_alternate_upstream_after_timeout`。

### Edge cases / errors

- 若单跳在 headers 前超时，仍记为该跳的 transport failure；只有进入下一轮选择前发现总预算已尽时，才生成最终 `504` 终态。
- 若某一跳快速返回 `5xx` 并触发同账号重试，等待和重试也必须消耗同一条总预算。
- 若 timeout route failover 已把“当前 route key”的健康候选全部排除，而池内同时存在已知 exhausted / 429 hard-stop 候选，则终态仍必须保持 `502 + pool_no_alternate_upstream_after_timeout`，不得错误降解成 pool-wide `429`。
- 非 `/v1/responses*` 的 pool 请求继续沿用现有超时语义，不参与这条 `300s` 护栏。

## 验收标准（Acceptance Criteria）

- Given 第一跳在约 `180s` 前拿不到首 chunk，When 第二跳开始时整链路只剩约 `120s`，Then 第二跳只能使用剩余预算，整条链路必须在 `<=300s` 内结束。
- Given 第一跳超时、第二跳很快成功，When 请求完成，Then 调用方收到成功响应，且记录中的 `poolAttemptCount=2`、`poolDistinctAccountCount=2`。
- Given 第一跳和第二跳都在首 chunk 前耗尽预算，When 总预算达到 `300s`，Then 返回 `HTTP 504`，并且第三跳不会再被尝试。
- Given `/v1/responses/compact` 命中同样的慢首 chunk 路径，When 总预算达到 `300s`，Then 也必须返回 `HTTP 504 + pool_total_timeout_exhausted`。
- Given 第一跳 timeout 后只剩同 route key 的健康账号，以及其它已知 exhausted 的候选，When resolver 继续 failover，Then 终态是 `no alternate upstream route is available after timeout`，而不是 `pool_all_accounts_rate_limited`。
- Given 非 `/v1/responses*` 的 pool 请求，When 本次修复完成，Then 既有超时语义保持不变。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test capture_target_pool_route_total_timeout`
- `cargo test pool_openai_v1_responses_compact_total_timeout_exhausts_before_third_route`
- `cargo test app_config_from_sources_uses_proxy_timeout_defaults`
- `cargo test app_config_from_sources_reads_proxy_timeout_envs`
- `cargo test app_config_from_sources_rejects_zero_pool_upstream_responses_total_timeout`

### Quality checks

- `cargo fmt`
- `cargo test`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/t9m3p-pool-responses-timeout-guardrails/SPEC.md`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 将 `/v1/responses` 单跳默认预算调整到 `180s`，并新增 `POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS=300`。
- [x] M2: 把总预算贯穿到 pool `/v1/responses*` 的 send / first-chunk / error-body 路径，并新增 `pool_total_timeout_exhausted`。
- [x] M3: 增加 `/v1/responses` 与 `/v1/responses/compact` 的总预算回归，以及配置解析回归。
- [ ] M4: 完成 fast-track 交付（提交、push、PR、checks、review-loop、merge、cleanup）。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：如果线上问题的主因最终是 sticky/affinity 被 timeout cut-out 放大，那么这次修复会把“无限拖长”收敛成明确 `504`，但不会自动提升成功率。
- 风险：总预算开始生效后，部分原本会在 `5-12` 分钟后失败的请求，会更早暴露为 `504`；这是预期收敛，不是回归。
- 风险：timeout route failover 目前仍按 `upstream_route_key` 排除同 route 候选；当池内大多数健康账号共享同一 route key 时，超时后仍可能很快落到 `no alternate upstream route`，但这必须是明确的 `502`，不是伪造的 pool-wide `429`。
- 假设：调用方宁可在 `300s` 内拿到明确失败，也不接受持续卡住无界等待；这与当前产品约束一致。

## 变更记录（Change log）

- 2026-03-25: 修复 timeout route failover 与 exhausted 候选并存时的终态误分类；被 `route_key` 排除的健康候选现在会正确收敛到 `pool_no_alternate_upstream_after_timeout`，不再错误返回 `pool_all_accounts_rate_limited`。

- 2026-03-23: 创建 spec，冻结 `180s / 300s / 504` 的 timeout guardrails 边界、验收与验证要求。
- 2026-03-23: 完成本地实现与 targeted regression；待 fast-track 交付收口。
- 2026-03-23: 补齐总预算从首次 upstream 尝试起算、same-account retry 的 distinct-account 统计保持稳定，以及 OAuth `/v1/responses/compact` send-phase 也受总预算裁剪；本地 `cargo test` 全量通过。
