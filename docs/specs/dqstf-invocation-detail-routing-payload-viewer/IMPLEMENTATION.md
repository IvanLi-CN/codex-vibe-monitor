# 调用详情可分享路由与结构化响应体查看器 - Implementation

## Current State

- Canonical spec: `docs/specs/dqstf-invocation-detail-routing-payload-viewer/SPEC.md`
- Implementation summary: 本地实现、定向验证与桌面/移动视觉证据均已完成
- Branch: `th/invocation-detail-route-viewer`
- Base: `origin/main@d0480696be2e385b268175aff47bfc383d370271`

## Implemented Coverage

- Dashboard 新增 `/dashboard/invocations/:invokeId` nested route；打开、刷新、直达和关闭均由 route 驱动，卡片 selection 只保留临时上下文。
- `DashboardInvocationDetailDrawer` 可只凭 `invokeId` 通过既有 records API 补取完整记录，未知记录继续呈现可关闭的 empty/error 状态。
- `StructuredPayloadViewer` 使用 `react-json-view-lite@2.5.0`，识别 JSON、严格 NDJSON 与 SSE transcript；纯文本自动换行。
- 超过 `1 MiB` 的 payload 默认显示原文，用户显式触发后才进行结构化解析。
- drawer section、错误文本、原文与 structured inspector 补齐 width/overflow contract；树视图使用有界双向滚动。
- Storybook 提供 JSON、NDJSON、SSE、纯文本和超大 payload 状态；mock-only Web Demo 提供可直达的 attention scene SSE fixture。

## Verification

- `cd web && bun run test -- --reporter=dot src/demo/runtime.test.ts src/features/invocations/structuredPayload.test.ts src/features/dashboard/DashboardInvocationDetailDrawer.test.tsx src/demo/handlers.test.ts`: 4 files passed，25 tests passed。
- `cd web && bun run build`: passed。
- `cd web && bun run demo:build`: passed。
- Chrome Control mock-only Web Demo proof: `http://127.0.0.1:34490/#/dashboard/invocations/demo-invocation-9002?demoScene=attention` 与 `&demoViewport=mobile390` 均可稳定打开，并已把 `异常响应体 / SSE 事件流` 证据写入 spec `## Visual Evidence`。

## Remaining Delivery Gate

- 无。视觉证据门禁已清除，可进入 push、PR 与 merge path。
