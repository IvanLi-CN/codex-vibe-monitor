---
title: Mock-only Web demo runtime
module: frontend-delivery
problem_type: reproducible-product-demo
component: React/Vite demo runtime
tags:
  - frontend
  - msw
  - pages
  - ui-demo
status: active
related_specs:
  - docs/specs/ykhfu-web-demo/SPEC.md
---

# Mock-only Web demo runtime

## Context

产品需要公开或 owner-facing 的完整 Web 演示，但真实后端、身份和设备写入既不稳定也不适合公开。组件 Storybook 不能验证正式路由、传输、SSE 与写入回显的组合行为。

## Resolution

- 使用构建时 runtime flag 将 live 与 demo 分开；路由只表达产品页面，不能决定连接真实服务还是 mock。
- 在 React render 前启动浏览器 MSW worker，并对所有应用 HTTP/SSE 提供 handlers；未处理的应用请求 fail closed。
- 用一个内存 model 驱动 fixtures、写入和 SSE，reset 后回到确定性 seed；secret 输入只保留脱敏结果。
- 将公开静态 demo 作为 Pages 子目录产物，向 Vite 和 MSW worker 显式传入部署 base。
- Storybook 继续覆盖 Inspector 等独立 fragment，完整页面证据从 mock-only demo 取得。

## Symptoms

- 将 existing Playwright fixtures 与 demo MSW 运行在同一个 Vite runtime 时，worker 会优先返回 demo handler 数据，导致 fixture-specific assertions 失去控制。
- 在 node test environment 直接 import browser SSE handler 时，缺少 `EventSource` 会阻止 HTTP handler 单测启动。

## Root cause

- Service worker 是浏览器请求的第一层 mock，不能与依赖 Playwright route fixture 的 regression 假定为可随意互换。
- MSW 的 `sse()` 需要浏览器 EventSource，而 node HTTP test server 只需要 REST handlers。

## Reuse notes

- 保留既有 fixture E2E 的 Vite runtime，另起 demo server 并在同一 required context 内传入 `E2E_BASE_URL`；不要为了增加 demo 验证而改写既有 fixture 数据。
- 将 browser-only SSE handlers 放在独立模块，HTTP handlers 单独导出供 `setupServer` 使用；browser worker 再合并两者。

## Guardrails / Reuse notes

- 公开 demo 不得提供连接真实 API、OAuth、数据库或硬件的环境变量、URL 开关或回退路径。
- Fixtures 必须保持生产 API 的多阶段读取语义，避免演示把 production 的 hydrate、loading 或 SSE 边界伪装成不存在。
- 分享 URL 只保存 route、scene 与非敏感 UI state；不要保存表单值、secret 或完整 action payload。

## References

- `docs/specs/ykhfu-web-demo/SPEC.md`
- `docs/solutions/performance/rollup-first-upstream-account-usage.md`
