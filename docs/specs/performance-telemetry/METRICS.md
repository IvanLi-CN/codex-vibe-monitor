# 性能指标合同

新指标的权威逐项定义是[77 项迁移映射](../../design/performance-observability-metrics.md)，标签、Histogram buckets、抽样和缺测规则是[指标合同](../../design/performance-observability.md#指标合同)。历史 SQLite 注册表保留于 Git 历史；不构成当前应用的存储或查询合同。

源 ID 共 77 项：9 项旧 collector/存储指标退役，1 项重复 publish duration 合并，其余迁移或修正真实来源口径。新增 HTTP、proxy、SQLite 分层等待、managed task 和采集质量指标由真实事件产生，不导入旧时间桶。

应用前缀 `cvm_`，时间 `_seconds`、容量 `_bytes`、事件 `_total`；app classic Histogram 与 hotpath native Histogram 独立抓取和查询。

## 请求诊断指标

所有新指标只使用固定 endpoint（Responses、Chat Completions、Compact、搜索、图像生成、图像编辑、other）及 phase/resource/outcome 枚举。请求身份与 TraceID 不进入标签。应用 recorder 将新系列注册限制为 3996 个物理样本槽，另预留 inflight、两个 CPU mode 和 limiter drop counter；hotpath 抓取最多 1000 系列，实例总预算为 5000。容量满时既有系列仍更新，新系列被丢弃并增加 `cvm_metric_series_dropped_total`，观测变为 degraded。

| 指标                                                                                                             | 单位与语义                                                             | 固定标签 / 分母                                                                                                        |
| ---------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `cvm_request_response_duration_seconds`                                                                          | 应用入口至下游 body EOF、error 或 Drop 的完整累计时长                  | endpoint；每个已观测请求一次                                                                                           |
| `cvm_request_response_ends_total`                                                                                | body 终态次数，complete 不代表 HTTP 成功或客户端收全数据               | endpoint、outcome（complete/error/cancelled）                                                                          |
| `cvm_request_milestone_seconds`                                                                                  | 从同一入口累计至 head、first_byte、model_delta                         | endpoint、milestone；仅已观测里程碑                                                                                    |
| `cvm_request_stage_seconds`                                                                                      | 具名区间，允许嵌套，不能相加                                           | endpoint、phase（request_read/request_parse/auth_route/attempt/connect/upstream_head/forward/finalize/journal_append） |
| `cvm_resource_wait_event_seconds` / `cvm_resource_wait_events_total`                                             | 单次等待区间耗时 / 次数；取消区间为已观测下界，可能延续到响应之后      | resource / resource、outcome（complete/cancelled）                                                                     |
| `cvm_request_resource_wait_seconds`                                                                              | 响应窗口内各资源等待累计，含零；同资源并行等待可重叠                   | endpoint、resource；每个已观测请求一次                                                                                 |
| `cvm_request_wait_affected_total`                                                                                | 同资源至少发生一次等待的请求数，逐请求去重                             | endpoint、resource；分母为对应 response duration count                                                                 |
| `cvm_request_wait_over_100ms_total`                                                                              | 单请求累计该资源等待 >100ms 的请求数                                   | endpoint、resource；分母与 affected 一致                                                                               |
| `cvm_request_local_wait_seconds`                                                                                 | 响应窗口内所有具名本地等待区间的去重并集                               | endpoint；每个已观测请求一次                                                                                           |
| `cvm_request_unattributed_seconds`                                                                               | 响应时长减去已观测阶段及等待区间的并集                                 | endpoint；不能解释为 CPU 或系统调度等待                                                                                |
| `cvm_request_persistence_seconds`                                                                                | 单终态命令从内存入队至 commit 或关联不可用的时长                       | endpoint、outcome（committed/coalesced_unavailable/association_unavailable）；不加到响应时长                           |
| `cvm_diagnostic_dropped_total`                                                                                   | 活跃上下文、详细记录或 attempt 上限导致的诊断损失                      | reason（active_limit/detail_limit/attempt_limit）                                                                      |
| `cvm_trace_exported_spans_total` / `cvm_trace_export_failed_spans_total` / `cvm_trace_queue_dropped_spans_total` | SDK 批处理管线导出成功、失败和有界准入丢弃的 span 数，均为独立质量信号 | 无动态标签；不等价于完整 trace 数                                                                                      |
| `cvm_trace_active_contexts`                                                                                      | 仍由任务或内存队列持有的诊断上下文数                                   | 无动态标签；上限 1024                                                                                                  |

资源枚举为 sqlite_coordinator、db_pool、account_capacity、retry_backoff、downstream_channel、terminal_priority、journal_lock。downstream_channel 只在 `try_send` 发现容量已满后记录实际等待，避免把立即发送归作背压。前台 DB 获取与 begin 测量的是 API 等待区间，不声称排除了驱动内部执行。connect 包括当前可见的 DNS/TCP/TLS/代理建立区间，不拆出无法单独观测的成本；attempt 区间结束于上游 head，body 转发另列。

完整响应、阶段、累计里程碑、每请求等待、未归因与落盘使用 16 个有限 buckets（秒）：`0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30, 60, 120, 300, 600`；其余短等待保留既有 short buckets。统计页同窗计算 p50/p95/p99、平均值与样本数；不同分位数、事件和请求分母不得混用。

响应 root 的 count/sum/max 及八个最长已结束等待区间是有界明细；响应关闭时仍在等待的区间累计到该时刻，root 标记 lower_bound，未结束事件的 max/count 尚未成为完整值。非流式、非文本、前置拒绝或不支持 model delta 的路径标记 TTFT not_applicable；应观测但未见有效 delta 的成功流标记 unobserved。落盘完成可晚于响应 root；共享 coordinator、pool 和执行工作仅记录一次，并通过 span links 关联，不能给每条请求复制独占成本。
