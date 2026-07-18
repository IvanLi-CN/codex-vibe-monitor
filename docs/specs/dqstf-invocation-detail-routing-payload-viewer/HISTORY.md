# 调用详情可分享路由与结构化响应体查看器 - History

## Migration

- Canonical spec: `docs/specs/dqstf-invocation-detail-routing-payload-viewer/SPEC.md`

## Key Decisions

- 2026-07-14: 使用 `#/dashboard/invocations/:invokeId` 作为 canonical share route，关闭动作固定回到 Dashboard，避免直接链接依赖外部 history。
- 2026-07-14: 结构化识别覆盖 JSON、严格 NDJSON 与 SSE transcript；无法可靠识别时保留纯文本。
- 2026-07-14: 超过 `1 MiB` 的内容默认不做结构化解析，由用户显式触发，以控制主线程风险。
- 2026-07-14: 页面级证据继续使用 mock-only Web Demo，Storybook 只承载 viewer 的可复用状态与交互回归。
- 2026-07-15: 为 mock-only Web Demo 增加 `demoViewport=mobile390` iframe 壳，给移动断点提供可复现、可分享且不依赖浏览器窗口 resize 的稳定证据入口。
- 2026-07-15: 调用详情升级为统一工作流视图。顶部 hero 区固定优先展示调用 ID、短对话 ID、总用时、最终结果、尝试次数和最终账号，避免排障时先被低价值 token 字段淹没。
- 2026-07-15: 时间线 contract 固定为“辅助块在前、尝试居中、失败裁定在后”；失败时必须追加系统裁定块来表示最终返回给调用方的响应，而不是仅依赖最后一次尝试的错误信息。
- 2026-07-15: 数据契约采用 hybrid storage。尝试事实继续保留在 `pool_upstream_request_attempts`，辅助时间线动作允许写入 `codex_invocations.timeline_json`；历史数据缺失时通过聚合接口重建，并显式暴露 `reconstructed/partial` 状态。
- 2026-07-15: 请求体完整原文只保证在调用级记录存在；尝试级详情默认暴露 `request_summary_json` / `response_summary_json` 结构化快照，不在 attempt 表重复存整份 raw body。
- 2026-07-15: 视觉设计收口到 `Rich Structured Snapshot + overview-first timeline`。首屏改为单一快照面板，辅助块默认先展示人类可读概览，原始 JSON / 响应体降为次级操作，避免调用详情退化成日志堆叠器。
- 2026-07-16: 尝试详情入口改为显式子页面目录。展开尝试后默认直接暴露 `时间详情 / 解析请求 / 请求头 / 请求体 / 解析响应 / 响应头 / 响应体` 七个子详情页，不再要求用户先经过“请求详情 / 响应详情”的二级入口。
- 2026-07-18: `attempt` 语义收紧为“真实开始向上游 dispatch”。pre-dispatch pool 终态、本地 owner-guard terminal 与 websocket pre-upstream terminal 不再前向写入 `pool_upstream_request_attempts`。
- 2026-07-18: 历史 pre-dispatch pseudo-attempt 采用聚合层渲染纠偏，不做数据库回写迁移；workflow detail 会把稳定特征命中的旧行折叠成 `路由决定 + 系统裁定`。
- 2026-07-18: 路由块详情 contract 固定为 `请求 / 请求头 / 请求体` 三分区，其中 `请求体` 继续回放调用级原始 request body，而不是复制 attempt-level raw body。
- 2026-07-18: 本地终态错误响应改为复用共享 envelope；HTTP 下游返回与 `ProxyCaptureRecord` 持久化使用同一份 status/headers/body，`systemFinalFailure.responseBody` 因而回放真实裁定 body，不再依赖 `"{}"` / `missing_body` 占位。
