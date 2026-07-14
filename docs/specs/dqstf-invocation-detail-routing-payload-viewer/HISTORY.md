# 调用详情可分享路由与结构化响应体查看器 - History

## Migration

- Canonical spec: `docs/specs/dqstf-invocation-detail-routing-payload-viewer/SPEC.md`

## Key Decisions

- 2026-07-14: 使用 `#/dashboard/invocations/:invokeId` 作为 canonical share route，关闭动作固定回到 Dashboard，避免直接链接依赖外部 history。
- 2026-07-14: 结构化识别覆盖 JSON、严格 NDJSON 与 SSE transcript；无法可靠识别时保留纯文本。
- 2026-07-14: 超过 `1 MiB` 的内容默认不做结构化解析，由用户显式触发，以控制主线程风险。
- 2026-07-14: 页面级证据继续使用 mock-only Web Demo，Storybook 只承载 viewer 的可复用状态与交互回归。
