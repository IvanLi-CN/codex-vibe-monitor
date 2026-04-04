# 上游账号 Tag Fast 模式服务层改造（#bnjhy）

## 状态

- Status: 已完成
- Created: 2026-04-04
- Last: 2026-04-04

## 背景 / 问题陈述

- 现有 Fast tier 请求改写仍残留在全局 proxy settings 语义中，无法按最终命中的上游账号动态生效。
- 号池路由已经支持按 tag 计算账号级 effective rule，但 tag 规则层还不能表达 `service_tier` 改写策略。
- 当 pool failover 从账号 A 切到账号 B 时，当前实现沿用预路由阶段生成的请求体，不能保证 `requestedServiceTier` 与真实发往 B 的请求一致。

## 目标 / 非目标

### Goals

- 在 tag 路由规则层新增 `fastModeRewriteMode` 四态：`force_remove`、`keep_original`、`fill_missing`、`force_add`，默认 `keep_original`。
- 多 tag 合并时固定采用 `force_remove > force_add > fill_missing > keep_original` 的最严格顺序。
- 将 `service_tier` 改写从全局预路由阶段迁移到“选中账号之后、发往上游之前”的逐账号尝试链路。
- 清理旧全局 fast rewrite 的运行时与内部 API 合约依赖，但保留 SQLite 物理列作为惰性遗留。
- 保持 `requestedServiceTier` 继续表示最终实际发往上游的请求值。

### Non-goals

- 不新增 Settings 页面、Tag Dialog 或其他前端可见配置入口。
- 不扩展 `POST /v1/responses/compact` 的 rewrite 范围。
- 不做 `proxy_model_settings.fast_mode_rewrite_mode` 的 SQLite 删列迁移。

## 范围（Scope）

### In scope

- `src/upstream_accounts/mod.rs`：tag schema、CRUD 合约、effective rule 合并与账号级 fast 模式解析。
- `src/main.rs`：移除旧全局 fast rewrite 运行时依赖，把请求体 tier 改写迁到 pool 单账号尝试链路，并保证 failover 按目标账号重新生成请求体。
- `src/api/mod.rs`：清理 `PUT /api/settings/proxy` 的残留 fast 字段写入路径。
- `src/tests/mod.rs` 与 `src/upstream_accounts/mod.rs` tests：补齐 tag 四态、合并顺序、逐账号改写与 failover 语义回归。

### Out of scope

- Settings、Tag 管理、Storybook 或其他 UI 层改造。
- 新增原始 tier / 改写后 tier 双字段对外观测。
- 改变已有 compact 请求透明透传的语义。

## 需求（Requirements）

### MUST

- `fastModeRewriteMode` 必须在 tag CRUD 的 create / update / list / detail / account summary / effective rule 中稳定 round-trip，未传时默认为 `keep_original`。
- 旧 tag 数据升级后必须默认得到 `keep_original`，保持现有行为不变。
- 多 tag 合并必须固定按 `force_remove > force_add > fill_missing > keep_original` 收敛。
- `POST /v1/responses` 与 `POST /v1/chat/completions` 仅在命中 pool 上游账号后，才按该账号 effective rule 改写请求体。
- `force_remove` 必须移除顶层 `service_tier` 与 `serviceTier`。
- `keep_original` 必须保持 tier 字段完全透传。
- `fill_missing` 仅在顶层 tier 缺失时补 `service_tier=priority`；若请求仅存在 `serviceTier`，必须保持原样透传。
- `force_add` 必须无条件把最终请求标准化为 `service_tier=priority`，并清理 `serviceTier` 别名。
- failover 每次切换账号时，都必须从原始请求快照重新套用当前账号的 tag fast 模式，而不是复用上一次账号生成的请求体。
- `requestedServiceTier` 必须始终反映最终实际发往上游的请求值。
- 残留全局 fast rewrite 不能再参与运行时决策，`PUT /api/settings/proxy` 也不能再承载该字段语义。

### SHOULD

- pool 运行态 SSE 快照在账号已选定后，应尽量同步展示该账号实际生效的 `requestedServiceTier`。
- 对无法解析为 JSON 的请求体，应维持透明透传，不因账号级 fast 模式额外阻断请求。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 用户通过 tag 规则把某些账号设为 `force_remove` / `keep_original` / `fill_missing` / `force_add` 后，服务端在每次选中该账号时动态改写目标请求体。
- 同一个账号绑定多个 tag 时，服务端先计算 effective rule，再把 effective fast 模式用于该账号的所有 pool 请求尝试。
- pool 首次命中账号 A 时按 A 的 effective fast 模式生成请求；若 A 失败并切换到 B，则重新基于原始请求生成 B 对应的新请求体。
- 请求最终落库时，`request_raw_path` 与 payload 内 `requestedServiceTier` 都指向“实际发给最终上游账号的请求”。

### Edge cases / errors

- 若请求体不是合法 JSON，则 tag fast 模式不生效，仍透传原始 body。
- `POST /v1/responses/compact` 始终跳过 fast rewrite。
- OAuth `/v1/responses` 仍遵守既有 body materialize / 大小限制；账号级 fast rewrite 不能绕过这些限制。

## 接口契约（Interfaces & Contracts）

- `CreateTagRequest.fastModeRewriteMode`
- `UpdateTagRequest.fastModeRewriteMode`
- `TagRoutingRule.fastModeRewriteMode`
- `TagSummary.routingRule.fastModeRewriteMode`
- `AccountTagSummary.routingRule.fastModeRewriteMode`
- `EffectiveRoutingRule.fastModeRewriteMode`

## 验收标准（Acceptance Criteria）

- Given 旧 tag 行没有 `fast_mode_rewrite_mode` 列值，When 服务升级后加载 tag，Then effective 值为 `keep_original`。
- Given 同一账号同时挂有 `keep_original`、`fill_missing`、`force_add`、`force_remove` 不同 tag，When 计算 effective rule，Then 最终模式按 `force_remove > force_add > fill_missing > keep_original` 收敛。
- Given pool 请求先命中 `keep_original` 账号 A，后 failover 到 `force_add` 账号 B，When 最终请求发给 B，Then `requestedServiceTier=priority` 且 request raw 保存的是 B 的最终请求体。
- Given tag fast 模式为 `force_remove`，When 请求体原本包含 `service_tier` 或 `serviceTier`，Then 发送给上游的最终请求中这两个字段都不存在。
- Given `PUT /api/settings/proxy`，When 提交旧的 `fastModeRewriteMode` 字段，Then 服务端不再依赖该字段保存或驱动运行时 fast rewrite。
- Given `POST /v1/responses/compact`，When 账号 tag fast 模式为任意四态，Then 请求体仍保持透明透传。

## 非功能性验收 / 质量门槛（Quality Gates）

- `cargo check`
- `cargo test`

## 风险 / 假设

- 假设：pool route 的最终请求体才是 `requestedServiceTier` 的唯一真相源；预路由阶段解析结果只作为初始信息。
- 风险：对需要重写的 file-backed replay body，failover 时会发生一次显式 materialize；当前接受该成本，以换取逐账号规则正确性。
