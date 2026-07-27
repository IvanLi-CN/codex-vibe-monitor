# 调用详情可分享路由与结构化响应体查看器（#dqstf）

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## 背景 / 问题陈述

Dashboard / Live / Records 三处调用详情曾经以“摘要字段 + 局部异常补充”的方式拼接，存在三个问题：

- 详情缺少最高价值的工作流信息，无法快速回答“这次调用最终怎么裁定、总共尝试了几次、最终落到了哪个账号”。
- 数据来源不透明。调用级记录、池化尝试、异常响应体分散在不同接口和不同存储表里，界面无法解释“这些字段来自哪里，哪些是重建结果”。
- 异常响应体直接放入 `pre` 后，超长 JSON、NDJSON 或 SSE 行还可能扩大 drawer 的内在宽度，造成页面级横向滚动，并缺少结构化排障能力。

## 目标 / 非目标

### Goals

- 为 Dashboard 调用详情提供稳定、可编码、可分享且刷新可恢复的路由。
- 在调用详情顶部优先展示最高价值的工作流摘要，包括调用 ID、短对话 ID、总用时、最终结果、尝试次数与最终账号。
- 以统一时间线暴露完整工作流：真实出站调用保留 `路由决定 -> 等待 -> 每次尝试 -> 最终系统裁定/成功收口`；pre-dispatch 终态统一收口为 `路由决定 -> 最终系统裁定`。
- 让详情加载只依赖稳定 `invokeId`，不依赖当前 Dashboard 卡片仍在内存中。
- 自动识别 JSON、严格 NDJSON 与 SSE transcript，并提供高亮、折叠和键盘可操作的树视图。
- 让纯文本、损坏内容和超长无空格内容自动换行，不再撑宽 drawer 或页面。
- 为超大 payload 提供显式的手动结构化入口，避免默认重解析阻塞 UI。

### Non-goals

- 不要求历史数据立刻 100% 回填为完整时间线；旧记录允许通过现有 invocation + attempt 数据进行 best-effort 重建。
- 不改变原始 request body 的保留策略；调用级 raw request payload 仍然是唯一必须保留的完整 request 原文来源。
- 不提供 JSON 编辑、搜索替换、下载或 schema 校验。
- 不重构账号详情、Prompt Cache 或 Records 的现有 URL 契约。

## 范围

- Dashboard route、调用卡片打开行为、详情 drawer 加载与关闭行为。
- Live / Records / Dashboard 共用的调用详情面板。
- 调用详情共享响应体区域及新的结构化 payload viewer。
- 工作流详情聚合接口，以及为 timeline / 尝试摘要提供的 SQLite 字段补充。
- 对应 unit、route、Storybook 与视觉证据。

## 功能与接口契约

### 可分享路由

- canonical route 为 `#/dashboard/invocations/:invokeId`，path parameter 必须使用 URI encoding。
- 从 Dashboard 卡片打开详情使用 history push；关闭按钮明确导航到 `#/dashboard`。
- 浏览器后退必须从详情返回 Dashboard；直接打开分享 URL 后关闭不得依赖 history back。
- 未知或不可加载的 `invokeId` 保留 drawer shell、错误说明与关闭动作，不静默跳转。
- `current/previous`、对话序号等卡片上下文可以在同一次打开中显示，但不得成为直达 URL 的恢复依赖。

### 工作流摘要与时间线

- 调用详情顶部必须先展示工作流 hero 信息，至少包含：调用 ID、短对话 ID、总用时、最终结果、尝试次数、最终账号。
- 原始 `prompt_cache_key` 必须作为二级上下文暴露，但不应抢占主视觉层级。
- `attempt` 语义固定为“已真实开始向上游 dispatch”；不得再把本地裁定、号池预检失败或预算终态伪装成 Attempt。
- 时间线顺序固定为：辅助块（如路由决定、等待）在前，随后是每次真实尝试；如果最终调用失败，则在最后一次真实尝试之后补充一个系统裁定块。对于没有真实 dispatch 的 pool 终态，时间线必须只包含 `路由决定` 与 `系统裁定`。
- 系统裁定块表示“最终返回给调用方的响应”，需要能够暴露下游状态、失败分类与返回体可用性；若响应体可读，则允许在块内切换到响应体视图。
- 尝试块与辅助块默认使用概览卡片呈现；页面同一时刻只展开一个时间线块，展开后的详情区域也只保留一个激活的子分区。各分区切换都应以次级操作呈现，不得让切换控件抢占主视觉位。
- `routingDecision` 的详情结构固定为 `请求 / 请求头 / 请求体` 三个分区：
  `请求` 展示 route/request/account/client/compression/body-capture 的结构化摘要；
  `请求头` 只展示请求头快照；
  `请求体` 复用调用级 request-body 读取路径。
- 成功、失败、池化、直连与“无尝试表行”的直通调用都必须走同一套工作流详情接口；仅当当前真相可以确认发生过真实出站但缺失 attempt 行时，才允许合成 synthetic attempt。
- 历史错误数据允许在接口聚合层做无迁移渲染纠偏：对 terminal pseudo-attempt，界面必须折叠为 `路由决定 + 系统裁定`，不得继续暴露假 Attempt。

### 数据来源与持久化契约

- `codex_invocations` 是调用级主记录，继续承载完整 request/response raw body 路径，以及 share route / hero 所需的调用级上下文。
- `pool_upstream_request_attempts` 只承载真实开始向上游发送的尝试；本地裁定、号池预检失败、路由阻断与其他未出站终态不得再新写入该表。
- 尝试级结构化详情允许通过 `request_summary_json` / `response_summary_json` 保存，必要时可回退为运行时重建。
- 除尝试外的时间线动作允许以 `timeline_json` 保存在 `codex_invocations` 上；缺失时，工作流详情接口必须基于调用级记录与尝试记录进行 best-effort reconstruction，并显式标记是否为 partial / reconstructed。
- 当前契约下，请求体完整原文只保证在调用级记录存在；尝试级接口默认暴露结构化摘要，而不是再次复制完整 request body。
- 每次真实号池 upstream attempt 都独立持久化 `response_raw_*` 元数据、原始响应文件与 `response_content_encoding`；成功、HTTP 失败、SSE `response.failed`、流中断及全链失败都按 `attempt_public_id` 精确绑定，不得从其他 attempt 回退。
- 响应体回放 API 使用 `invocation id + attempt_public_id` 精确读取当前 attempt；历史缺失 attempt raw 时保持 unavailable，历史最终 attempt 仍可兼容读取调用级 raw body。
- 运行时重建尝试响应摘要时，attempt 级 raw metadata/encoding 优先；调用级 `response_raw_*` 只允许作为历史最终 attempt 的兼容回退。
- 本地生成的终态裁定响应必须复用单一共享 envelope，同时驱动实际 HTTP 下游返回与调用级持久化；`systemFinalFailure.responseBody` 必须回放真实下发 body，不得再落 `"{}"`、`missing_body` 等占位假空体，除非历史记录从未持久化真实 body。

### Payload 识别与渲染

- 识别顺序固定为完整 JSON、严格 NDJSON、SSE transcript、纯文本回退。
- NDJSON 只有在每个非空行都能独立解析为 JSON 时成立，避免把普通日志误判成结构化内容。
- SSE 以空行分隔 event block；识别 `event`、`id`、`retry` 与 `data` 字段。`data` 合并后若为 JSON，则使用树视图，否则按文本显示。
- 结构化树使用 `react-json-view-lite`，支持键盘展开/折叠，并匹配现有 light/dark semantic tokens。
- 小型 JSON 默认展开两层；较大的 JSON、NDJSON 与 SSE 默认只展开根节点或逐条 event。
- UTF-8 体量超过 `1 MiB` 时默认显示纯文本与手动结构化操作；只有用户触发后才解析。

### 尺寸与滚动

- drawer shell、body、section、flex/grid child 与 payload container 必须具备 `min-width: 0` / `max-width: 100%` 约束。
- 结构化内容限制最大高度，并在自身容器内支持横向和纵向滚动。
- 纯文本使用保留换行的自动换行策略，并允许任意长 token 断行。
- drawer 继续只有一个页面级纵向内容滚动体；内部滚动只用于 payload inspector 的有界内容。

## 验收标准

- 点击调用后 URL 变为 `#/dashboard/invocations/<invokeId>`，刷新和粘贴 URL 均恢复同一详情。
- 关闭按钮回到 `#/dashboard`，浏览器前进/后退行为符合 route history。
- 顶部 hero 区优先呈现调用 ID、短对话 ID、总用时、最终结果、尝试次数与最终账号，且原始 `prompt_cache_key` 在次级上下文可见。
- 新的 pre-dispatch pool 失败时间线只展示 `路由决定 + 系统裁定`，`hero.timelineAttemptCount = 0`。
- 真实出站调用仍按顺序展示辅助块、尝试块和失败场景下的系统裁定块；同一时刻仅展开一个块，展开区保持紧凑。
- 每次真实 upstream attempt 都能按自身 `attempt_public_id` 回放响应体；四次 `upstream_response_failed` 的失败链路中，四条响应体互不串位。
- 全链失败必须写入调用级终态记录；账号请求记录与调用详情都能看到对应 attempt 的状态、编码、字节数与响应体可用性。
- 点击尝试块后，默认展示概览，并可通过次级操作进入请求 / 响应；点击路由块后，默认展示概览，并可通过次级操作进入 `请求 / 请求头 / 请求体`；点击失败裁定块后，默认展示概览，并可通过次级操作进入 `裁定 / 返回体`。
- 对包含重试的调用，每个 Attempt 的响应详情必须显示自身响应体大小、响应头和 lazy response-body 内容；如果历史记录没有该 attempt 原文，应明确显示 `attempt_response_body_not_captured` 的 unavailable 状态。
- 历史 pseudo-attempt 在纠偏后不得再显示为 Attempt；若旧记录从未持久化真实 body，允许继续显示 unavailable。
- JSON、NDJSON、SSE JSON data 均显示可折叠、高亮、键盘可操作的结构化视图。
- 纯文本、解析失败内容和超长无空格文本保持可读并自动换行。
- 超过 `1 MiB` 的 payload 默认不解析，手动操作后才进入结构化视图。
- 桌面 drawer 与移动 bottom sheet 均无页面级横向溢出。
- Storybook 覆盖 hero + 时间线主路径、展开后的尝试详情，以及瞬态待落盘状态。

## Visual Evidence

页面级绑定场景：mock-only Web Demo `#/dashboard/invocations/demo-invocation-9002?demoScene=attention&demoViewport=desktop`，固定桌面网页视口 `1440x900`。

视口证据元数据：`requested_viewport=1440x900`，`viewport_strategy=explicit-browser-viewport`，`capture_scope=browser-viewport`。

组件级回归场景：Storybook `Invocations/InvocationWorkflowDetailPanel/BlockedPoolWorkflow`。

专用 unavailable 场景：Storybook `Invocations/InvocationWorkflowDetailPanel/BlockedPoolWorkflowMissingArchivedRequestBody`。

- Web Demo 路由级证据必须覆盖真实 Dashboard 抽屉，而不是独立组件画布。
- Story id: `invocations-invocationworkflowdetailpanel--blocked-pool-workflow`
- Story id: `invocations-invocationworkflowdetailpanel--blocked-pool-workflow-missing-archived-request-body`
- 视觉证据覆盖四种状态：
  - Dashboard 路由 unavailable 态：`demo-invocation-9002` 的 attempt 卡片展开 `请求体` 后，必须同时看到 `qPvNNAK8` attempt 标识、请求/响应指标条、HTTP 请求压缩、`归档 未存档`，以及 `请求体不可用：该记录没有保留可展示的载荷。`
  - 概览态：时间线只包含 `路由决定 + 系统裁定`，证明 pre-dispatch 失败不再渲染假 Attempt。
  - 路由详情态：路由块展开后可切换 `请求 / 请求头 / 请求体` 三个分区，证明 request summary、header snapshot 与调用级 request body 回放都可直接查看。
  - 裁定返回体态：系统裁定块展开后切换到 `返回体`，证明详情页显示的是实际下发给调用方的 JSON body，而非 `missing_body` 或空占位。
  - unavailable 回放态：`请求体` lazy fetch 完成后，界面必须从 loading 收口到人类可读提示 `该记录没有保留可展示的载荷。`，而不是无限 loading 或直接暴露内部 `missing_body` reason。

PR: none

视觉证据=存在
视觉证据目标源=ui_demo
证据落盘=已落盘：`assets/attempt-response-body-demo.png`
证据绑定sha=3fadab7e
空白裁剪=无需裁剪（固定 1440x900 页面边界存在半透明抽屉背景，裁剪器按 ambiguous_border 保持原图）
聊天回图=已展示
证据说明：attempt `qPvNNAK8` 展开后显示 `captureSource=attempt_raw_file`、独立大小/编码元数据与实际 JSON 响应体；页面来自 mock-only、免登录 Demo。

![Attempt-scoped response body replay](assets/attempt-response-body-demo.png)

## References

- `docs/specs/hnu7b-mobile-first-navigation-and-overlays/SPEC.md`
- `docs/specs/ykhfu-web-demo/SPEC.md`
- `docs/solutions/workflow/mock-only-web-demo-runtime.md`
