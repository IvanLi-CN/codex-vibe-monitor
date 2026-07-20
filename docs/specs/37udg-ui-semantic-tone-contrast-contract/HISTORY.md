# UI semantic tone contrast contract - History

## Change History

- 2026-07-20: 创建 spec，冻结 filled-content vs tone-ink contract、受影响 shared surfaces、Storybook dark evidence 与 source contract test 范围。
- 2026-07-20: 完成 theme token、共享 Badge、InvocationWorkflowDetailPanel、AppLayout/PWA offline chip 迁移，并以 Storybook dark captures + unit/build/source-contract 验证收口。

## Key Decisions

- 将本次修复建成独立 topic spec，而不是回写 `x4v2n`，因为 `x4v2n` 的既有范围显式排除了 badge 与业务状态色。
- 低透明语义底的 shared text 一律走 tone-ink contract；`*-content` 只保留给 filled semantic surface。
- 视觉证据优先使用 Storybook dark scenarios，而不是依赖真实页面偶发态截图。
