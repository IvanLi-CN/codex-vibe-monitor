# Dashboard Hot Topic 内存投影与 SSE 稳定性实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: child tickets complete; aggregate rollout validation pending
- Lifecycle: active
- Catalog note: Dashboard 七条 hot topic 已具备 typed materializer、稳定 selection 与只读健康诊断

## Coverage / rollout summary

- working-conversations、open-range parallel-work 与 open-window timeseries 均使用 revision-aware typed materializer；健康 live path 不进入通用 builder。
- activity SSE descriptor 固定 `recentLimit=16`，组件按当前可见数量在客户端截断，避免 visibility 变化重建 topic key。
- working-conversations 卡片继续消费每 key 最多 16 条 recent 预览，但客户端固定映射 current/previous/earlier 三个槽位；缺失槽位使用两行中性占位，正常记录收紧为两行，失败记录保留无 label 的错误摘要行。
- 三槽位卡片保留调用详情、账号跳转、键盘可达性、完整值 title/aria 与 blocked/in-flight 诊断；该 owner-facing 信息密度变化不修改 HTTP/SSE wire shape 或后端 recent 上限。
- `runtimePressureHealth.dashboardHotTopics` 按七条 topic 报告 class、state、subscriber、build、fallback、live DB read、serialization、cadence miss 与 reconnect churn。
- System Status 以只读方式展示 healthy、deferred、hot-DB-read 与 cadence-miss；字段缺失时保持 unknown 兼容。
- 完整 Dashboard topic bundle 的 10,000 mutation、双订阅共享 frame 与零 fallback/DB-read 门禁由 stateful topology test 覆盖。

## Remaining Gaps

- 在 aggregate PR 完成 integration CI、owner acceptance 与线上受控 A/B 验收。

## Related Changes

- Ticket #785: working-conversations typed projection and bounded recovery.
- Ticket #786: parallel-work typed projection.
- Ticket #787: open-window timeseries typed materializer.
- Ticket #792: exhaustive topic classification, stable activity selection, hot-topic health and System Status diagnostics.

## References

- `./SPEC.md`
- `./HISTORY.md`
- `../high-frequency-runtime-data-plane/IMPLEMENTATION.md`
