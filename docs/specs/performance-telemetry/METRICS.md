# 独立性能遥测指标目录

## 口径

本目录是代码内 `METRIC_SPECS` 注册表的 owner-facing 说明。指标 ID、类型和允许维度以 [performance_telemetry.rs](../../../src/performance_telemetry.rs) 为准；新增或删除指标时必须同步更新本目录和注册表测试。

- `R`：近期指标。保留最近 7 天的 1 分钟桶和第 8 至 30 天的 5 分钟桶。
- `L`：长期指标。除近期桶外，继续保留到 13 个月的 1 小时桶。
- `Counter`：时间桶内的事件或累计量，查询时使用 `sum`。
- `Duration`：毫秒耗时，保留 `count/sum/min/max` 和固定直方图；直方图边界为 `<=1`、`<=5`、`<=10`、`<=25`、`<=50`、`<=100`、`<=250`、`>250` ms。
- `Gauge`：采样到的存量或状态；查询时使用时间加权平均，并保留末值、最小值和最大值。
- 所有维度都是固定枚举，不包含账号、模型、URL、SQL、IP、invocation、conversation 或订阅者 ID。
- 指标先在进程内聚合，采集器每 60 秒以内批量写入独立 SQLite；指标事件不会逐条写入数据库。

当前注册表包含 77 个指标 ID，展开为 104 条近期序列和 22 条长期序列。

## 服务入口

| ID               | 类型 / 维度 / 留存  | 含义                                                                             |
| ---------------- | ------------------- | -------------------------------------------------------------------------------- |
| `http.in_flight` | Gauge / `other` / R | 当前正在处理的 HTTP 请求数量。只表示并发存量，不记录每个请求的耗时、状态或结果。 |

## P1、P2 与 SQLite

P1 是终端关键写入，P2 是派生或后台写入。`p1.queue_depth` 和 `p1.queue_bytes` 当前读取写入器的总 pending queue；`p2.queue_depth` 在此基础上扣除排队 P1 数量。

| ID                                    | 类型 / 维度 / 留存                                                                    | 含义                                                            |
| ------------------------------------- | ------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| `p1.ack_duration_ms`                  | Duration / `p1` / L                                                                   | P1 从协调器等待和 SQLite 执行开始，到 terminal ACK 完成的耗时。 |
| `p1.queue_depth`                      | Gauge / `p1` / L                                                                      | 写入器 pending queue 当前条目数。                               |
| `p1.queue_bytes`                      | Gauge / `p1` / L                                                                      | 写入器 pending queue 的估算字节数。                             |
| `p2.queue_depth`                      | Gauge / `p2` / R                                                                      | 扣除排队 P1 后的后台 P2 待处理条目数。                          |
| `p1.retry_count`                      | Counter / `p1` / R                                                                    | P1 被延后后重新安排的次数。                                     |
| `p2.retry_count`                      | Counter / `p2` / R                                                                    | P2 被延后后重新安排的次数。                                     |
| `p1.transfer_bytes`                   | Counter / `p1` / R                                                                    | P1 处理后转入 P2 的待处理数据估算字节数。                       |
| `p2.next_attempt_delay_ms`            | Gauge / `p2` / R                                                                      | 当前 P2 调度距离下一次尝试的剩余毫秒数。                        |
| `p2.deferred_age_ms`                  | Gauge / `p2` / R                                                                      | 当前 P2 延后任务已经等待的毫秒数。                              |
| `p2.flush_attempt_count`              | Counter / `p2` / R                                                                    | P2 flush 尝试次数。                                             |
| `p2.pressure_defer_count`             | Counter / `p2` / L                                                                    | 因数据库压力保护而延后 P2 的次数。                              |
| `p2.lock_retry_count`                 | Counter / `p2` / L                                                                    | 因 SQLite 锁冲突而重试 P2 的次数。                              |
| `sqlite.busy_count`                   | Counter / `main` / L                                                                  | 主库出现 SQLite `BUSY` 的次数。                                 |
| `sqlite.locked_count`                 | Counter / `main` / L                                                                  | 主库出现 SQLite `LOCKED` 的次数。                               |
| `sqlite.pool_timeout_count`           | Counter / `main` / L                                                                  | 获取主库连接池超时的次数。                                      |
| `sqlite.background_skip_count`        | Counter / `main` / R                                                                  | 后台任务因前台优先级或数据库压力而被跳过的次数。                |
| `sqlite.write_duration_ms`            | Duration / `main` / R                                                                 | 一个主库批量写入批次的执行耗时。                                |
| `sqlite.write_rows`                   | Counter / `main` / R                                                                  | 批量写入提交的逻辑行数。                                        |
| `sqlite.write_bytes`                  | Counter / `main` / R                                                                  | 批量写入提交的估算字节数。                                      |
| `sqlite.wal_bytes`                    | Gauge / `main` / R                                                                    | 主库 WAL 文件大小。                                             |
| `sqlite.coordinator_waiters`          | Gauge / `p1_terminal`、`interactive_proxy`、`p2_derived`、`maintenance_retention` / R | 各写入类别当前等待写协调器的任务数。                            |
| `sqlite.coordinator_wait_duration_ms` | Duration / 同上四类 / R                                                               | 各写入类别获得协调器前的平均等待耗时。                          |
| `sqlite.coordinator_bypass_count`     | Counter / `coordinator` / R                                                           | 绕过标准写协调路径直接写入的次数。                              |
| `sqlite.maintenance_fairness_count`   | Counter / `coordinator` / R                                                           | 维护任务获得公平调度机会的次数。                                |

## 读模型与 SSE

投影切片固定为 `current`、`network`、`terminal`。这些指标描述投影和发布链路，不描述单个代理请求。

| ID                                   | 类型 / 维度 / 留存                                 | 含义                                                                                     |
| ------------------------------------ | -------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `projection.last_good_age_ms`        | Gauge / `dashboard` / L                            | Dashboard 投影距离最近一次成功生成的毫秒数。                                             |
| `projection.build_count`             | Counter / `dashboard` / L                          | Dashboard 运行投影构建次数。                                                             |
| `projection.live_db_read_count`      | Counter / `dashboard` / R                          | 投影路径直接读取主库实时数据的次数。                                                     |
| `sse.active_subscribers`             | Gauge / `dashboard` / L                            | 当前活跃 Dashboard SSE 订阅数。                                                          |
| `projection.cadence_miss_count`      | Counter / 三个投影切片 / R                         | 各切片未按计划节奏完成发布的次数。                                                       |
| `projection.revision_count`          | Counter / 三个投影切片 / R                         | 各切片 revision 递增次数。                                                               |
| `projection.publish_duration_ms`     | Duration / 三个投影切片 / R                        | 一个投影发布窗口的耗时。                                                                 |
| `projection.publish_count`           | Counter / 三个投影切片 / R                         | 投影发布窗口完成次数。                                                                   |
| `projection.snapshot_bytes`          | Gauge / 三个投影切片 / R                           | 生成的快照或 terminal delta 的估算字节数。                                               |
| `projection.reconcile_duration_ms`   | Duration / `dashboard` / R                         | Dashboard 投影 reconcile 耗时。                                                          |
| `projection.reconcile_count`         | Counter / `dashboard` / R                          | reconcile 成功次数。                                                                     |
| `projection.reconcile_defer_count`   | Counter / `writer_pressure`、`background_busy` / R | reconcile 因写压力或后台繁忙而延后的次数。                                               |
| `projection.reconcile_failure_count` | Counter / `dashboard` / R                          | reconcile 失败次数，不包括明确延后的情况。                                               |
| `sse.publish_error_count`            | Counter / 三个投影切片 / R                         | SSE 广播发送失败次数。                                                                   |
| `sse.publish_duration_ms`            | Duration / 三个投影切片 / R                        | 当前实现中记录同一个 publish window 时长，与 `projection.publish_duration_ms` 数值重复。 |
| `sse.frame_bytes`                    | Counter / 三个投影切片 / R                         | 发送出的 SSE frame 估算字节数。                                                          |

## 维护与归档

| ID                                  | 类型 / 维度 / 留存           | 含义                                                                        |
| ----------------------------------- | ---------------------------- | --------------------------------------------------------------------------- |
| `maintenance.backlog_age_ms`        | Gauge / `maintenance` / L    | retention recovery 中最老待处理积压的年龄。                                 |
| `maintenance.run_duration_ms`       | Duration / `maintenance` / L | 一次维护任务执行耗时。                                                      |
| `maintenance.processed_rows`        | Counter / `maintenance` / R  | 维护任务处理的逻辑行数。                                                    |
| `maintenance.raw_bytes_before`      | Gauge / `maintenance` / R    | raw 维护开始前的 raw 文件字节数。                                           |
| `maintenance.raw_bytes_after`       | Gauge / `maintenance` / R    | raw 维护结束后的 raw 文件字节数。                                           |
| `maintenance.compressed_file_count` | Counter / `maintenance` / R  | 本次维护压缩的 raw 文件数量。                                               |
| `maintenance.removed_file_count`    | Counter / `maintenance` / R  | 本次维护删除的 raw 文件和孤儿文件数量。                                     |
| `maintenance.archived_rows`         | Counter / `maintenance` / R  | 归档的 invocation、proxy attempt、upstream attempt 和 quota snapshot 行数。 |

## 进程、容量与采集器

| ID                                | 类型 / 维度 / 留存          | 含义                                                    |
| --------------------------------- | --------------------------- | ------------------------------------------------------- |
| `process.rss_bytes`               | Gauge / `process` / L       | 进程 RSS 总量。                                         |
| `process.cpu_percent`             | Gauge / `process` / L       | 进程 CPU 使用率；Linux 上由进程 tick 与系统 tick 计算。 |
| `process.rss_anon_bytes`          | Gauge / `process` / L       | RSS 中匿名内存大小。                                    |
| `process.swap_bytes`              | Gauge / `process` / L       | 进程使用的 Swap 大小。                                  |
| `process.managed_bytes`           | Gauge / `process` / L       | 现有内存诊断能够归属到已知组件的内存估算。              |
| `process.unattributed_anon_bytes` | Gauge / `process` / R       | 无法归属到已知组件的匿名内存。                          |
| `storage.main_db_bytes`           | Gauge / `main_db` / L       | 主库文件大小。                                          |
| `storage.telemetry_db_bytes`      | Gauge / `telemetry_db` / L  | 独立遥测库文件大小。                                    |
| `process.thread_count`            | Gauge / `process` / R       | 进程线程数量。                                          |
| `process.disk_free_bytes`         | Gauge / `process` / R       | 数据库所在文件系统的可用磁盘空间。                      |
| `storage.telemetry_wal_bytes`     | Gauge / `telemetry_wal` / R | 遥测库 WAL 文件大小。                                   |
| `telemetry.queue_depth`           | Gauge / `collector` / R     | 遥测内存队列当前占用量。                                |
| `telemetry.dropped_samples`       | Counter / `collector` / L   | 队列满、数据库不可用或无法持久化时丢弃的样本数量。      |
| `telemetry.flush_duration_ms`     | Duration / `collector` / R  | 一次遥测批量 flush 的耗时。                             |
| `telemetry.flush_failure_count`   | Counter / `collector` / L   | 遥测批量写入失败次数。                                  |
| `telemetry.flush_batch_events`    | Counter / `collector` / R   | flush 批次中的事件数。                                  |
| `telemetry.flush_batch_buckets`   | Counter / `collector` / R   | flush 批次合并出的时间桶数量。                          |
| `telemetry.backoff_count`         | Counter / `collector` / R   | 遥测写入失败后进入退避的次数。                          |

## 浏览器真实体验

浏览器页面族固定为 `dashboard`、`records`、`system`。`data_ready_ms` 和 `update_to_paint_ms` 另外按 `mobile`、`desktop` 拆分；其余浏览器指标只按页面族拆分。每页最多缓存 8 个事件，每分钟最多发送一次。

| ID                                | 类型 / 维度 / 留存       | 含义                                                                |
| --------------------------------- | ------------------------ | ------------------------------------------------------------------- |
| `browser.data_ready_ms`           | Duration / 页面+设备 / R | 页面组件初始化到第一次数据就绪帧的耗时。                            |
| `browser.update_to_paint_ms`      | Duration / 页面+设备 / R | 数据就绪后到下一次浏览器 paint 的耗时。                             |
| `browser.long_task_ms`            | Duration / 页面族 / R    | 浏览器观测到的 long task 持续时间。                                 |
| `browser.long_task_count`         | Counter / 页面族 / R     | long task 数量。                                                    |
| `browser.api_request_duration_ms` | Duration / 页面族 / R    | 页面同源 API resource timing 耗时，排除性能遥测查询接口。           |
| `browser.api_request_count`       | Counter / 页面族 / R     | 上述页面 API resource 条目数量。                                    |
| `browser.sse_duration_ms`         | Duration / 页面族 / R    | `/events` SSE resource timing 持续时间。                            |
| `browser.sse_disconnect_count`    | Counter / 页面族 / R     | `/events` resource 条目结束次数；当前还不能区分正常关闭和异常断开。 |
| `browser.unsupported_count`       | Counter / 页面族 / R     | 浏览器不支持或无法初始化 PerformanceObserver 的次数。               |
| `browser.visibility_hidden_count` | Counter / 页面族 / R     | 页面进入 hidden 状态的次数。                                        |

## 不包含的指标

以下内容明确不进入独立指标库：

- 代理请求的阶段耗时、TTFT、TTFB、总耗时、状态码、结果、上游重试和请求字节数。
- 历史 API 的请求量、耗时、状态和响应大小。
- 按账号、模型、URL、IP、invocation、conversation 或订阅者 ID 建立的序列。

这些数据继续分别由业务主库调用明细、历史查询和现有 System Status 负责。
