# 性能指标迁移映射

本文件是 [锁定设计](performance-observability.md) 的逐项迁移合同。源清单为切换前 [77 个指标 ID](../specs/performance-telemetry/METRICS.md)；该清单描述切换前信号；当前实现与验证进度见[实现记录](../specs/performance-telemetry/IMPLEMENTATION.md)。每个旧 ID 必须有一行动作，不能只删除旧模块而丢失其观测能力。

新指标用秒、字节和累计 Counter；固定标签遵循设计白名单。表中的标签只说明分类，不要求把全部基础标签写在每行。Histogram 的 `_count` 是实际记录的耗时样本数，Counter 的 `_total` 是被计数的事件；抽样时两者分母不同。新指标由事件来源更新，不从旧库或旧时间桶转换。

## HTTP 与写入

| 旧 ID                                 | 新指标 / 动作                                                         | 口径与来源                                                                        |
| ------------------------------------- | --------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `http.in_flight`                      | `cvm_http_inflight`                                                   | 修正为请求进入至 body 完成/错误/取消，包含存活流；不能在 response head 时减       |
| `p1.ack_duration_ms`                  | `cvm_sqlite_batch_ack_duration_seconds{class="p1_terminal"}`          | 每个真实批次记录一次，从开始等待 coordinator 到 commit/ACK；不含之前 queue wait   |
| `p1.queue_depth`                      | `cvm_sqlite_pending_items{queue="all"}`                               | 旧值实际为总 pending；保留总量，同时新增真实 P1 分类，不能将旧值改名后宣称只含 P1 |
| `p1.queue_bytes`                      | `cvm_sqlite_pending_bytes{queue="all"}`                               | 旧值为总 pending 估算字节；分类字节只在真实 accounting 支持时导出                 |
| `p2.queue_depth`                      | `cvm_sqlite_pending_items{queue="p2_derived"}`                        | 从真实分类 accounting 读取；all 与分类值不得重复相加                              |
| `p1.retry_count`                      | `cvm_sqlite_retries_total{class="p1_terminal"}`                       | 每次实际重新安排累计一次                                                          |
| `p2.retry_count`                      | `cvm_sqlite_retries_total{class="p2_derived"}`                        | 同上，与 pressure defer 分开                                                      |
| `p1.transfer_bytes`                   | `cvm_sqlite_transfer_bytes_total{from="p1_terminal",to="p2_derived"}` | 实际转移的估算字节                                                                |
| `p2.next_attempt_delay_ms`            | `cvm_sqlite_next_attempt_delay_seconds{class="p2_derived"}`           | 当前距离下一次允许尝试的剩余时间                                                  |
| `p2.deferred_age_ms`                  | `cvm_sqlite_deferred_age_seconds{class="p2_derived"}`                 | 当前延后工作的实际等待年龄                                                        |
| `p2.flush_attempt_count`              | `cvm_sqlite_flush_attempts_total{class="p2_derived"}`                 | 每次真实尝试累计，成功/失败另有 outcome                                           |
| `p2.pressure_defer_count`             | `cvm_sqlite_defers_total{class="p2_derived",reason="pressure"}`       | 压力保护让行；不是实际 SQLite 锁错误                                              |
| `p2.lock_retry_count`                 | `cvm_sqlite_retries_total{class="p2_derived",reason="lock"}`          | retry family 统一 reason 枚举；分类总量由标签求和，不能再重复计一个无 reason 总量 |
| `sqlite.busy_count`                   | `cvm_sqlite_errors_total{kind="busy"}`                                | 主库真实 BUSY 来源一次累计                                                        |
| `sqlite.locked_count`                 | `cvm_sqlite_errors_total{kind="locked"}`                              | 主库真实 LOCKED 来源一次累计                                                      |
| `sqlite.pool_timeout_count`           | `cvm_sqlite_pool_timeouts_total`                                      | 获取主库连接失败来源累计，pool wait 单独计时                                      |
| `sqlite.background_skip_count`        | `cvm_sqlite_background_skips_total`                                   | 既有后台 skip 决策，不伪装为 SQL 执行失败                                         |
| `sqlite.write_duration_ms`            | `cvm_sqlite_batch_execute_duration_seconds`                           | 从获得 write permit 后开始事务执行到 commit；失败样本/计数有明确 outcome          |
| `sqlite.write_rows`                   | `cvm_sqlite_written_rows_total`                                       | 成功提交的逻辑行数；不是失败尝试的参数条数                                        |
| `sqlite.write_bytes`                  | `cvm_sqlite_written_bytes_total`                                      | 成功批次的估算字节，保留估算说明                                                  |
| `sqlite.wal_bytes`                    | `cvm_sqlite_wal_bytes{database="main"}`                               | 主库 WAL metadata，无文件扫描                                                     |
| `sqlite.coordinator_waiters`          | `cvm_sqlite_coordinator_waiters{class}`                               | class 固定 p1_terminal / interactive_proxy / p2_derived / maintenance_retention   |
| `sqlite.coordinator_wait_duration_ms` | `cvm_sqlite_coordinator_wait_duration_seconds{class}`                 | 修正为每次实际准入等待的 Histogram；不能把旧窗口平均值录成一个请求样本            |
| `sqlite.coordinator_bypass_count`     | `cvm_sqlite_coordinator_bypasses_total`                               | 真实绕过事件；不为观测改变写协调规则                                              |
| `sqlite.maintenance_fairness_count`   | `cvm_sqlite_maintenance_fairness_total`                               | 现有公平准入机会来源累计                                                          |

retry 的 reason 在各来源统一为固定枚举；未知原因用明确 `other`，不新增动态字符串。已有业务 pressure/accounting 仍用于业务决策，观测只记录来源，不改变策略。

## 读模型与 SSE

| 旧 ID                                | 新指标 / 动作                                                       | 口径与来源                                                                                 |
| ------------------------------------ | ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| `projection.last_good_age_ms`        | `cvm_projection_last_good_age_seconds{projection="dashboard"}`      | 最近成功发布的实际年龄；未有成功值时 unknown                                               |
| `projection.build_count`             | `cvm_projection_builds_total{projection="dashboard"}`               | source build 事件，不把 scrape 次数当构建次数                                              |
| `projection.live_db_read_count`      | `cvm_projection_live_db_reads_total{projection="dashboard"}`        | 投影路径实际 live 主库读取                                                                 |
| `sse.active_subscribers`             | `cvm_sse_active_subscribers{topic="dashboard"}`                     | 实际订阅生命周期                                                                           |
| `projection.cadence_miss_count`      | `cvm_projection_cadence_misses_total{slice}`                        | 固定 current / network / terminal                                                          |
| `projection.revision_count`          | `cvm_projection_revision_changes_total{slice}`                      | 保留真实 revision 递增事件数；当前 revision 水位另用 Gauge，不从两次 scrape 的差猜测事件数 |
| `projection.publish_duration_ms`     | `cvm_projection_publish_duration_seconds{slice}`                    | 每个真实 publish window 计一次                                                             |
| `projection.publish_count`           | `cvm_projection_publications_total{slice}`                          | 实际完成窗口数量，不能从 revision 差推定                                                   |
| `projection.snapshot_bytes`          | `cvm_projection_snapshot_bytes{slice}`                              | 当前快照/terminal delta 估算大小                                                           |
| `projection.reconcile_duration_ms`   | `cvm_projection_reconcile_duration_seconds{projection="dashboard"}` | 实际 reconcile 窗口；deferred 与执行失败分开                                               |
| `projection.reconcile_count`         | `cvm_projection_reconciliations_total{outcome="success"}`           | 成功 reconcile 次数                                                                        |
| `projection.reconcile_defer_count`   | `cvm_projection_reconciliations_total{outcome="deferred",reason}`   | reason 固定 writer_pressure / background_busy                                              |
| `projection.reconcile_failure_count` | `cvm_projection_reconciliations_total{outcome="error"}`             | 真正失败，不包含明确让行                                                                   |
| `sse.publish_error_count`            | `cvm_sse_publish_errors_total{slice}`                               | 广播发送失败，和浏览器断开/取消不同                                                        |
| `sse.publish_duration_ms`            | 合并到 `cvm_projection_publish_duration_seconds{slice}`             | 旧值重复同一窗口；不能迁成两个计时样本                                                     |
| `sse.frame_bytes`                    | `cvm_sse_frame_bytes_total{slice}`                                  | 既有 frame 估算字节，不宣称是 socket 层准确 wire bytes                                     |

reconcile family 的每次操作只有一个 outcome；reason 的不存在状态使用一致 schema，不能让同一 family 任意出现/消失标签键。是否 deferred 发生在执行前，不能伪造一个零耗时 reconcile 样本。

## 维护与资源

| 旧 ID                               | 新指标 / 动作                                                 | 口径与来源                                                                                       |
| ----------------------------------- | ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `maintenance.backlog_age_ms`        | `cvm_maintenance_backlog_age_seconds{operation="retention"}`  | 既有 retention recovery 积压最老年龄；blocked 与 actionable 状态不能混为已完成                   |
| `maintenance.run_duration_ms`       | `cvm_maintenance_run_duration_seconds{operation="retention"}` | 该维护操作窗口；任务 dispatcher 全部执行窗口另用 task family                                     |
| `maintenance.processed_rows`        | `cvm_maintenance_processed_rows_total{operation="retention"}` | 实际处理的逻辑行数                                                                               |
| `maintenance.raw_bytes_before`      | `cvm_maintenance_raw_bytes{phase="before"}`                   | 最近一次维护开始前 inventory，不临时遍历文件                                                     |
| `maintenance.raw_bytes_after`       | `cvm_maintenance_raw_bytes{phase="after"}`                    | 对应维护结束 inventory，附 freshness                                                             |
| `maintenance.compressed_file_count` | `cvm_maintenance_files_total{action="compressed"}`            | 每次操作处理量累计；不把最近一批数量不断重复累计                                                 |
| `maintenance.removed_file_count`    | `cvm_maintenance_files_total{action="removed"}`               | 同上，沿用业务删除来源                                                                           |
| `maintenance.archived_rows`         | `cvm_maintenance_archived_rows_total`                         | 实际归档行数                                                                                     |
| `process.rss_bytes`                 | `cvm_process_rss_bytes`                                       | 进程实际 RSS                                                                                     |
| `process.cpu_percent`               | `cvm_process_cpu_seconds_total{mode}`                         | mode 固定 user / system；替换旧百分比口径，由 CPU 秒计算相对单核/明确 quota 的比例，不回填旧比例 |
| `process.rss_anon_bytes`            | `cvm_process_rss_anon_bytes`                                  | 可支持平台的匿名 RSS，其他平台 missing                                                           |
| `process.swap_bytes`                | `cvm_process_swap_bytes`                                      | 实际可观测 swap                                                                                  |
| `process.managed_bytes`             | `cvm_process_managed_bytes`                                   | 已知组件估算，保留估算性质，不增加 payload clone                                                 |
| `process.unattributed_anon_bytes`   | `cvm_process_unattributed_anon_bytes`                         | 原归因模型的估算，不宣称是精确泄漏或 allocation                                                  |
| `storage.main_db_bytes`             | `cvm_storage_database_bytes{database="main"}`                 | 主库文件 metadata                                                                                |
| `storage.telemetry_db_bytes`        | 退役                                                          | 新应用没有性能库；不伪造对应值，Prometheus 容量由平台观测                                        |
| `process.thread_count`              | `cvm_process_threads`                                         | 进程线程数                                                                                       |
| `process.disk_free_bytes`           | `cvm_storage_available_bytes{filesystem="data"}`              | 已知数据卷可用空间，不用任意 path 作为标签                                                       |
| `storage.telemetry_wal_bytes`       | 退役                                                          | 新应用没有性能 WAL                                                                               |

## 旧 collector 自身指标

| 旧 ID                           | 新指标 / 动作 | 口径与来源                                                |
| ------------------------------- | ------------- | --------------------------------------------------------- |
| `telemetry.queue_depth`         | 退役          | 无旧 collector queue                                      |
| `telemetry.dropped_samples`     | 退役          | 不复用旧队列/写库丢弃语义；新浏览器丢弃和采集质量分别记录 |
| `telemetry.flush_duration_ms`   | 退役          | 无应用持久化 flush                                        |
| `telemetry.flush_failure_count` | 退役          | 无旧 writer                                               |
| `telemetry.flush_batch_events`  | 退役          | 无旧 flush batch                                          |
| `telemetry.flush_batch_buckets` | 退役          | 无旧 time buckets                                         |
| `telemetry.backoff_count`       | 退役          | 无旧 SQLite writer 退避                                   |

新方案健康由 scrape `up`/duration、sampler 新鲜度/错误、hotpath 采样状态和浏览器接受/拒绝/丢弃说明；这些不能伪装成以上已删除指标的历史延续。

## 浏览器

| 旧 ID                             | 新指标 / 动作                                        | 口径与来源                                                  |
| --------------------------------- | ---------------------------------------------------- | ----------------------------------------------------------- |
| `browser.data_ready_ms`           | `cvm_browser_data_ready_seconds{page,device}`        | 真实页面数据就绪；不以任意 API completion 猜测              |
| `browser.update_to_paint_ms`      | `cvm_browser_update_to_paint_seconds{page,device}`   | 对应数据更新到可观测绘制                                    |
| `browser.long_task_ms`            | `cvm_browser_long_task_duration_seconds{page}`       | 被接受的 long-task 样本                                     |
| `browser.long_task_count`         | `cvm_browser_events_total{page,event="long_task"}`   | 被接受上报的事件计数；不表示所有访客事件                    |
| `browser.api_request_duration_ms` | `cvm_browser_api_duration_seconds{page}`             | 同源 API duration，排除全部 observability 上报/诊断请求     |
| `browser.api_request_count`       | `cvm_browser_events_total{page,event="api_request"}` | 上述实际接受的事件计数                                      |
| `browser.sse_duration_ms`         | `cvm_browser_sse_duration_seconds{page}`             | 实际连接生命周期；不能依赖尚未结束的 resource 条目          |
| `browser.sse_disconnect_count`    | `cvm_browser_sse_ends_total{page,outcome}`           | 改为 normal / error / unknown，不把所有正常关闭称为异常断开 |
| `browser.unsupported_count`       | `cvm_browser_unsupported_total{page}`                | unsupported 状态，不等于零 long-task                        |
| `browser.visibility_hidden_count` | `cvm_browser_visibility_hidden_total{page}`          | 可观测的 hidden 事件                                        |

## 新增排查能力

新增族均从真实事件记录，不属于旧 77 项历史的转换：

| 能力              | 新指标合同                                                                                                                                                                                                                                                                                                                                                                          |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| HTTP              | `cvm_http_requests_total{route,method,status_class}`；`cvm_http_header_duration_seconds{route,method}`；`cvm_http_body_duration_seconds{route,method}`；`cvm_http_body_ends_total{route,outcome}`                                                                                                                                                                                   |
| 对外调用          | `cvm_proxy_invocations_total{endpoint,outcome}`；`cvm_proxy_phase_duration_seconds{endpoint,phase}`；`cvm_proxy_ttfb_seconds{endpoint}`；`cvm_proxy_ttft_seconds{endpoint}`；`cvm_proxy_stream_duration_seconds{endpoint}`；`cvm_proxy_upstream_attempts_total{endpoint,outcome}`；`cvm_proxy_retries_total{endpoint,reason}`；`cvm_proxy_transfer_bytes_total{endpoint,direction}` |
| SQLite 分层等待   | `cvm_sqlite_queue_wait_duration_seconds{class}`；`cvm_sqlite_pool_acquire_duration_seconds`；`cvm_sqlite_coordinator_hold_duration_seconds{class}`；`cvm_terminal_enqueue_to_commit_duration_seconds`；真实 P1 pending 分类                                                                                                                                                         |
| 全部 managed task | `cvm_task_runs_total{task_key,outcome}`；`cvm_task_run_duration_seconds{task_key}`；实际 backlog/freshness，task_key 来自注册表；不取代维护库的运行记录                                                                                                                                                                                                                             |
| 新观测质量        | `cvm_observability_sampler_last_success_timestamp_seconds{source}`；`cvm_observability_sampler_errors_total{source}`；浏览器 dropped/rejected/accepted 与观察能力，reason 为固定枚举                                                                                                                                                                                                |
| 代码诊断          | 锁定版本的 `hotpath_*` 函数/SQL/路由/锁能力，native Histogram 和样本数；不在 SDK 再复制一套相同函数/SQL聚合器                                                                                                                                                                                                                                                                       |

`endpoint`、`phase`、`outcome`、`reason` 是接口家族/阶段/结果的固定枚举，不能使用完整 URL、模型或账号。body lifetime 与 proxy stream lifetime 的不同起点必须显示，不能互相替代。SSE 的结果和业务 terminal 不等于普通 HTTP response status；相关异常/取消另计。已退役的 WS upgrade 仅为普通 HTTP 501 拒绝，不创建代理 stream 样本。

TTFB、TTFT 和请求/响应阶段遵循 [现有领域口径](../../CONTEXT.md#invocation-timing)。没有首个有效 model delta 的 invocation 不生成零 TTFT；missing/取消/无输出由计数说明。phase 之间可能重叠，不能简单相减得到 CPU overhead。

## 映射验收

- 本表覆盖源清单全部 77 个旧 ID，无遗漏、重复源 ID 或隐式保留旧 writer。
- 9 个旧库/collector 自身 ID 退役，1 个重复 publish duration 合并；其余信号迁移或明确修正口径。
- ms→seconds 只转换一次；Counter 每个来源事件累计一次，不能周期重复加入累计数或 batch 最后值。
- 规范终态、批次、操作和 task run 各用自己的分母；真实等待样本取代窗口平均值，不导入旧均值。
- 静态标签、导出白名单、runtime entry limit、Histogram/采样 count 和 missing 语义经过压力与查询验证。
