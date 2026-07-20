# 调用详情可分享路由与结构化响应体查看器 - Implementation

## Current State

- Canonical spec: `docs/specs/dqstf-invocation-detail-routing-payload-viewer/SPEC.md`
- Implementation summary: 调用详情继续使用统一工作流视图；当前真相已收紧为 `attempt = 真实开始向上游 dispatch`，pre-dispatch pool 终态统一表现为 `路由决定 + 系统裁定`，本地裁定返回体回放真实下游 body。
- Branch: `th/fix-invocation-payload-loading`
- Base: `main@1bd0ae7d63ce9123a8c8311c934c842ae99dcffa`

## Implemented Coverage

- Dashboard 新增 `/dashboard/invocations/:invokeId` nested route；打开、刷新、直达和关闭均由 route 驱动，卡片 selection 只保留临时上下文。
- `DashboardInvocationDetailDrawer` 可只凭 `invokeId` 通过既有 records API 补取完整记录，未知记录继续呈现可关闭的 empty/error 状态。
- `DashboardInvocationDetailDrawer` 在首轮 lookup 只拿到瞬态 `id <= 0` 记录时，会继续按 `invokeId` 轻量重查，直到异步 SQLite 落盘后的持久化记录可见，避免终态 `HTTP 400` / `failed` 调用长期卡在“调用未落盘”。
- 新增 `GET /api/invocations/:id/workflow-detail` 聚合接口，输出 hero、timeline、partial/reconstructed 状态，以及尝试级 request/response 结构化摘要。
- `codex_invocations` 新增 `timeline_json` 字段；`pool_upstream_request_attempts` 新增 `request_summary_json` / `response_summary_json` 字段，用于承载工作流时间线和尝试级结构化快照。
- 工作流详情接口在没有 attempt 行时可合成 synthetic attempt，并在缺失 `timeline_json` 时根据调用级记录和尝试表进行 best-effort reconstruction。
- 工作流详情聚合层现在会识别历史 pre-dispatch pseudo-attempt 形态，并在不做数据库回写迁移的前提下把它们折叠成 `路由决定 + 系统裁定`；`hero.timelineAttemptCount` 与时间线 Attempt 数都只统计真实出站。
- `InvocationWorkflowDetailPanel` 作为新的共享详情组件，统一服务于 Dashboard、Records 和 Live 三个入口；顶部 hero 区优先展示调用 ID、短对话 ID、总用时、最终结果、尝试次数和最终账号。
- hero 区进一步收敛为单一的 `Rich Structured Snapshot`，把调用身份、关键结果指标和排障摘要放进同一首屏，避免旧版双栏高度错位和低价值字段抢视线。
- 时间线块支持 `路由 / 等待 / 尝试 / 最终裁定` 四类节点；页面同一时刻只展开一个时间线块，块内只保留一个激活子分区。
- 时间线块统一改为 overview-first 交互：默认先展示人类可读 overview，请求 / 响应 / 原始 JSON / 响应体都通过右上角次级操作进入，不再占主视觉位。
- 尝试块进一步改成显式子页面目录：默认展开首个尝试块，并直接展示 `概览 + 7 个尝试子详情页` 的入口矩阵，避免请求 / 响应细节继续藏在两级按钮后面。
- 路由块详情固定为 `请求 / 请求头 / 请求体` 三个分区；其中 `请求体` 直接复用调用级 request-body 读取路径，不在 attempt 表复制 raw body。
- 调用详情的 lazy payload loader 现在统一使用 request/response sequence guard：点击 `请求体` / `响应体` 后，异步完成必须以最近一次请求为准收口到 `ready` 或 `error`，不能再因 effect 自清理把已成功返回的结果永久丢成 `loading`。
- mock-only Web Demo 现在为 `demo-invocation-9002` 补齐 `/api/invocations/:id/workflow-detail`、`/request-body` 和 `/response-body` 路由级夹具；Dashboard 可直接从分享路由回放真实 attempt 卡片，并稳定复现“请求体未存档但不再卡 loading”的 owner-facing 证据面。
- 本地生成的终态错误响应改为复用共享 envelope，同时驱动 HTTP 下游返回与 `ProxyCaptureRecord` 持久化；`systemFinalFailure.responseBody` 对 503/429/同类本地裁定现在回放真实 JSON body，不再落 `"{}"` / `missing_body` 假空体。
- pre-dispatch pool 失败、budget terminal、websocket pre-upstream owner-guard 等本地终态不再前向写入 `pool_upstream_request_attempts`；真实出站调用的 attempt 主路径保持不变。
- `StructuredPayloadViewer` 使用 `react-json-view-lite@2.5.0`，识别 JSON、严格 NDJSON 与 SSE transcript；纯文本自动换行。
- 超过 `1 MiB` 的 payload 默认显示原文，用户显式触发后才进行结构化解析。
- drawer section、错误文本、原文与 structured inspector 补齐 width/overflow contract；树视图使用有界双向滚动。
- Storybook `Invocations/InvocationWorkflowDetailPanel` 现同时覆盖真实出站失败路径、`BlockedPoolWorkflow` 的 pre-dispatch 阻断路径，以及 `BlockedPoolWorkflowMissingArchivedRequestBody` 的 lazy unavailable 回放路径；后者专门锁定“请求成功返回 unavailable，但界面必须结束 loading 并隐藏内部 reason code”的回归面。

## Verification

- `cargo fmt --all`: passed。
- `cargo check`: passed。
- `cargo test failover_preserves_assigned_account_when_sticky_owner_is_preflight_blocked -- --nocapture`: passed。
- `cargo test capture_target_pool_route_timeout_surfaces_blocked_policy_terminal -- --nocapture`: passed。
- `cargo test websocket_prepare_rate_limited_owner_returns_owner_unavailable -- --nocapture`: passed。
- `cd web && bun run test -- src/demo/handlers.test.ts`: 1 file passed，10 tests passed。
- `cd web && bun run test -- src/features/invocations/InvocationWorkflowDetailPanel.test.tsx`: 1 file passed，6 tests passed。
- `cd web && bun run test src/features/dashboard/DashboardInvocationDetailDrawer.test.tsx`: 1 file passed，9 tests passed。
- `cd web && bun run demo:build`: passed。
- `cd web && bun run build-storybook`: passed。
- Chrome + Storybook `BlockedPoolWorkflowMissingArchivedRequestBody` 验证：`请求体` lazy fetch 最终显示 `该记录没有保留可展示的载荷。`，且页面文本不再包含内部 `missing_body` reason。
- Chrome + mock-only Web Demo Dashboard 路由验证：`#/dashboard/invocations/demo-invocation-9002?demoScene=operational&demoTheme=dark` 内的 attempt `qPvNNAK8` 展开 `请求体` 后，界面显示 `请求体不可用：该记录没有保留可展示的载荷。`，且保留请求/响应指标、压缩信息与 `归档 未存档`。
- 本地截图验证已补充 `workflow-detail-dashboard-attempt-request-body-unavailable.png`，并写回 spec `## Visual Evidence` 作为页面级最终证据；Storybook 截图继续只承担组件级回归证据。

## Remaining Delivery Gate

- 当前任务范围内无剩余交付 gate；更广泛的 legacy 故事夹具迁移仍是独立清理项，不阻塞本次语义修正交付。
