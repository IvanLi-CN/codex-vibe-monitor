# 上游账号列表紧凑化改版 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/62uhg-upstream-account-roster-compact/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-16: 创建增量规格，冻结上游账号列表紧凑化的布局目标、信息层级与验证口径。
- 2026-03-16: 列表重构为四段式紧凑布局，账号区收敛为两行字段值 + 一行标记带，`5 小时` / `7 天` 合并为同列双窗口摘要。
- 2026-03-16: 补齐 `UpstreamAccountsTable` 组件测试、账号页集成测试以及 Storybook 长名称 / 多标签场景。
- 2026-03-16: 本地验证通过：`cd web && bun run test`、`cd web && bun run build`、`cd web && bun run lint`；浏览器 smoke 通过 Storybook `CompactLongLabels` 场景确认 `1280px` 与窄视口下均无横向溢出，详情入口可正常打开。
- 2026-03-16: 根据实际预览反馈，进一步缩短窗口行标签并上调窗口列宽占比，优先保证“使用量 + 下次重置”文本在桌面宽度下尽量完整显示。
- 2026-03-16: 根据最新界面反馈，移除账号列表行中的分组名显示，账号区仅保留账号名与下方标记带。
- 2026-03-16: 根据最新界面反馈，为同步列补充“上次调用时间”，并扩展上游账号汇总接口返回最近一次调用时间字段。
- 2026-03-16: 为最近调用时间聚合补充 `codex_invocations` 表达式索引，并将列表返回的 `lastActivityAt` 统一序列化为 UTC ISO，避免浏览器按本地时区误解析。
- 2026-03-16: 调整标记带折叠策略与 `+N` 提示交互，继续保留前 `3` 个 tags，同时让折叠提示支持悬浮与键盘聚焦读取。
- 2026-03-17: 快车道收敛完成，PR visual evidence 已同步，review-loop 对最新 PR head 收敛完成，进入直接合并阶段。
- 2026-03-25: 修复 `CompactLongLabels` 场景里 `Compact 不支持` 与 tags `+1` 的 `1.5px` 垂直下沉；把 `title` 直接挂回 badge 本体，移除额外 `span > Badge(div)` 包裹，并补齐 Storybook play DOM 对齐断言、组件结构回归与新一轮暗色主题视觉证据。
- 2026-03-25: 根据最新界面反馈，列表中的 `Compact 可用` 标记不再显示；仅保留 `Compact 不支持` 作为异常提示，支持状态继续保留在详情抽屉字段中。
- 2026-03-27: 统一缺失窗口占位语义；当 `secondaryWindow == null` 时，列表次级窗口行保留标签但把使用值、重置时间和百分比统一切到弱一级 ASCII `-`，并补充 Storybook / Vitest / 页面集成验证与新的 mock-only 视觉证据。
- 2026-03-27: 根据后续验收反馈，进一步收紧无周限额场景：当 `secondaryWindow == null` 且 `secondaryLimit === null` 时，列表不再显示误导性的 `7D` 标签，仅保留弱一级 `-` 占位与空轨道。
