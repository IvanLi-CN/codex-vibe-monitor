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
- 线上 Dashboard 开关 A/B 证明“零 live SQL”和“subscriber 共享 frame”不足以约束整页成本；一个页面激活多个 topic 时，完整 live snapshot 广播、cached payload 深拷贝和 topic 级 JSON materialization 仍会放大 CPU。
- current/phase、network/rate、terminal totals 固定拆为 `250ms / 1s / 5s` 三个 revisioned slice；网络可见性不得继续每 `250ms` 唤醒完整 Dashboard projection。
- Dashboard topic 按 activity、summary、network timeseries、network recent 分批迁移到 typed materializer。legacy delivery 只保留一个发布版本用于显式回滚，不作为长期并行架构。
- current/phase、network/rate 与 terminal totals 的触发窗口分别拥有独立 generation 和固定 deadline；network-only 变化不得推进 current revision，terminal durable totals 只在 terminal window 结束后刷新。
- Issue #756 将四类 Dashboard live topic 落为 typed base 和 revision dependency graph：activity 依赖 current/network，summary 依赖 current，network timeseries/recent 依赖 network。Auto delivery 只接收 projection slice 并为每个 topic revision 生成一个共享 `Arc<SerializedTopicFrame>`；业务 snapshot 广播和通用 JSON mutation 留在 explicit legacy rollback path。网络切片相等判定包含 recent 与 current-rate 依赖，确保内存数据变化推进正确 revision 而不触发 SQLite 或 reconcile。
- Dashboard slice 与脱离锁的 frame materialization 都按 revision 单调提交；较旧的 producer 事件或待提交帧不得倒退缓存、replay 或 SSE cursor。
- Issue #759 将 network timeseries 与 network recent 收敛到借用式 typed materializer：timeseries 只替换 live point，recent 直接引用 network slice，避免完整 cached topic 深拷贝。`auto` 下两个 topic 由同一个 `1s` network projection cadence 驱动，不启动订阅任务的独立 recent 定时器；`legacy` kill switch 保留原有 recent cadence，并通过 SSE entrypoint 的实际 frame 覆盖验证。相同 revision 不会重建 frame、序列化或推进 cursor。
- Issue #758 将 terminal slice 接入 activity 与 summary typed materializer：activity 的依赖 tuple 扩展为 current/network/terminal，summary 扩展为 current/terminal。Auto 线路在 5 秒 deadline 广播结构化 terminal delta，并在活动 topic 的 typed base 上按 revision 原地聚合，不走 SQLite、reconcile、完整 topic clone 或通用 JSON overlay；重复 terminal revision 不推进 SSE cursor。`legacy` 保留原有 Records refresh 作为一发布周期的回退路径。
- Activity terminal materialization retains its hidden aggregation inputs so a slice preserves existing model-performance, latency and recent-invocation response semantics while still delivering only one bounded typed projection through the shared frame path.
- Activity baselines record the sequence of durable queued terminals they already include, and terminal batches derive model and latency responses once per affected aggregate rather than once per delta.
