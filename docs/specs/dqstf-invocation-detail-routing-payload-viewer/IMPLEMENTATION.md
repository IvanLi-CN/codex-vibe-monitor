# 调用详情可分享路由与结构化响应体查看器 - Implementation

## Current State

- Canonical spec: `docs/specs/dqstf-invocation-detail-routing-payload-viewer/SPEC.md`
- Implementation summary: 本地实现与自动化验证完成，视觉证据待浏览器能力恢复后补齐
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

- `cd web && bun run test -- --reporter=dot`: 111 files passed，1218 tests passed，6 skipped。
- `cd web && bun run test-storybook`: 6 files passed，11 tests passed，52 unsupported stories skipped。
- `cd web && bun run build`: passed。
- `cd web && bun run demo:build`: passed。
- `cd web && bun run build-storybook`: passed。
- changed-files Biome check: passed；全仓 lint 仍包含未改动存量文件的既有 errors/warnings。

## Remaining Delivery Gate

- mock-only Web Demo 已在当前 worktree 的有效端口租约上启动，但 Chrome Control bootstrap 与 chrome-devtools 连接均不可用，无法按仓库规定完成浏览器截图和 overflow measurement。
- 在浏览器能力恢复并补齐 spec `## Visual Evidence` 前，不进入 push、PR 或 `Step 5C Ready`。
