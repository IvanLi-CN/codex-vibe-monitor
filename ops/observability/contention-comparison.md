# 维护竞争前后比较

本说明固定观测版本与性能修复版本的生产比较口径；受控性能验收继续使用 CI 或当前任务的 Agent VM。生产只读取自然流量和维护活动，不补造请求或触发归档、补算。指标语义见[指标合同](../../docs/specs/performance-telemetry/CONTENTION-METRICS.md)。

## 窗口与版本

观测应用和 Grafana provisioning 均已核对生效后记录 UTC 起点。立即开始性能修复开发、测试和审查；至少两小时且覆盖归档及补算活动的基线只约束修复上线。两小时没有实际活动、数据仍低样本或采集有缺口时，记录条件不足，不把经过时间当作有效基线。

每次比较在私有运维记录中保存以下字段，不将生产实例身份、凭据或原始请求写入本仓库：

| 字段 | 内容                                                                                                         |
| ---- | ------------------------------------------------------------------------------------------------------------ |
| 时间 | baseline/fix 的 UTC `start`、`end`、窗口秒数和查询 step；使用绝对时间，关闭自动刷新                          |
| 应用 | 实际运行镜像 digest、OCI revision、应用版本及各次重启的 UTC 时间                                             |
| 图表 | 实际 provisioned 的 `cvm-sqlite`、`cvm-proxy` JSON 与 SHA256；Grafana 数据库 revision 另记，不替代源文件版本 |
| 查询 | 查询定义所在提交 SHA、面板 ID/refId、变量替换后的完整 PromQL；两窗口沿用同一查询定义                         |
| 筛选 | 相同 service/environment；按实际 instance、operation、endpoint 分层，记录实例替换及任务开关                  |
| 配置 | 有关批次、预算、任务启用和观测开关的脱敏配置差异；独立标出索引准备窗口                                       |
| 质量 | 抓取间隔、可用率、缺口、采样器新鲜度、计数器重置、观测降级/丢弃及各分位数的样本数                            |

使用只读 Grafana Viewer 身份和固定 datasource UID `cvm-prometheus`。完整 PromQL 以绑定版本的 dashboard JSON `panels[].targets[]` 为准，避免另抄一套查询后漂移。窗口汇总用 `/api/datasources/proxy/uid/cvm-prometheus/api/v1/query`，将 `time` 固定为窗口结束、`$__range` 替换为该窗口秒数；趋势用 `query_range`，固定 `start/end/step`。后者的 `increase(...[$__range])` 是每个评价点向前看的滚动窗口，只有结束点覆盖所选完整窗口；不能把这些滚动结果再相加。速率图的 `$__rate_interval` 两窗口保持一致。精确选择目标标签，变量中的正则特殊字符须按 Grafana 实际展开方式转义。

`scripts/cvm-observe` 的现有指标入口使用相对当前时间的窗口，适合排障；不能代替这里的绝对 UTC 基线。调用固定 Grafana HTTPS proxy 的凭据文件用法见[访问合同](README.md#公网只读机器访问)，不要将 Token 放到 URL、查询文件或报告。保存查询结果及 API warnings；请求成功但结果为空仍是 unknown。

## 固定查询集

下表是 `cvm-sqlite` 和 `cvm-proxy` 的持久面板 ID；A/B 比较都读取同一版本的表达式，窗口汇总按上述固定结束时刻执行。历史 offset 图仅用于浏览；正式比较分别评价两个已记录的绝对窗口，不假定部署恰好发生在一天或七天前。

| 功能模块       | 面板 / 查询                                                                                 | 必须一起报告                                                                                                                                       |
| -------------- | ------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| 归档维护       | sqlite 201–207：各阶段 p95、样本、超过 250ms/1s/10s 次数、提交行/秒、积压数/年龄、批次结果  | 选择 operation；`write_hold` 与其内部阶段重叠，不能相加；同时保留批次结果和已提交行数，暂停或少做工作不算改善                                      |
| 后台补算       | sqlite 211–217：通知、真实尝试、待确认义务、年龄、下次尝试、维护库结果与 p95                | 通知与尝试分开；按固定 task 和 operation/outcome 分层，单独列 BUSY/LOCKED；待确认义务不是全量历史待补算行数                                        |
| 统计看板       | sqlite 221–227：Summary 结果/原因、成功 p95/p99、样本、固定 SQL、投影待处理量和最后发布时间 | 失败和降级不能并入成功延迟；缺口按 query/phase 原单位报告，不将不同投影计数相加当作全库积压                                                        |
| 归一化成本     | sqlite 231、232                                                                             | 每千完成代理请求、每千提交行的归档写许可秒；另外保存分子秒数、完成请求数、提交行数及窗口长度；零分母为 unknown                                     |
| 采集质量       | sqlite 233、236；overview 采样器/观测健康面板                                               | 抓取最小值不能证明无缺测，需核对样本密度、新鲜度及降级/丢弃；样本不足时保留 N 和趋势，不宣称稳定分位数                                             |
| 代理与终态保护 | proxy 102、112–114，以及原有响应延迟/终态调用数                                             | 真实 enqueue→commit 与已确认追踪终态数/年龄、日志待 ACK、运行中 attempt、容量、准入保护计数分别报告；恢复年龄为下界，累计 Gauge 不用 rate/increase |

为便于复查，在固定窗口结束点补存下列绝对量。`S` 仅表示与面板相同的精确 service/environment/instance 选择器，执行时替换为真实标签；归档量额外选固定 operation，代理量按 endpoint/outcome 保留分层。

```promql
sum by (operation) (increase(cvm_retention_phase_duration_seconds_sum{S,phase="write_hold"}[<window-seconds>s]))
sum by (operation) (increase(cvm_maintenance_processed_rows_total{S}[<window-seconds>s]))
sum by (operation) (increase(cvm_maintenance_archived_rows_total{S}[<window-seconds>s]))
sum by (endpoint,outcome) (increase(cvm_proxy_invocations_total{S}[<window-seconds>s]))
sum by (operation,outcome) (increase(cvm_retention_batches_total{S}[<window-seconds>s]))
sum by (task,outcome) (increase(cvm_backfill_wake_attempts_total{S}[<window-seconds>s]))
sum by (outcome,reason) (increase(cvm_summary_responses_total{S}[<window-seconds>s]))
```

Gauge 积压至少保留窗口起点、终点和最大值；年龄同时保留 count，确认零积压后才解释年龄零。只比较成功延迟会掩盖快速失败，只比较吞吐会掩盖积压增长。Counter 使用逐序列 `increase` 后汇总；重启可能丢失最后一次抓取后的事件，须记录边界缺口，不能把重置解释为业务计数下降。

## 比较结论

每项输出 baseline、fix、绝对差值、相对变化、样本数及采集限制。baseline 为零或 unknown 时不计算相对百分比。p95 至少 100 个成功样本、p99 至少 1,000 个成功样本；维护活动低频时仍可报告单次占用、预算超限次数、处理量及积压，不放宽门槛。

自然负载不匹配时，按 operation/task/endpoint、工作量、错误比例和配置分层作描述性比较；归一化也不能消除所有混杂因素，不宣称等负载因果结果。受控 CI/VM 比较另列，保留环境、同数据、任务启用、完成工作量和原有时间断言。索引构建、重启恢复和稳态窗口分别报告。
