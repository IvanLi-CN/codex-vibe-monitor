# 活动总览时间范围与历史活动归位 主题背景

## Lifecycle / Compatibility

- 本主题统一全局与上游账号活动总览的范围、主值、比较和实时图表规则，并将历史活动日历集中到统计页；统计页既有长期统计面板仍由其独立主题维护。
- 移除的 `7d`、`usage` 选择通过回到今日兼容；历史持久选择不应导致空面板。此需求本身不改变调用数据、计费结果或保留策略。

## Replacements / Background

- 七卡的即时主值与累计值并存：今日和 24 小时保留实时表现，昨日显示日均。将所有范围主值统一改为均值不属于本主题的目标。
- 范围内表现用均值比较，累计用量用等长范围比较；实时卡不新增昨日同一分钟采样对照。昨日的比较基线必须补齐为前日。
- [自然日七卡主题](../gz5ns-dashboard-natural-day-kpi-semantics/SPEC.md) 的既有四区布局、自适应精度及基础指标定义继续适用；其范围、比较标签和成功倍数约定由本主题相关要求替代。
- [调用时间线主题](../dashboard-invocation-timeline/SPEC.md)、[ADR 0019](../../adr/0019-dashboard-timeline-replacement-contract.md) 与后继 [ADR 0047](../../adr/0047-dashboard-live-timeline-range-independence.md) 的调用身份、布局、快照、完整分页、容量和错误恢复合同继续适用；自然日绑定、昨日关闭实时和午夜截断视窗由后继 ADR 明确替代。
- 页面范围、指标口径和历史入口属于可逆的产品行为选择；时间线独立视窗的结构性边界由 ADR 0047 固化。

## Related Changes

- `docs/adr/0047-dashboard-live-timeline-range-independence.md`

## Delivery

- PR #1169 (`feat(web): unify activity ranges and history`) 已在 `f67805a6` 收敛到 current-head CI 全绿；源码质量预算修复只调整统计范围边界的模块归属，不改变本主题的外部行为或视觉证据。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
