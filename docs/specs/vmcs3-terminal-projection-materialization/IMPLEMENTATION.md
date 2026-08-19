# Terminal Projection 中间层与增量物化实现说明

## 结构

- `src/terminal_projection.rs` 保存紧凑 terminal event key、P1 ACK 水位和每个消费者的可回收 cursor。
- `src/sqlite_batch_writer.rs` 在 raw terminal P1 成功后通知 Hub；journal ACK 仍在同一 P1 成功边界完成。
- `src/long_term_stats.rs` 使用 `long_term_projection_state`、调用级 canonical interval state 与 `long_term_projection_dirty_buckets` 将持久 row cursor 变成受 pressure gate 约束的增量 rollup 物化；正常路径只 hydrate 新 terminal row，目标自然日重建只处理修复。旧六路展开的 interval state 在迁移期间与 canonical state 合并读取，压缩写入后再以短事务渐进清理。invocation 修正、invocation archive 与 attempt archive 的 completed manifest 变更都会在其源事务中入队对应上海自然日；archive 覆盖范围重写同时入队旧、新范围。
- `src/api/slices/system_routes_and_tasks.rs` 将 Hub 和长期 projection 的内存 health 作为 additive `projectionHealth` 暴露；页面只读展示并可展开细项。
- `proxy_raw_payload_blobs` 与 `proxy_raw_payload_blob_links` 把已发布 response raw 的物理对象与 invocation/attempt owner 分开。保留流程先解除 owner link，最后一个 link 消失后才允许删除物理文件；新 link 同时驱动 System Status raw 指标的有界增量盘点。

## 运行边界

- terminal ingress 先写 Dashboard 紧凑 delta 并注册 projection event；enqueue 失败会同时撤销这两个 speculative 状态。
- P1 ACK 后长期统计最多等待一个 60 秒固定窗口，然后按新的 `rowId` hydrate terminal、更新内存 interval 并集，并在受 `512` 行上限的 P2 micro-transactions 中持久化 canonical interval state。正常终态若 cursor 尚未跨越其 `rowId` 走增量；若乱序完成导致 cursor 已跨越该行，write-side trigger 改为标记目标自然日 repair。interval baseline 缺失或其他修正同样标记 repair；repair 读取 live、目标日期相关的 archive rows 与关联 attempt source，在状态批次完成后原子替换 rollup、推进 cursor 和清理 repair marker。任一 archive source 不可读、写入 pressure 或 shutdown 都保留 last-good 和 dirty marker，并将该桶延后五分钟，不阻塞其余脏桶。
- 压力门关闭时 flush 被明确延期；已有 API/页面继续读取 last-good durable rollup，下一次有资格的窗口恢复。
- Projection schema、trigger 与兼容迁移只在启动初始化执行；60 秒 P2 flush 和每日验证只读写投影数据，不执行 DDL。
- Hourly rollup retention remains enforced by the incremental P2 path: it removes expired hourly rollups and legacy hourly interval segments in bounded low-priority transactions while preserving permanent daily history. Canonical intervals remain available for durable daily replay.
- `wall_time_ms` is a persisted interval-union snapshot rather than an additive counter. After restart, the first new delta hydrates that union before replacing the bucket value, preserving exact overlap semantics.

## Memory Attribution

- `src/memory_diagnostics.rs` runs a startup sample and a 30-second low-frequency sample without cloning business state or querying SQLite. It reads process/cgroup memory files and emits structured attribution fields.
- Component estimates use capacities and bounded counters from the existing stores. Terminal hub pending bytes and timeseries staging are intentionally not counted twice; operation logs expose `retained_bytes`, `retained_delta_bytes`, `peak_delta_bytes`, `load_row_count` and `clone_avoided`.
- The operation hooks cover long-term flush, timeseries minute flush and raw inventory maintenance. A raw writer estimate uses the existing semaphore occupancy and the bounded ingress queue size; durable spool bytes remain disk telemetry, not RSS attribution.
- `peak_delta_bytes` is derived from the process `VmHWM` delta, while `rss_delta_bytes` remains the endpoint RSS delta. This keeps transient allocation peaks separate from retained component estimates.
