# High-Frequency Runtime Data Plane History

## Key Decisions

- 高频路径的边界由类型依赖和计数测试约束，不再仅依赖“memory-first”命名或日志字段。
- request body 只允许一个 replay snapshot 和一个语义投影；file-backed body 不因 dispatch/rewrite 再变成完整内存副本。
- Dashboard current-state 与 terminal totals 使用不同 cadence：前者 `250ms`，后者 `5s`；SQLite reconcile 保持 `60s`。
- SSE revision 在 producer 侧只序列化一次，subscriber 数量不得放大 builder 或 serialization 成本。
- SQLite writer accounting 必须跨 P1/P2 ownership transfer 原子守恒；下溢属于 degraded health，而不是可忽略的 telemetry 异常。
- 先使用 glibc `MALLOC_ARENA_MAX=8` 限制 allocator 保留，不在本阶段引入 jemalloc。
- System Status 的运行压力诊断只读取内存计数器，避免诊断面反向制造数据库压力。
- 阶段目标为 RSS p95 `2 GiB`，`1 GiB` 保留为后续软目标；不得为了达标降低并发或丢数据。
