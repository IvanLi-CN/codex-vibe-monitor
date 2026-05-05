# Codex 远程压缩请求记录、展示与计费接入（#g3amk）

## 背景 / 问题陈述

- 现有反向代理采集链路面向 `POST /v1/responses` 与 `POST /v1/chat/completions` 已有完整的请求记录、SSE records、Dashboard / Live 统计与成本估算能力，但 `POST /v1/responses/compact` 尚未被稳定识别为独立代理目标。
- Codex 远程压缩请求若未进入现有 `codex_invocations` 采集链路，就无法出现在请求列表、SSE records、summary、timeseries 与 cost totals 中，导致 Dashboard / Live 对真实流量低估。
- 当前 pricing 引擎已经按 model 维度支持 exact model、dated alias 与 unknown model 语义；compact 不需要也不应再引入 endpoint 专属价格配置。
- 当前公开 compact 接口参数里 `service_tier` 不存在；服务端是否存在未公开兼容行为未检查，因此本次必须避免对 compact 自动注入 `service_tier` 或 chat-only `stream_options.include_usage`。

## 目标 / 非目标

### Goals

- 把 `POST /v1/responses/compact` 识别为新的 `ProxyCaptureTarget`，稳定进入现有代理采集、持久化、SSE records 与 `/api/invocations` 返回链路。
- compact 请求落库时保留原始 endpoint=`/v1/responses/compact`，并让主列表可见标记“远程压缩 / Compact”，同时详情区继续展示 endpoint 原文。
- compact 响应复用现有 usage / model 解析与 `estimate_proxy_cost`，让 request count、tokens、cost 自动流入 `stats`、`summary`、`timeseries`。
- 明确 compact 不走 Fast mode rewrite，不注入 `service_tier`，不注入 chat-only `stream_options.include_usage`。
- 当未来 compact 请求体或转发头缺少 `promptCacheKey` / `stickyKey` 时，后端可基于同一客户端稳定指纹的近期普通 `/v1/responses` 记录补齐对话归因。
- 在设置页 pricing 区说明 compact 按命中的模型单价估算成本，不新增公开配置结构。

### Non-goals

- 不新增 compact endpoint 专属 pricing schema、额外数据库列或新的 `/api/*` 统计字段。
- 不新增 compact 独立统计页、筛选页或独立成本面板。
- 不对 compact 注入 `service_tier`、套用 Fast mode rewrite 或附带 chat-only `stream_options.include_usage`。
- 不验证公开文档之外 compact 是否私下兼容 `service_tier`；当前状态为未检查。
- 不对已落库旧 compact 做历史 backfill 或启发式补数；缺少 key 的旧记录继续保持原状。

## 范围（Scope）

### In scope

- `src/main.rs`：扩展 compact capture target、payload endpoint、usage/model 推断、成本估算复用与相关 Rust 回归。
- `web/src/components/InvocationTable.tsx`：主列表（桌面 / 移动）显示 compact 标记，同时保留详情 endpoint 原文展示。
- `web/src/i18n/translations.ts`、`web/src/pages/Settings.tsx`：新增 compact pricing 说明文案。
- `docs/specs/README.md` 与本规格状态同步。

### Out of scope

- `PricingCatalog` / `/api/settings/pricing` 结构改造。
- 对 compact 新增 `serviceTier` 推断或独立服务等级配置。
- 对非 compact 端点调整既有统计口径。

## 需求（Requirements）

### MUST

- 仅 `POST /v1/responses/compact` 命中 compact capture target；其它 method 或 path 不得误判。
- compact 落库 payload 必须保留 `endpoint: "/v1/responses/compact"`，以便 `/api/invocations` 与 SSE `records` 继续通过既有 `endpoint` 字段识别。
- compact 成本估算必须复用现有 `estimate_proxy_cost` 链路，保持 exact model、dated alias 与 unknown model 的既有语义不变。
- 当 compact 响应缺少 response model 时，估价必须回退使用请求体 model。
- compact 请求不得触发 Fast mode rewrite，不得自动注入 `service_tier`，不得自动注入 chat-only `stream_options.include_usage`。
- future compact 记录在请求体或转发头缺少 `promptCacheKey` 时，若同一客户端稳定指纹在短 TTL 内存在最近普通 `/v1/responses` 的唯一映射，落库 payload 必须补齐 `promptCacheKey`，并在可用时补齐 `stickyKey`。
- 客户端指纹诊断字段只能保存短 hash / fingerprint，不得保存 `session_id`、`originator`、`x-codex-window-id`、`x-codex-installation-id`、`traceparent` 的明文值。
- compact 记录必须自动计入 `GET /api/stats`、`GET /api/stats/summary`、`GET /api/stats/timeseries` 的请求数、tokens 与 cost。
- InvocationTable 主列表在桌面与移动布局都必须对 compact 记录显示可见标记，详情面板继续展示原始 endpoint 文本。

### SHOULD

- compact 标记应复用现有 badge 语义，不引入新的 API 字段或额外数据转换层。
- 设置页文案应明确“compact 按模型单价估算”，避免用户误解为 endpoint 单独定价。
- 新增标记不应引入桌面表格横向滚动、列表截断失控或详情按钮错位。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 请求进入代理时，`capture_target_for_request()` 对 `POST /v1/responses/compact` 返回 `ProxyCaptureTarget::ResponsesCompact`。
- `prepare_target_request_body()` 在 compact 路径只做 JSON 解析与信息提取，不执行 Fast rewrite，也不执行 chat stream usage 注入。
- 响应采集阶段沿用现有 usage / model 解析逻辑，compact 的 `response.compaction` 响应若携带 `usage` 即正常提取 tokens。
- payload summary 通过 `target.endpoint()` 持久化 compact endpoint，后续 `/api/invocations`、SSE `records`、startup backfill 与详情展示均保持同一来源。
- 普通 `/v1/responses` 成功提取 `promptCacheKey` 时，后端维护运行期“客户端稳定指纹 -> 最近 `promptCacheKey` / `stickyKey`”映射；stable key 只使用 `session_id`、`originator`、`x-codex-window-id`，且必须至少包含 `session_id` 或 `x-codex-window-id` 这类强稳定键，`x-codex-installation-id` 与 `traceparent` 仅作为诊断 fingerprint 保存。
- compact 缺少 key 时，从同一 stable client fingerprint 的近期唯一映射补齐对话归因，并写入 `promptCacheKeyAttributionSource="client_fingerprint_recent"`；无 stable fingerprint、TTL 过期、无法匹配或同一 fingerprint 在 TTL 内出现多个不同 key 时保持无 key。
- 前端通过现有 `endpoint` 字段判断 compact，并在主列表 badge 位置显示“远程压缩 / Compact”。
- 统计接口继续使用同一 `codex_invocations` 数据源，因此 compact 自动进入 totals、summary 与 timeseries。

### Edge cases / errors

- 若 compact 响应缺少 response model，但请求体有 `model`，则成本估算回退使用请求体 model。
- 若 compact model 未命中 pricing catalog，成本行为保持现有 unknown-model 语义，不新增特殊 fallback。
- 若 compact 响应缺少 `usage`，记录仍可落库，但 tokens / cost 继续遵循现有“无 usage 则无成本”的代理语义。
- 若 payload 缺少 endpoint，旧记录仍按现有 fallback 逻辑解析为普通 responses；compact 专属识别不回写旧记录。
- 若 compact 与最近普通 responses 来自不同客户端稳定指纹，不得跨客户端继承 prompt-cache key。

## 接口契约（Interfaces & Contracts）

| 接口（Name）                   | 类型（Kind）         | 范围（Scope） | 变更（Change） | 负责人（Owner） | 使用方（Consumers）    | 备注（Notes）                                  |
| ------------------------------ | -------------------- | ------------- | -------------- | --------------- | ---------------------- | ---------------------------------------------- |
| `POST /v1/responses/compact`   | HTTP proxy endpoint  | internal      | Add            | backend         | Codex proxy clients    | 新的被监控代理目标，与 `/v1/responses` 并列    |
| `ApiInvocation.endpoint`       | TypeScript field     | internal      | Extend         | backend / web   | InvocationTable / SSE  | 允许返回 `"/v1/responses/compact"`             |
| `PricingCatalog`               | Rust / TS schema     | internal      | None           | backend / web   | settings / cost engine | 继续按 model 定价，不新增 compact 专属 schema  |
| `InvocationTable` compact 标记 | UI presentation rule | internal      | Modify         | web             | Dashboard / Live       | 仅新增显示语义，不改变详情区 endpoint 原文展示 |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given 代理收到 `POST /v1/responses/compact`，When 上游返回 `response.compaction` 且含 `usage`，Then `codex_invocations` 新增 row，`endpoint=/v1/responses/compact`，tokens 与 cost 正常落库。
- Given compact 响应未携带 model，When 估价，Then 回退使用请求体 model；Given model 未命中 catalog，Then cost 行为保持现有 unknown-model 语义。
- Given compact row 已落库，When 查询 `GET /api/invocations` 或接收 SSE `records`，Then 前端能看到 compact 标记且详情仍显示 endpoint 原文。
- Given compact row 已落库，When 查询 `GET /api/stats`、`GET /api/stats/summary`、`GET /api/stats/timeseries`，Then request count、tokens、cost 自动包含 compact。
- Given Fast mode rewrite 开启，When 发送 compact 请求，Then 发往上游的请求体不新增 `service_tier`，也不附带 chat-only `stream_options.include_usage`。
- Given 普通 `/v1/responses` 带 `promptCacheKey` 且包含客户端稳定指纹，When 同一稳定指纹在短 TTL 内发送缺 key 的 compact，Then 新 compact 落库 payload 含同一 `promptCacheKey` / `stickyKey`，并能出现在 prompt-cache conversation recent invocations。
- Given compact 缺 key 且客户端稳定指纹不同、缺失、只有弱字段或 TTL 过期，When 落库，Then 不得继承其它对话的 `promptCacheKey`。
- Given 新增 compact 标记后运行前端测试，When 检查桌面与移动布局，Then 不出现新增横向滚动、截断失控或详情按钮错位。

## 实现前置条件（Definition of Ready / Preconditions）

- compact 请求匹配规则固定为 `POST /v1/responses/compact`：已确定。
- compact 必须进入现有 totals、summary、timeseries 与成本统计：已确定。
- compact 成本估算沿用现有 model pricing，而不是 endpoint pricing：已确定。
- compact 当前公开参数里 `service_tier` 不存在，因此本次默认不注入：已确定。
- compact 是否存在公开文档之外可工作的 `service_tier` 兼容行为：未检查。
- 本地是否已有真实 compact 历史数据可供手工回放验证：未检查。
- compact 对话归因历史策略：仅修未来，旧数据不补写。

### Quality checks

- `cargo fmt -- --check`
- `cargo test`
- `cargo check`
- `cd web && npm run test`
- `cd web && npm run build`
- `cd web && npm run test:e2e -- tests/e2e/invocation-table-layout.spec.ts`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 后端识别 `POST /v1/responses/compact` 为独立 capture target，并保留 payload endpoint 原文。
- [x] M2: compact 复用现有 usage / pricing 链路，自动进入 tokens / cost / summary / timeseries 统计。
- [x] M3: InvocationTable 主列表新增 compact 标记，设置页补充“按模型单价估算”的说明文案。
- [x] M4: Rust / Vitest / build / Playwright 回归通过，确认 compact 标记未引入新的布局回归。
- [x] M5: 完成 fast-track 远端交付（push / PR / checks 结果明确 / review-loop 收敛）。

## 方案概述（Approach, high-level）

- 以 `ProxyCaptureTarget::ResponsesCompact` 作为后端唯一新增分支，复用既有响应 usage 解析、模型回退与 pricing 计算，避免复制统计链路。
- 用 `endpoint` 作为唯一前后端共享识别信号：后端持久化 compact endpoint，前端用它渲染主列表 compact badge，详情区直接展示原值。
- 将 compact 的“跳过 rewrite / 注入”边界内聚到 capture target 能力函数，避免普通 responses/chat 逻辑误应用到 compact。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：旧历史 payload 若没有 endpoint 字段，无法自动从历史数据中追溯 compact 类型；当前只保证新采集请求稳定识别。
- 风险：旧 compact payload 若缺少 prompt-cache key，不会出现在 prompt-cache conversation 归因视图；当前按“仅修未来”策略避免历史误归因。
- 风险：compact 若未来公开 `service_tier` 参数，当前实现仍会保持“不注入”策略，需要后续根据官方文档再评估。
- 假设：compact 响应中的 `usage` 结构继续与现有 usage 解析兼容。
- 假设：`session_id`、`originator`、`x-codex-window-id` 在同一客户端窗口内足够稳定，可用于短 TTL 内归因；`x-codex-installation-id` 与 `traceparent` 可能在普通 responses 与 compact 间不一致，不参与 stable matching。

## 变更记录（Change log）

- 2026-03-09: 创建规格，冻结 compact 识别、统计口径、计费口径与“不注入 `service_tier`”边界。
- 2026-03-09: 已完成后端 compact capture / pricing / stats 接入，以及 InvocationTable compact 标记与 settings 文案改动。
- 2026-03-09: 已完成本地 Rust / web 验证与 review-loop 审查；远端 PR、checks 与 merge readiness 已收敛。
- 2026-04-27: 补充 future-only compact prompt-cache 归因要求；缺 key 的新 compact 可通过同一客户端稳定指纹继承最近普通 responses 的 `promptCacheKey` / `stickyKey`，旧记录不 backfill。

## 参考（References）

- `docs/specs/dvwja-proxy-fast-mode-request-rewrite/SPEC.md`
- `docs/specs/rw32e-invocation-fast-mode-indicator/SPEC.md`
- `docs/specs/r8m3k-invocation-table-responsive-no-overflow/SPEC.md`
