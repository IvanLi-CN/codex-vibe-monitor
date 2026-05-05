# 号池 Tag 路由与管理扩展 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/t3rdp-account-pool-tag-routing/SPEC.md`

## Migrated History Notes

## Change log

- 2026-04-04：为 tag 与 `effectiveRoutingRule` 增加 `priorityTier`（`primary` / `normal` / `fallback`），fresh assignment、sticky 迁移目标与 node-shunt 改为“优先级外层分层 + 同层沿用现有 comparator”的双层调度；账号详情与 Tags 列表同步展示优先级结果，并回填新的 Storybook mock-only 视觉证据。
- 2026-04-01：补充标签规则弹窗在有限值与无限值两种并发档位下的 owner-facing Storybook 视觉证据，并把本 spec 的历史 `## Visual Evidence (PR)` 迁移为标准 `## Visual Evidence`。
- 2026-04-01：将路由候选里的活跃 sticky 共享窗口从 30 分钟统一收敛为 5 分钟；有限额账号的 sticky 软降权、并发负载统计与相关验收语义同步切到同一 5 分钟口径，不新增接口或配置项。
- 2026-03-25：修正半小时活跃 sticky 软降权的“有限额账号”定义，改为本地限额或最新远程额度窗口任一存在即生效；远程单窗口与 `0%` 已知窗口同样视为有限额信号，并补齐端到端回归，明确只有本地与远程额度信号都缺失时才继续豁免该软降权；主干 freshness 合入后继续保持路由候选比较器回归夹具与 credits 元数据字段对齐，不改变既有规则语义。
- 2026-03-22：补充半小时活跃 sticky 软降权的适用范围，明确该软降权只作用于至少配置了一个本地 `5 小时` 或 `7 天` 限额的账号；两个本地限额都为空的账号继续沿用既有候选排序，不受该软降权影响。
- 2026-03-18：补充账号页与新增页的 tag 字段交互契约，明确必须收敛为“内联 chips + 尾部添加触发器 + anchored popover 搜索/多选/创建”的单控件模型，并要求多选过程保持 popover 打开；本轮 PR 视觉证据继续限定为 Storybook mock-only。
