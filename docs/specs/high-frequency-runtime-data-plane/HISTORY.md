# High-Frequency Runtime Data Plane History

## Key Decisions

- 高频路径的边界由类型依赖和计数测试约束，不再仅依赖“memory-first”命名或日志字段。
- request body 只允许一个 replay snapshot 和一个语义投影；file-backed body 不因 dispatch/rewrite 再变成完整内存副本。
- Dashboard current-state 与 terminal totals 使用不同 cadence：前者 `250ms`，后者 `5s`；SQLite reconcile 保持 `60s`。
- SSE revision 在 producer 侧只序列化一次，subscriber 数量不得放大 builder 或 serialization 成本。
- SQLite writer accounting 必须跨 P1/P2 ownership transfer 原子守恒；下溢属于 degraded health，而不是可忽略的 telemetry 异常。
- 先使用 glibc `MALLOC_ARENA_MAX=8` 限制 allocator 保留，不在本阶段引入 jemalloc。
- System Status 的运行压力诊断只读取内存计数器，避免诊断面反向制造数据库压力。
- Issue #737 将 topic revision 固化为共享不可变 `Arc<SerializedTopicFrame>`；cache、replay 与 broadcast 共享 frame，subscriber 通过共享 `Bytes` 分片发送 SSE envelope，且 byte-identical revision 不推进 cursor。
- Issue #738 将运行压力健康度作为 additive System Status 合同，并以缺失即 unknown 的方式保持旧后端兼容；诊断详情不得包含 payload、调用 ID 或原始 SQL。
- 阶段目标为 RSS p95 `2 GiB`，`1 GiB` 保留为后续软目标；不得为了达标降低并发或丢数据。
- Dashboard live projection 在 mutation 时维护紧凑账号聚合；发布阶段不得重新遍历 retained runtime records。性能测试必须覆盖不同 key 的真实保留集合，而不是反复覆盖同一个 key。
- 请求语义管线的 parse、materialization、buffer peak 与 fallback 计数属于 `runtimePressureHealth` 合同，不能只存在于单请求日志。
