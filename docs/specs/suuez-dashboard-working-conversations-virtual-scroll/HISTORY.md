# Dashboard 工作中对话无限列表、虚拟滚动与增量同步 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/suuez-dashboard-working-conversations-virtual-scroll/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-10: 新建 follow-up spec，冻结“无限列表 + 页面级滚动驱动的按行虚拟化 + compact 分页 API + SSE patch / resync + 头插锚点补偿 + merge-ready 收口”这组决策。
- 2026-04-10: 后端已落地 prompt-cache conversations 分页 / compact 合同与 Rust 回归；Dashboard hook / mapper / section 已切到分页无限列表、页面级滚动驱动的按行虚拟化与局部 patch 路径。
- 2026-04-10: 已新增 hook 级 Vitest 覆盖首屏 compact page、loadMore cursor/snapshotAt、loaded-key patch、unseen-key resync、reconnect resync 与本地 stale prune；组件级测试新增 DOM 子集渲染断言。
- 2026-04-10: Storybook 已补齐 loading / empty / error、mobile 390、wide 1660、virtualized large dataset 与 head-insert anchor compensation 入口；本地 `cargo test`、Dashboard 定向 Vitest、`bun run build`、`bun run storybook:build` 全部通过，视觉证据已落盘并获主人批准继续推进 PR 收敛。
- 2026-04-11: 修复分页 working-conversations 与 full-detail upstream account hydration 的 `total_cost` 数值聚合，把 `COALESCE/SUM` 统一锚到 `REAL 0.0`，并新增 `NULL cost` snapshot pagination Rust 回归，锁住 Dashboard 偶发 `500 total_cost mismatched types` 热修。
- 2026-04-11: Storybook 已刷新 loading / empty / error、mobile 390、wide 1660、virtualized large dataset 与 head-insert anchor compensation 入口，相关视觉证据继续归档在本 spec；本次 `total_cost` 热修复用该 spec，不新增额外视觉证据。
