# 调用详情可分享路由与结构化响应体查看器 - Implementation

## Current State

- Canonical spec: `docs/specs/dqstf-invocation-detail-routing-payload-viewer/SPEC.md`
- Implementation summary: 调用详情已升级为统一的工作流视图；Dashboard / Records / Live 共用 hero + timeline + structured detail panel
- Branch: `th/invocation-detail-timeline`
- Base: `origin/main@c744248d5bc7222eb5573e1a2d623fe96be8da17`

## Implemented Coverage

- Dashboard 新增 `/dashboard/invocations/:invokeId` nested route；打开、刷新、直达和关闭均由 route 驱动，卡片 selection 只保留临时上下文。
- `DashboardInvocationDetailDrawer` 可只凭 `invokeId` 通过既有 records API 补取完整记录，未知记录继续呈现可关闭的 empty/error 状态。
- 新增 `GET /api/invocations/:id/workflow-detail` 聚合接口，输出 hero、timeline、partial/reconstructed 状态，以及尝试级 request/response 结构化摘要。
- `codex_invocations` 新增 `timeline_json` 字段；`pool_upstream_request_attempts` 新增 `request_summary_json` / `response_summary_json` 字段，用于承载工作流时间线和尝试级结构化快照。
- 工作流详情接口在没有 attempt 行时可合成 synthetic attempt，并在缺失 `timeline_json` 时根据调用级记录和尝试表进行 best-effort reconstruction。
- `InvocationWorkflowDetailPanel` 作为新的共享详情组件，统一服务于 Dashboard、Records 和 Live 三个入口；顶部 hero 区优先展示调用 ID、短对话 ID、总用时、最终结果、尝试次数和最终账号。
- hero 区进一步收敛为单一的 `Rich Structured Snapshot`，把调用身份、关键结果指标和排障摘要放进同一首屏，避免旧版双栏高度错位和低价值字段抢视线。
- 时间线块支持 `路由 / 等待 / 尝试 / 最终裁定` 四类节点；页面同一时刻只展开一个时间线块，块内只保留一个激活子分区。
- 时间线块统一改为 overview-first 交互：默认先展示人类可读 overview，请求 / 响应 / 原始 JSON / 响应体都通过右上角次级操作进入，不再占主视觉位。
- 尝试块进一步改成显式子页面目录：默认展开首个尝试块，并直接展示 `概览 + 7 个尝试子详情页` 的入口矩阵，避免请求 / 响应细节继续藏在两级按钮后面。
- `StructuredPayloadViewer` 使用 `react-json-view-lite@2.5.0`，识别 JSON、严格 NDJSON 与 SSE transcript；纯文本自动换行。
- 超过 `1 MiB` 的 payload 默认显示原文，用户显式触发后才进行结构化解析。
- drawer section、错误文本、原文与 structured inspector 补齐 width/overflow contract；树视图使用有界双向滚动。
- Storybook 新增 `Invocations/InvocationWorkflowDetailPanel`，覆盖失败工作流概览态，以及请求头 / 请求体 / 响应头 / 响应体四个详情分区的稳定视图。

## Verification

- `cargo check -q`: passed。
- `cd web && bun run build`: passed。
- `cd web && bun run test -- --run src/features/invocations/InvocationWorkflowDetailPanel.test.tsx`: 1 file passed，2 tests passed。
- `cd web && bun x storybook build --output-dir /tmp/cvm-storybook-workflow-detail-final4`: passed。
- 本地 Playwright 截图验证了 Storybook `FailedPoolWorkflow` 的概览态、尝试子详情目录，以及请求头 / 请求体 / 响应头 / 响应体几个核心详情视图，并已把证据写入 spec `## Visual Evidence`。

## Remaining Delivery Gate

- 历史的 Dashboard / Records 详情测试和故事夹具仍保留旧接口假设；当前实现已通过新的工作流面板测试与生产构建验证，但若要让整组 legacy 测试恢复 green，需要继续把它们迁到 `fetchInvocationWorkflowDetail` 契约。
