# 请求记录分析页：稳定快照 + 聚焦分析 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/6whgx-records-stable-snapshot-analytics/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-10: 新建规格，冻结稳定快照语义、接口扩展、聚焦统计与新数据提示口径。
- 2026-03-10: 完成前后端实现、Rust/Vitest 覆盖与浏览器冒烟；待 PR/checks/review-loop 收尾。
- 2026-03-10: 浏览器复查补齐记录页筛选与排序表单字段的 name 属性，清除表单字段缺少 name/id 的可访问性告警。
- 2026-03-10: 根据 review 收敛空结果 summary 零值、搜索并发失效保护、轻量 new-count 轮询接口，以及搜索后新数据提示复位。
- 2026-03-10: PR #107 已更新并通过 checks，review-loop 收敛完成；补上搜索按钮回归与 `new-count` 强制 `snapshotId` 校验后，规格状态切换为已完成。
- 2026-03-11: 补充 PR 可公开界面截图，并同步到规格固定视觉证据区与 PR 正文。
- 2026-03-12: 将新数据提示从静态数量 + tooltip 说明调整为可点击刷新入口；hover/focus 切换“加载新数据”主题态，点击后显示旋转刷新图标并防止重复触发。
- 2026-03-13: 将记录页新数据提示抽成独立 `RecordsNewDataButton` 组件，补充 Storybook 独立 stories，并新增组件三态截图作为 PR 视觉证据来源。
- 2026-03-19: 为记录页筛选表单补齐浏览器原生自动填充抑制；`FilterableCombobox` 默认关闭浏览器自动填充并允许显式 override，原生 `input/select` 同步复用共享属性策略。
- 2026-03-24: 补充记录页行内详情重做后的页级 Storybook 视觉证据，覆盖完整诊断面板与号池尝试明细区。
