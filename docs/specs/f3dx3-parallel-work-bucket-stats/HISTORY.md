# 并行工作 bucket 统计 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/f3dx3-parallel-work-bucket-stats/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-07: 完成 `GET /api/stats/parallel-work`、固定窗口聚合、Stats 页 section、Storybook docs 与前后端测试；根据主人反馈将并行工作 section 改为按项目既有 segmented toggle 习惯切换窗口显示，同一时刻不再并排展示三个统计。
- 2026-04-07: 按主人反馈把并行工作趋势图改为全宽交互图表，并补上 hover / click 详情浮窗、Storybook 交互覆盖与前端回归验证。
- 2026-04-07: 按主人反馈把窗口选择器移到卡片右上角，并同步刷新 loading / error / empty / populated 布局与 Storybook docs 证据。
- 2026-04-07: 按主人反馈移除卡片内单独的窗口标题与说明，把整段窗口元信息统一折叠进问号气泡提示，并刷新 Storybook docs 证据。
- 2026-04-07: 按主人反馈把问号图标继续移动到右上角选择器旁边，同排显示且不再单独占一行，并刷新 Storybook docs 证据。
- 2026-04-07: 按主人反馈进一步压缩高度，把问号图标与选择器整体上移到 section 标题区右上角，卡片主体直接从指标卡开始渲染，不再浪费额外高度。
- 2026-04-07: 按主人反馈把问号图标改为贴在“并行工作”标题右侧并垂直居中对齐，选择器继续留在标题区右上角。
- 2026-04-07: 按主人反馈去掉 populated 卡片的人工最小高度，收紧底部无意义空白，并刷新 Storybook docs 证据。
- 2026-04-07: 按主人反馈给趋势图补上 X/Y 轴刻度与辅助网格线，并修正首尾时间刻度避免被边界裁切。
- 2026-04-07: review-loop follow-up：对非整点 UTC offset 时区保留请求时区的 `minute7d` 精确统计，同时把历史 `hour30d` / `dayAll` 窗口显式回退到 `Asia/Shanghai` 对齐并补前端提示，避免整个接口 400；同时修正 `useParallelWorkStats` 在 hydration 期间收到 SSE open 后未排队补刷新的 stale 问题。
- 2026-04-07: review-loop follow-up：把固定窗口的起止边界改为按 reporting time zone 的本地墙钟回退，修正 DST 整点时区在最近 30 天窗口上的首尾小时漂移；同时把 `useParallelWorkStats` 的 SSE open 重同步改为排队静默刷新，避免重连抖动时反复打断在途请求。
- 2026-04-07: 刷新 Storybook docs 视觉证据并落盘到 spec 资产目录，当前等待主人确认截图可随提交一起 push 后再进入 PR 收敛。
