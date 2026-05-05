# Dashboard 今日 KPI 上下文统计卡片 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/2qsev-dashboard-tpm-cost-per-minute-kpi/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-10: 新建 follow-up spec，冻结 Dashboard 今日 KPI 切换为 5m-avg TPM 与 Cost/min 的范围、验收与视觉证据要求。
- 2026-04-10: 完成前端 5m-avg 速率派生、6-tile KPI 重排、Vitest/Storybook 覆盖与本地视觉证据归档，并获主人授权将截图随 PR 一起提交。
- 2026-04-28: 根据主人反馈修正速率口径：从最近 5 个已完成分钟桶改为最近 5 分钟活跃尾段均值，当前部分分钟参与，前置空闲不稀释速率，活动后的安静期会随当前时间继续拉长分母；同时为 KPI 标题增加字段说明 tooltip。
- 2026-04-30: 将今日 KPI 升级为一个主信息加两个辅助信息；合并成功/失败，新增并行对话卡片，补充工作分钟日均、前 7 完整日均值、缓存命中和较昨日百分比颜色区分。
