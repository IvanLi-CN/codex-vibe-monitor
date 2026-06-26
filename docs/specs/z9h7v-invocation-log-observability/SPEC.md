# 请求日志可观测性增强（IP / Cache Tokens / 分阶段耗时 / Prompt Cache Key / Body Logging Toggles）（#z9h7v）

## 背景 / 问题陈述

- 当前 `/api/invocations` 虽已包含 token 与成本，但缺少请求方来源信息（IP）、稳定请求标识（prompt cache key）与易读的阶段耗时展示。
- Web 端表格仅突出错误详情，无法在单次请求维度快速定位“慢在何处”。
- 代理链路已有 `payload` 与分阶段耗时字段落库能力，尚未系统化对外输出与前端展示。
- 远程压缩的新流量已迁移到 `/v1/responses` 内的 server-side compaction；只看 endpoint 已无法区分“启用了远程压缩 V2”与“最终真的触发了压缩响应”。
- 图片相关请求的运行时 `imageIntent` 已参与路由，但还未成为稳定的 invocation 对外可观测合同，导致 101 开日志时无法直接肉眼确认“图片工具”请求。

## 目标 / 非目标

### Goals

- 在不变更 SQLite 表结构的前提下，补齐请求级上下文字段：`requesterIp`、`promptCacheKey`、`endpoint`、`failureKind`。
- `/api/invocations` 向前端稳定返回分阶段耗时字段，支持“首字节 / 总耗时”与完整阶段详情展示。
- Live 与 Dashboard 共用表格统一升级，主表保持简洁，详情区保留完整诊断信息。
- 请求详情不再展示 `source`，也不把 `source` 当作代理名兜底；代理字段仅展示 payload 中已确认的 `proxyDisplayName`。
- 调用记录相关模型展示统一采用“响应模型优先”语义，并在请求模型与响应模型不一致时显示低干扰的上游路由差异图标。
- 号池尝试明细展示每次尝试实际落库的 `proxy_binding_key_snapshot`，用于失败链路诊断。
- `GET /api/settings` 与 `PUT /api/settings/proxy` 暴露两个独立布尔开关：`requestBodyLoggingEnabled`、`responseBodyLoggingEnabled`，默认都为 `true`。
- 关闭 body 记录时，仅停止新的 request/response 原文 body 落盘与响应 preview 持久化；结构化 payload、tokens、timing、routing/account、prompt cache key、reasoning/service tier 等字段继续写入。
- `/api/invocations` 与 SSE `records` 在不改 schema 的前提下额外返回 `compactionRequestKind` / `compactionResponseKind`，把原始 endpoint 与压缩语义解耦。
- `/api/invocations`、SSE `records` 与 Prompt Cache / Dashboard preview 在不改 schema 的前提下额外返回 `imageIntent`，使图片请求语义脱离 endpoint 和历史 raw body 存活状态而独立存在。
- `/api/invocations`、SSE `records`、Prompt Cache preview 与 Dashboard working conversations 在不改 schema 的前提下额外返回 `requestModel` / `responseModel`，用于区分请求模型与实际响应模型。
- Records 与 Dashboard 两个 owner-facing 列表同时显示独立“图片工具”徽标，避免同一条 invocation 在不同列表面出现语义漂移。

### Non-goals

- 不新增独立请求详情页。
- 不改现有统计聚合口径与时序聚合逻辑。
- 不新增“总日志开关”。
- 不对历史 `.gz` / raw 文件做立即删除、迁移或重压缩。

## 范围（Scope）

### In scope

- `src/main.rs` 代理采集增强、`/api/invocations` 列表字段扩展。
- `src/main.rs` 启动阶段全量回填历史 proxy 记录中的 `payload.promptCacheKey`。
- `web/src/lib/api.ts` 类型对齐后端返回。
- `web/src/components/InvocationTable.tsx` 新增 cache/latency 列与通用详情展开区。
- `web/src/i18n/translations.ts` 新增中英文文案键。
- `proxy_model_settings` 单例新增 request/response body logging 持久化字段与 settings 页面双开关 UI。
- request raw / response raw / response preview 按设置开关 fail-soft 退化，详情页与历史回填接受“新记录没有 raw body”为正常状态。

### Out of scope

- 任何数据库 schema 变更。
- 采集敏感头（如 `Authorization`）或原文脱敏策略重构。
- 统计页图表结构改造。

## 需求（Requirements）

### MUST

- requester IP 提取优先级固定：`x-forwarded-for` 首值 > `x-real-ip` > `Forwarded(for=...)` > peer ip 兜底。
- prompt cache key 提取优先级固定：请求体候选 JSON 指针（`/prompt_cache_key`、`/promptCacheKey`、`/metadata/prompt_cache_key`、`/metadata/promptCacheKey`）> 请求头候选键（`x-prompt-cache-key` 等）。
- `build_proxy_payload_summary` 在成功/失败路径都包含 `requesterIp` 与 `promptCacheKey`（缺失时为 null）。
- `/api/invocations` 返回新增字段：`requesterIp`、`promptCacheKey`、`endpoint`、`failureKind`。
- 前端主表新增 `Cache Tokens` 与 `Latency` 列（`First byte / Total`），详情区展示完整阶段耗时。
- 调用详情的“代理”字段只使用 `proxyDisplayName`；缺失时显示 `—`，即使 `source` 为 `xy`、`crs` 或其他来源也不参与展示。
- 号池尝试明细每条尝试展示“代理/Proxy”：`proxyBindingKeySnapshot` 缺失时显示 `—`，值为 `__direct__` 时显示 `Direct`，其他值通过绑定节点解析为代理显示名；解析失败时显示紧凑 key，完整 key 仅保留在 hover title。
- 启动回填会将历史记录中的 `payload.codexSessionId` 移除，并写入 `payload.promptCacheKey`。
- `requestBodyLoggingEnabled=false` 时，新请求不再写入 `request_raw_path` / request raw 文件；相关 size/truncation 字段维持空值或零值语义，不把该情况视为损坏。
- `responseBodyLoggingEnabled=false` 时，新响应不再写入 `response_raw_path` / response raw 文件，同时 `raw_response` inline preview 也不再持久化。
- `responseBodyLoggingEnabled=false` 时，调用详情读取响应 body、异常 drawer 与历史回填链路必须返回既有 unavailable/fallback 语义，而不是 500 或“缺文件即损坏”语义。
- `/v1/responses/compact` 继续视为 `Compact`；不把它改名成 `V1`，也不把 `/v1/responses` 内的 V2 语义挤占到 endpoint 字段。
- `/v1/responses` 请求体含 `context_management[type=compaction][compact_threshold]` 时，运行态记录必须写入 `compactionRequestKind="remote_v2"`，且不依赖 request body raw logging。
- `/v1/responses` 终态只有在响应中实际检测到 compaction item 时才写入 `compactionResponseKind="remote_v2"`；“请求启用了 V2 但响应未触发”不得在终态列表误显示为 `远程压缩V2`。
- `imageIntent` 对外合同固定为四态：`"yes" | "direct_image" | "no" | "unknown"`；缺字段历史记录继续返回 `null` / 前端显示 `—`，本次不做历史 backfill。
- `/v1/responses` 请求若由 `gpt-image-*`、`image_generation` 或等价图片工具信号触发，必须持久化 `imageIntent="yes"`；`/v1/images/generations|edits` 必须持久化 `imageIntent="direct_image"`。
- `requestBodyLoggingEnabled=false` 时，`compactionRequestKind` 与 `imageIntent` 仍必须稳定落库并对外可见，不能依赖 request raw body 后读。
- 公开模型展示合同固定为：主显示值采用 `responseModel ?? model ?? requestModel`；只有在 `requestModel` 与 `responseModel` 同时存在、且忽略空白/大小写并按 dated alias/base-model 归并后仍不一致时，才显示“上游改路由”的差异图标。
- 调用详情必须固定展示“请求模型 / 响应模型”两个字段；旧记录若只有历史 `model` 字段，则回填到“响应模型”，请求模型显示 `—`。

### SHOULD

- 历史/非代理记录缺字段时前端统一展示 `—`，不抛错。
- 不影响 SSE 通道协议与统计接口行为，仅将字段名由 `codexSessionId` 变更为 `promptCacheKey`。

### COULD

- 后续按需增加“导出详情”或独立详情页。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 请求进入 `/v1/chat/completions` 或 `/v1/responses` 采集路径时，后端提取 IP 与 prompt cache key，并随 payload 一并落库。
- `/api/invocations` 通过 `json_extract(payload, ...)` 投影扩展字段，不依赖新增列。
- 前端表格默认显示关键字段（token/cost/latency），用户展开后看到请求元信息与阶段耗时明细。
- 前端展开详情时隐藏 `source` 行，避免把来源分类误读成出站代理。
- `/api/invocations/{invoke_id}/pool-attempts` 读取 `pool_upstream_request_attempts.proxy_binding_key_snapshot` 并作为 `proxyBindingKeySnapshot` 返回。
- 号池详情中，真实上游尝试与合成终态记录分开展示。`budget_exhausted_final` 或 `sameAccountRetryIndex <= 0` 仅作为号池终态说明，不作为普通尝试卡片展示，不显示同账号重试序号或阶段耗时。
- 启动阶段执行历史回填：读取 `request_raw_path` 指向的原始请求 JSON，提取 `prompt_cache_key` 后写回 payload。
- Settings 页面在现有 proxy card 内新增两个独立开关，文案明确区分“请求 body 记录”与“响应 body 记录”，并说明关闭仅影响新记录，旧记录继续走 retention。
- 关闭 request body logging 时，请求原文不会进入新的异步 raw writer；关闭 response body logging 时，响应原文异步 writer 与详情页 inline preview 同时关闭。

### Edge cases / errors

- 若 `x-forwarded-for` 首值不可解析，则回退到下一级来源，不中断请求。
- 若 prompt cache key 候选键全部未命中，返回 `null` 并在前端显示 `—`。
- 若阶段耗时缺失（旧记录），前端逐项显示 `—`。
- 若号池达到不同账号尝试上限，前端应明确说明终态记录未发起新的上游请求，并可保留上一失败账号与上一错误状态作为诊断上下文。
- 当 body logging 开关关闭导致新记录没有 raw 路径或 preview 时，详情页、回填与异常查看都要把它当作“未保留 body”，不是“raw 文件丢失”。

## 接口契约（Interfaces & Contracts）

### `GET /api/invocations` 记录对象（新增可选字段）

- `requesterIp?: string`
- `promptCacheKey?: string`
- `endpoint?: string`
- `failureKind?: string`

### `GET /api/invocations/{invokeId}/pool-attempts` 尝试对象

- `proxyBindingKeySnapshot?: string | null`

### `GET /api/settings` / `PUT /api/settings/proxy` 新增字段

- `requestBodyLoggingEnabled: boolean`
- `responseBodyLoggingEnabled: boolean`

### `GET /api/invocations` / SSE `records` 记录对象（新增可选字段）

- `compactionRequestKind?: "compact" | "remote_v2" | null`
- `compactionResponseKind?: "compact" | "remote_v2" | null`
- `imageIntent?: "yes" | "direct_image" | "no" | "unknown" | null`
- `requestModel?: string | null`
- `responseModel?: string | null`

### `GET /api/invocations` 记录对象（已存在并沿用）

- `tReqReadMs?`、`tReqParseMs?`、`tUpstreamConnectMs?`、`tUpstreamTtfbMs?`、`tUpstreamStreamMs?`、`tRespParseMs?`、`tPersistMs?`、`tTotalMs?`

## 验收标准（Acceptance Criteria）

- Given 请求携带 `x-forwarded-for` 与 `metadata.prompt_cache_key`，When 请求完成并查询 `/api/invocations`，Then 返回 `requesterIp`、`promptCacheKey`、`cacheInputTokens` 与阶段耗时字段。
- Given 请求无转发头且 body 无 prompt_cache_key，When 请求完成，Then 前端详情对应字段显示 `—` 且页面无错误。
- Given 成功或失败记录，When 用户展开表格详情，Then 可见 endpoint、failureKind 与完整阶段耗时。
- Given 旧记录或 `source=xy` 记录缺扩展字段，When 页面渲染，Then 不崩溃且缺值显示 `—`。
- Given 调用详情记录 `source=xy` 或其他非 proxy 值但缺少 `proxyDisplayName`，When 用户展开详情，Then 不显示 `source` 行且代理字段显示 `—`。
- Given 号池失败尝试存在 `proxyBindingKeySnapshot=fpb_...` 且绑定节点可解析，When 用户展开号池尝试明细，Then 该尝试显示“代理/Proxy”与对应代理显示名，不把完整内部 key 作为主视觉值。
- Given 号池失败尝试存在 `proxyBindingKeySnapshot=fpb_...` 但绑定节点不可解析，When 用户展开号池尝试明细，Then 该尝试显示紧凑 key，完整 key 仅保留在 hover title。
- Given 号池尝试 `proxyBindingKeySnapshot=__direct__`，When 用户展开号池尝试明细，Then 该尝试显示 `Direct`。
- Given 历史 proxy 记录存在 `request_raw_path` 且 payload 缺 `promptCacheKey`，When 服务启动完成，Then 字段被自动回填且不会重复更新已完成记录。
- Given `requestBodyLoggingEnabled=false` 且 `responseBodyLoggingEnabled=true`，When 新代理调用完成，Then invocation 记录保留结构化 payload / stats / timing，但 `request_raw_path` 与新 request raw 文件都不存在。
- Given `requestBodyLoggingEnabled=true` 且 `responseBodyLoggingEnabled=false`，When 新代理调用完成，Then invocation 记录保留结构化 payload / stats / timing，但 `response_raw_path` 为空，且 `raw_response` preview 为空字符串。
- Given 两个开关都关闭，When 新代理调用完成并打开详情，Then Settings 页面保存成功、调用记录仍可查询，且 body 读取接口返回既有 unavailable/fallback 语义而非 500。
- Given `/v1/responses` 请求启用了 remote compaction V2，When 记录处于 `running` 或 `pending`，Then 列表 badge 显示 `远程压缩V2`，详情同时显示原始 endpoint 与 `压缩请求=远程压缩V2`。
- Given `/v1/responses` 请求启用了 remote compaction V2 但响应未触发 compaction，When 记录进入终态，Then 列表 badge 回退为 `Responses`，详情显示 `压缩请求=远程压缩V2`、`压缩响应=—`。
- Given `/v1/responses` 响应出现 `response.output_item.added` 的 compaction item 或 `response.compaction` 负载，When 记录进入终态，Then 列表 badge 显示 `远程压缩V2`，详情显示 `压缩响应=远程压缩V2`。
- Given `/v1/responses` 请求命中图片工具意图，When Records 与 Dashboard 渲染该 invocation，Then 两个列表都显示独立的“图片工具”徽标，且不改写 endpoint badge。
- Given `/v1/images/generations` 或 `/v1/images/edits` 请求完成，When 用户打开详情，Then `图片工具` 字段显示 `direct_image`，同时保留原始 endpoint。
- Given 历史 invocation 缺少 `imageIntent`，When Records 或 Dashboard 渲染，Then 列表不显示图片徽标，详情字段显示 `—`。
- Given 新记录同时携带 `requestModel=gpt-5.4` 与 `responseModel=gpt-5.5`，When Records、InvocationTable 或 Dashboard working conversations 渲染，Then 主模型文本显示 `gpt-5.5`，并在模型 badge 前显示上游路由差异图标。
- Given `requestModel` 与 `responseModel` 仅大小写不同，或仅 dated alias/base-model 归并后等价，When 列表渲染，Then 不显示模型路由差异图标。
- Given 调用详情打开，When 记录存在双模型字段，Then 页面始终分别展示“请求模型 / 响应模型”，且 mismatch 时仅响应模型带差异图标。
- Given 历史记录仅存在 `model`，When 调用详情打开，Then 请求模型显示 `—`，响应模型显示该历史 `model` 值。

### Manual verification

- 启动 backend/frontend 后打开 `/dashboard` 与 `/#/live`，验证新增列与详情展开可用。

## Visual Evidence

- source_type: storybook_canvas
  story_id_or_title: Settings/SettingsPage/Default
  state: proxy body logging toggles
  evidence_note: verifies the Settings page adds independent request body logging and response body logging switches with retention helper copy in the existing proxy settings card.
  image:
  ![Settings body logging toggles](./assets/settings-body-logging-toggles.png)

- source_type: storybook_canvas
  story_id_or_title: Monitoring/InvocationTable/PoolAttemptDetailLifecycle
  state: expanded pool attempt detail
  evidence_note: verifies the invocation detail hides `source` and the pool-attempt card renders a resolved proxy display name from `proxyBindingKeySnapshot`.
  image:
  ![Pool attempt proxy binding detail](./assets/pool-attempt-proxy-binding-storybook.png)
- source_type: storybook_canvas
  story_id_or_title: Records/InvocationRecordsTable/BudgetExhaustedTerminalRecord
  state: expanded pool attempt detail with synthetic terminal record
  evidence_note: verifies seven real pool attempts render as attempt cards while the `budget_exhausted_final` row renders as a separate terminal state with no retry index or timing labels.
  image:
  ![号池终态记录拆分效果](visual-evidence/pool-terminal-state.png)
- source_type: storybook_canvas
  story_id_or_title: Dashboard/WorkingConversationsSection/ConversationHistoryDrawerOpen
  state: dashboard conversation history drawer
  evidence_note: verifies the dashboard conversation detail opens to the full retained call history drawer with no time-range selector, paginates the 316-record retained history snapshot, renders the zoomable and pannable activity chart, keeps records newest-first, and uses the dark floating tooltip surface.
  image:
  ![Dashboard conversation history drawer](visual-evidence/dashboard-conversation-history-drawer.png)
- source_type: storybook_canvas
  story_id_or_title: Monitoring/PromptCacheConversationTable/ShortSameDayDrawerOpen
  state: short same-day conversation history drawer
  evidence_note: verifies the retained-call activity chart uses the conversation's first and latest invocation timestamps instead of expanding the x-axis to the full local day.
  image:
  ![Short same-day conversation chart range](./assets/conversation-history-short-range.png)

- source_type: storybook_canvas
  story_id_or_title: Monitoring/InvocationTable/EndpointBadgeStates
  state: endpoint badge matrix with remote compaction V2 semantics
  evidence_note: verifies `Compact` remains bound to `/v1/responses/compact`, while `/v1/responses` can surface `远程压缩V2` without overwriting the raw endpoint path.
  image:
  PR: include
  ![Invocation endpoint badge states](./assets/invocation-endpoint-remote-v2-storybook.png)

- source_type: storybook_canvas
  story_id_or_title: Monitoring/InvocationTable/EndpointBadgeStates
  state: mixed endpoint + image badge matrix
  evidence_note: verifies `imageIntent=yes|direct_image` renders an independent `图片工具` badge that can coexist with `远程压缩V2`, while legacy rows without `imageIntent` stay badge-free.
  image:
  PR: include
  ![Invocation image tool badge states](./assets/invocation-endpoint-image-signals-storybook.png)

- source_type: storybook_canvas
  story_id_or_title: Dashboard/WorkingConversationsSection/TransportBadgeMixed
  state: dashboard image badge preview
  evidence_note: verifies Dashboard current/previous invocation slots mirror Records image-badge semantics and keep endpoint/path semantics unchanged.
  image:
  PR: include
  ![Dashboard image tool badge preview](./assets/dashboard-image-signals-storybook.png)

- source_type: storybook_canvas
  story_id_or_title: Monitoring/InvocationTable/ModelRoutingMismatch
  state: request/response model mismatch
  evidence_note: verifies the primary model badge follows the response model and adds the routed-model indicator only when normalized request/response models differ.
  image:
  PR: include
  ![Invocation routed model mismatch](./assets/invocation-model-routing-mismatch.png)

- source_type: storybook_canvas
  story_id_or_title: Records/InvocationRecordsTable/LegacyModelOnly
  state: legacy response-model fallback
  evidence_note: verifies legacy records without `requestModel`/`responseModel` still render the historical `model` value as the response-model display while request model degrades to `—`.
  image:
  PR: include
  ![Legacy response model fallback](./assets/invocation-model-routing-legacy.png)

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：上游请求不保证稳定携带 `prompt_cache_key`，仍可能出现正常缺失。
- 开放问题：是否后续在 SQLite 增加独立 `prompt_cache_key` 列（本次不做）。
- 假设：现有代理链路 payload 存储可承载新增上下文字段。
