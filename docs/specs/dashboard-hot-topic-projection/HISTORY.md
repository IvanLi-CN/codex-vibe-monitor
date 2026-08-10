# Dashboard Hot Topic 内存投影与 SSE 稳定性演进历史

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 将 working-conversations、parallel-work 与 open-window timeseries 从通用订阅构建路径中独立出来，定义为必须使用 typed projection 的 Dashboard HotProjection。
- 将 activity topic 的 `recentLimit` 固定为 16，动态显示数量改由客户端本地截断，避免数据变化触发 SSE descriptor 重建。
- 将 System Status additive 诊断视为允许的 owner-facing 只读变更；Dashboard 的既有交互和公开数据合同保持不变。

## Key Reasons / Replacements

- 既有 activity、summary 和 network typed materializer 只能证明部分 topic 已收口，不能证明完整 Dashboard 页面不会触发数据库或通用 JSON builder。
- 生产观测显示 Dashboard 页面活跃与 CPU/SQLx 占用相关；剩余三条通用 topic 是需要单独验收的架构缺口。
- 本主题使用完整 Dashboard topic bundle、per-topic build/SQL/serialization 计数和页面 A/B 作为完成证明，替代单组件“零 SQL”结论。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
- `../high-frequency-runtime-data-plane/HISTORY.md`
