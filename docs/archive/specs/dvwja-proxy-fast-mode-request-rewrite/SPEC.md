# 反向代理 Fast 模式请求改写（三态设置，`requestedServiceTier`=上游实际请求值）（#dvwja）

## 状态

- Status: 已完成（4/4）
- Created: 2026-03-09
- Last: 2026-04-05

## 背景 / 问题陈述

- 现有内置反向代理已经能采集 `requestedServiceTier` 与 `serviceTier`，但尚不支持按设置自动把请求改写为 Fast / priority processing。
- 现有 `requestedServiceTier` 口径已经冻结为“实际发给上游的请求值”，因此任何代理级请求改写都必须同步反映到落库 payload、HTTP `/api/invocations`、SSE `records` 与前端详情展示。
- 反向代理目前只有 `chat.completions` 流式 `include_usage` 的 env 级改写能力，没有可在设置页切换的三态 Fast rewrite 开关。

## 目标 / 非目标

### Goals

- 在 reverse-proxy `proxy` 设置域新增 `fastModeRewriteMode: 'disabled' | 'fill_missing' | 'force_priority'`，并通过 `/api/settings` 与 `/api/settings/proxy` 稳定读写。
- 让 `POST /v1/responses` 与 `POST /v1/chat/completions` 都支持三态 Fast 请求改写。
- 保持 `requestedServiceTier` 继续表示最终发给上游的请求值；若请求被改写，则该字段必须反映改写后的值。
- 保持 startup backfill 与新语义一致：从 `request_raw_path` 回填的仍是最终实际上游请求值。
- 在设置页代理卡片内提供三态配置与文案，不新增公开观测字段。

### Non-goals

- 不扩展到 `POST /v1/embeddings`、`/v1/images`、`/v1/audio` 或其它 `/v1/*` 端点。
- 不新增用于区分“原始请求值”和“改写后请求值”的公开 API 字段或数据库列。
- 不把 `PROXY_ENFORCE_STREAM_INCLUDE_USAGE` 搬到设置页，也不改变其 env-only 行为。
- 不新增 Fast tier 聚合统计、筛选器或额外仪表盘卡片。

## 范围（Scope）

### In scope

- `src/main.rs`：proxy settings schema / load / save / API 合约扩展、请求体三态改写、payload summary 保真、历史回填与测试。
- `web/src/lib/api.ts`、`web/src/hooks/useSettings.ts`、`web/src/pages/Settings.tsx`：设置类型、保存逻辑、UI 选择器与说明文案。
- `web/src/components/SettingsPage.stories.tsx`、Vitest / Playwright 场景更新。
- `docs/specs/README.md` 与本规格状态同步。

### Out of scope

- `forwardProxy` 设置域改造。
- `/api/invocations` 或 SSE 新增额外字段。
- 任何数据库表新增新行级观测列。

## 需求（Requirements）

### MUST

- `fastModeRewriteMode` 默认值必须为 `disabled`，旧数据库升级后保持现有行为不变。
- 改写范围仅限 `POST /v1/responses` 与 `POST /v1/chat/completions`。
- `disabled`：不改写 tier。
- `fill_missing`：仅当请求体顶层缺少 `service_tier` / `serviceTier` 时注入 `service_tier: 'priority'`。
- `force_priority`：无条件把最终请求 tier 改为 `service_tier: 'priority'`。
- 改写后若请求体存在 `serviceTier` 别名，必须清理冲突并统一为顶层 `service_tier`，避免双字段并存。
- `requestedServiceTier` 必须始终表示最终发给上游的请求值；未改写时沿用原值，改写命中时返回 `priority`。
- 任何 pool 路由请求只要最终出站 body 已被重写或物化成新的内存快照，就不得继续转发下游原始 `Content-Length`；上游客户端必须按最终 body 重新计算。
- 即使 pool transport failure 发生在首字节之前，request raw 与 `requestedServiceTier` 也必须反映最终出站请求，而不是改写前的原始 body。
- startup backfill 继续只扫描 `source=proxy` 且 `request_raw_path IS NOT NULL` 的记录，并从 request raw 文件回填最终请求值。

### SHOULD

- 请求体 JSON 解析失败时保持透传原 body，不因 Fast rewrite 额外拦截请求。
- 新设置应复用现有代理设置卡片的 auto-save 流程，不引入单独提交按钮。
- 说明文案应明确三态差异，并提示只有受支持的双接口会应用改写。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 设置页加载 `/api/settings` 后展示当前 `fastModeRewriteMode`，用户切换时通过 `PUT /api/settings/proxy` 自动保存。
- proxy 请求进入 `prepare_target_request_body` 后，先解析原请求 JSON，再根据当前模式决定是否注入或覆盖 tier。
- 请求真正发往上游前，代理重新计算 `requested_service_tier`，并将其写入 payload summary 与 raw request 文件。
- 启动阶段 `backfill_proxy_requested_service_tiers` 从 request raw 文件读取最终请求 JSON，因此与运行时落库口径保持一致。

### Edge cases / errors

- 若请求体不是合法 JSON，Fast rewrite 不生效，`requestedServiceTier` 保持按现有逻辑缺失。
- 若请求体已有 `service_tier: 'auto'/'default'/'flex'/'scale'`：`fill_missing` 不覆盖；`force_priority` 覆盖为 `priority`。
- 若请求体只携带 `serviceTier`：`fill_missing` 视为“已有 tier”；`force_priority` 覆盖为标准字段 `service_tier=priority`。
- 当模式为 `disabled` 时，请求体字段形状保持原样透传；只有 `fill_missing` / `force_priority` 命中时，才统一输出顶层 `service_tier` 并清理 `serviceTier` 别名。
- 非目标端点继续保持完全透明透传，不读取 body JSON。

## 接口契约（Interfaces & Contracts）

| 接口（Name）              | 类型（Kind）         | 范围（Scope） | 变更（Change） | 负责人（Owner） | 使用方（Consumers）     | 备注（Notes）                              |
| ------------------------- | -------------------- | ------------- | -------------- | --------------- | ----------------------- | ------------------------------------------ |
| `GET /api/settings.proxy` | HTTP API             | internal      | Modify         | backend         | web settings            | 新增 `fastModeRewriteMode`                 |
| `PUT /api/settings/proxy` | HTTP API             | internal      | Modify         | backend         | web settings            | 接受三态枚举                               |
| `ProxySettings`           | TypeScript interface | internal      | Modify         | web             | settings page / stories | 新增三态字段                               |
| `requestedServiceTier`    | HTTP/SSE field       | internal      | Semantics only | backend + web   | invocation surfaces     | 保持字段名不变，仅冻结“最终上游请求值”语义 |

## 验收标准（Acceptance Criteria）

- Given 旧数据库无该设置列，When 服务启动后读取 `/api/settings`，Then `fastModeRewriteMode` 返回 `disabled`。
- Given `fill_missing` 且 `/v1/responses` 请求缺 tier，When 请求被转发，Then 上游收到 `service_tier=priority`，且 `requestedServiceTier` 为 `priority`。
- Given `fill_missing` 且 `/v1/chat/completions` 请求已有 `serviceTier=flex`，When 请求被转发，Then 上游仍收到 `flex`，且 `requestedServiceTier` 为 `flex`。
- Given `force_priority` 且任一目标端点请求已有 `service_tier=default`，When 请求被转发，Then 上游收到 `service_tier=priority`，且 `requestedServiceTier` 为 `priority`。
- Given 历史 proxy 记录存在 `request_raw_path` 且 payload 缺 `requestedServiceTier`，When 服务启动执行回填，Then 回填值与 raw request 中最终 tier 一致。
- Given 设置页切换三态后刷新页面，When 重新加载 `/api/settings`，Then UI 回显与服务端保存值一致。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests：覆盖 schema migration 默认值、settings API、双接口三态 rewrite、alias cleanup 与 requested tier 语义。
- Vitest：覆盖 settings payload normalization 与设置页三态 UI 文案/回显。
- Playwright：覆盖设置页切换三态并验证刷新后保持。
- 回归：`cargo test`、`cargo check`、`cd web && npm run test`、`cd web && npm run build`。

## 方案概述（Approach, high-level）

- 继续复用 `proxy_model_settings` 单例设置存储 reverse-proxy 行为，避免引入新的配置域。
- 复用 `prepare_target_request_body` 作为唯一请求改写入口，把 Fast tier rewrite 与 `include_usage` 注入放在同一 JSON 重写管线中。
- 保持 `requestedServiceTier` 与 request raw 文件都基于“最终发给上游的 body”取值，使运行时与 backfill 口径天然一致。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：部分模型/项目即便请求 `priority`，响应 `serviceTier` 仍可能不是 `priority`；这是上游行为，代理只保证请求改写成功。
- 风险：旧请求体若同时包含 `service_tier` 与 `serviceTier` 且值冲突，改写前的“原始意图”会被标准化；本次接受该行为，因为字段语义冻结为最终上游请求值。
- 开放问题：None。
- 假设：现有 request raw 文件保存的是最终发给上游的 body，这一实现约定保持不变。

## 里程碑（Milestones）

- [x] M1: 新规格建档并在 `docs/specs/README.md` 建立索引。
- [x] M2: 后端设置存储、API 合约与双接口 Fast rewrite 完成。
- [x] M3: 设置页三态 UI、类型与测试完成。
- [x] M4: 本地验证、spec sync、快车道提交与 PR 收敛完成。

## 变更记录（Change log）

- 2026-03-09: 创建规格，冻结三态 Fast rewrite 语义、双接口范围与 `requestedServiceTier` 最终请求值口径。
- 2026-03-09: 完成 SQLite 设置迁移、双接口 tier 改写与 `requestedServiceTier` 最终值回写，补齐 Settings UI、Storybook mock、Vitest 与 Playwright 覆盖，并通过 `cargo test`、`cargo check`、`cd web && npm run test`、`cd web && npm run build`。
- 2026-03-09: 根据 review 调整 `disabled` 模式为真正透明透传；仅在 `fill_missing` / `force_priority` 生效时才标准化 `service_tier` 字段形状，并补充对应回归测试。
- 2026-03-09: 创建 PR #102，补齐 release labels，并在变基到最新 `main` 后确认本地验证与 GitHub Actions checks 全部通过。
- 2026-04-05: 补充 pool fast hotfix 不变量：body rewrite 后必须丢弃 stale `Content-Length`，且 send-stage transport failure 仍需保留最终出站 request raw 与 `requestedServiceTier`。
