# UI surface 对比层级收口 - History

## Change History

- 2026-07-17: 创建 spec，冻结共享 surface vocabulary、Dashboard / Settings / Account Pool 高可见度治理范围、Web Demo 视觉证据口径与文档同步要求。
- 2026-07-17: 完成 `surface-card`、`surface-subtle`、`surface-inset`、`field-surface`、`menu-surface`、`dialog-chrome-surface` 与 destructive callout token 落地；基础 primitive 和目标页面迁移到共享 surface。

## Key Decisions

- 将本次修复建成独立 topic spec，而不是修改 `quhzx-ui-guidelines-system` 的 docs-only 交付语义。
- 视觉证据使用 Web Demo 而非 Storybook，因为当前 Storybook preview 被无关 React Refresh duplicate-symbol 问题阻断。
- 截图证据通过 Codex thread 回传，不把临时截图资产写入仓库。
