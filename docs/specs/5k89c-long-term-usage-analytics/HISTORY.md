# Stats 长期用量与性能统计演进记录

## 关键决策

- 长期统计使用独立汇总表和独立 API，避免在现有 Stats 内容上叠加状态耦合。
- 日汇总永久保存，小时汇总保留默认 400 天且最少 366 天；所有清理动作以日汇总 materialized 为前置条件。
- 首次升级采用可恢复后台回填；回填未完成时只显示准备进度，避免把部分历史误报为完整结果。
- API Key 删除保留稳定统计身份，凭据、会话和路由状态全部清除；其它上游继续使用原删除行为。

## 关联演进

- 长周期归档与 retention 约束沿用 `9aucy` 的 materialization/cleanup 边界。
- 请求阶段耗时、模型与 cache 字段沿用 `z9h7v` 的可观测性事实源。

## 实现落点

- 长期统计读取路径只依赖日汇总；archive 仅在后台回填阶段解压读取，并通过 `long_term_usage_stats` replay target 阻止未完成 archive 清理。
- `LongTermStatsSection` 把长期统计状态隔离在现有 Stats 之后，模型/账号表格共享固定列定义并分别启用行、列虚拟化。
