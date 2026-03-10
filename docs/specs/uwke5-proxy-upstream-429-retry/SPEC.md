# 反向代理上游 429 自动重试（设置可配）（#uwke5）

## 状态

- Status: 部分完成（4/5）
- Created: 2026-03-10
- Last: 2026-03-10

## 背景 / 问题陈述

- 现有反向代理在上游返回 `429 Too Many Requests` 时会直接把结果返回客户端，缺少代理侧的自动重试能力。
- forward proxy 运行态已经会基于每次 attempt 更新权重，但 `429` 目前不会被单独识别为“可重试的限流失败”，因此无法在下一次 attempt 前重新挑选 proxy。
- 设置页现有 `proxy` 域已承载 `/v1/models` hijack、merge upstream 与 Fast rewrite 配置，本次新增的重试开关需要沿用同一套 auto-save 与 `/api/settings/proxy` 合约。
- 非 capture 的 `/v1/*` 透传路径当前直接把请求体流式包给上游，无法在收到 `429` 后安全重放同一份 body。

## 目标 / 非目标

### Goals

- 为反向代理所有上游请求补充 `429` 自动重试，默认最大重试次数为 `3`，并允许在设置页的 `proxy` 卡片内配置为 `0..5`。
- 让 `GET /api/settings`、`PUT /api/settings/proxy`、Rust `ProxyModelSettings*`、TypeScript `ProxySettings` 与 SQLite `proxy_model_settings` 在 `upstream429MaxRetries` 上保持稳定 round-trip。
- 仅在“上游响应头阶段返回 429”时触发自动重试；一旦响应已开始向下游写出，则不再做二次尝试。
- 中间 `429` attempt 只写 `forward_proxy_attempts`，记为 `upstream_http_429`，不新增中间 `codex_invocations` 记录。
- `/v1/models` 在 hijack+merge upstream 模式下复用同一重试策略；若重试耗尽，仍保持当前 fallback 到 preset models 的行为。

### Non-goals

- 不为 `5xx`、网络错误、stream 中途断流等其它失败类型引入新的自动重试策略。
- 不改变现有请求体大小限制、请求体读取超时、坏路径拦截与 capture payload 结构的既有口径。
- 不新增独立的 dashboard/统计 UI 来展示 429 retry 次数。

## 范围（Scope）

### In scope

- `src/main.rs`：proxy settings schema / load / save / API 合约扩展、generic proxy 请求改为可重放 body、429 retry helper 接线与测试。
- `src/forward_proxy/mod.rs`：shared retry helper、`Retry-After` 解析、forward proxy attempt failure kind 与 `/v1/models` merge fetch 接线。
- `web/src/lib/api.ts`、`web/src/pages/Settings.tsx`、`web/src/i18n/translations.ts`、`web/src/components/SettingsPage.stories.tsx`：新设置项与 auto-save UI。
- `docs/specs/README.md` 与本规格状态同步。

### Out of scope

- `forwardProxy` 设置域改造。
- 首字节后开始 streaming 的响应重试。
- 新增独立数据库表或 invocation 级 retry 计数字段。

## 需求（Requirements）

### MUST

- `upstream429MaxRetries` 默认值必须为 `3`，允许值范围为 `0..5`，其中 `0` 表示关闭自动重试。
- `proxy_model_settings` 必须新增 `upstream_429_max_retries INTEGER NOT NULL DEFAULT 3`，旧库升级后未配置实例保持默认 `3`。
- 所有反向代理上游请求在响应头阶段收到 `429` 时，都必须先记录一次 `forward_proxy_attempts` 失败（failure kind=`upstream_http_429`），再依据配置决定是否等待并重试。
- 若上游响应带 `Retry-After`：必须支持 `delay-seconds` 与 HTTP-date 两种格式；若缺失、非法或已过期，则退回内置 backoff `min(500ms * 2^(attempt-1), 5s)`。
- capture-target 路径成功重试后，最终客户端只能看到一次成功响应，且 `codex_invocations` 只持久化最终结果。
- 非 capture `/v1/*` 透传路径必须先把请求体按现有 body limit 读入内存，再发往上游，确保 429 后可重放同一份方法 / query / headers / body。
- 若 429 重试耗尽：直接反代路径必须原样返回最终 `429` 的状态码、关键响应头与响应体；`/v1/models` merge 路径则维持现有 preset fallback，并继续打 `x-proxy-model-merge-upstream=failed`。

### SHOULD

- 429 retry helper 应集中封装，避免 capture-target、generic pass-through 与 `/v1/models` 各自手写循环。
- 设置页控件应复用代理卡片现有 auto-save 行为，不新增单独提交按钮。
- 测试应明确覆盖 `Retry-After` 优先级、fallback backoff、重试后重新选 proxy 与 exhaustion 语义。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 服务启动读取 `proxy_model_settings` 时，将 `upstream_429_max_retries` 解析为 `0..5` 的整型配置，并通过 `/api/settings` 暴露给前端。
- 设置页代理卡片新增 `upstream429MaxRetries` 控件；用户修改后立即通过 `PUT /api/settings/proxy` auto-save。
- 代理请求首次选定 `SelectedForwardProxy` 后，若上游响应为 `429`，先记录 attempt 失败、等待 `Retry-After`/fallback backoff、重新选 proxy，再用同一份请求重新发起下一次 attempt。
- capture-target 路径在最终成功后继续沿用现有 capture/persist/broadcast 流程；中间 429 attempt 不写 invocation 主记录。
- generic pass-through 路径在最终成功或非 429 响应时，继续沿用现有 header 过滤、redirect 规范化与 stream forwarding 逻辑。
- `/v1/models` merge upstream 在重试成功时继续走 merge；重试耗尽或其它错误时与当前行为一致，回退 preset payload。

### Edge cases / errors

- 当 `upstream429MaxRetries=0` 时，429 自动重试完全关闭，首个 `429` 结果按当前路径语义直接返回/回退。
- 若 `Retry-After` 给出的 HTTP-date 早于当前时间，则视为无效并使用 fallback backoff。
- 若请求体在读入阶段已触发 `413`、读取超时或 client-closed 错误，必须在首次 attempt 前就终止，不进入 429 retry 逻辑。
- 若上游不是 `429`，helper 不应额外等待，也不改变既有 `5xx`、send error、stream error 的处理分支。
- 若 429 最终发生在 `/v1/models` merge upstream，merge status 继续标记为 `failed`，但不把 preset fallback 误记为上游 merge success。

## 接口契约（Interfaces & Contracts）

| 接口（Name）               | 类型（Kind）         | 范围（Scope） | 变更（Change） | 负责人（Owner） | 使用方（Consumers）        | 备注（Notes）                          |
| -------------------------- | -------------------- | ------------- | -------------- | --------------- | -------------------------- | -------------------------------------- |
| `GET /api/settings.proxy`  | HTTP API             | internal      | Modify         | backend         | web settings               | 新增 `upstream429MaxRetries`           |
| `PUT /api/settings/proxy`  | HTTP API             | internal      | Modify         | backend         | web settings               | 接受 `0..5` 整数                       |
| `ProxyModelSettings`       | Rust struct          | internal      | Modify         | backend         | proxy handlers             | 新增 `upstream_429_max_retries`        |
| `ProxySettings`            | TypeScript interface | internal      | Modify         | web             | settings page / stories    | 新增 `upstream429MaxRetries`           |
| `forward_proxy_attempts`   | SQLite row payload   | internal      | Modify         | backend         | live stats / failure stats | 新增 failure kind `upstream_http_429`  |
| proxy upstream send helper | internal helper      | internal      | Add            | backend         | proxy handlers             | 统一 `Retry-After` 与 backoff 重试逻辑 |

## 验收标准（Acceptance Criteria）

- Given 旧库尚未包含 `upstream_429_max_retries`，When 服务启动执行 schema migration，Then 新列被补齐且默认读取值为 `3`。
- Given 设置页把 `upstream429MaxRetries` 改成 `5`，When auto-save 完成并刷新页面，Then `/api/settings` 与前端状态都返回 `5`。
- Given capture-target 路径首个上游响应为 `429`、第二次为 `200`，When 客户端发起请求，Then 客户端只看到成功响应，且 invocation 表只有一条最终成功记录。
- Given generic `/v1/*` 透传路径首个上游响应为 `429`、第二次为 `200`，When 客户端发起请求，Then 第二次请求收到与第一次完全相同的 body，并成功返回。
- Given 上游 `429` 带 `Retry-After: 2` 或合法 HTTP-date，When helper 执行等待，Then 实际等待优先采用该值；Given header 无效，Then 退回 fallback backoff。
- Given `/v1/models` merge upstream 连续返回 `429` 直到耗尽，When 代理返回 hijack 结果，Then 返回 preset payload 且 `x-proxy-model-merge-upstream=failed`。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests：settings roundtrip、legacy schema migration default、capture-target retry success/exhaustion、generic proxy body replay、`/v1/models` merge retry、`Retry-After` 解析/backoff。
- Front-end tests：API normalize/update payload、Storybook/mock 设置 roundtrip。
- Browser smoke：真实浏览器打开 settings 页面，确认新控件可保存且页面会话保持打开供复查。

### Quality checks

- `cargo test`
- `cd web && npm run test`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: proxy settings schema / load / save / API 合约新增 `upstream429MaxRetries`。
- [x] M2: shared 429 retry helper 落地，并接入 capture-target、generic pass-through 与 `/v1/models` merge。
- [x] M3: 设置页代理卡片新增 `0..5` auto-save 控件与文案。
- [x] M4: Rust + web 自动化验证补齐，`Retry-After`/exhaustion 语义确认。
- [ ] M5: fast-track 交付完成（spec sync、push、PR、checks、review-loop 收敛）。

## 风险 / 假设

- 风险：generic pass-through 改为先读 body 会失去当前“边读边发”的上行特性，但请求体上限已存在，因此这次以内存可重放优先。
- 风险：若多个 proxy 节点都返回 `429`，重试只会在 `0..5` 次范围内切换，不保证一定避开限流。
- 假设：`forward_proxy_attempts` failure kind 只用于内部观测，本次新增 `upstream_http_429` 不需要前端单独新增展示字段。

## 变更记录（Change log）

- 2026-03-10: 创建 spec，冻结 429 自动重试的范围、设置接口与 exhaustion 语义。
- 2026-03-10: 实现落地：新增 `upstream429MaxRetries` 设置、429 重试 helper、全链路接线与回归测试。
