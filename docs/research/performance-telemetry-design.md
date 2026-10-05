# 运行性能指标设计

本文件记录被取代的本地 SQLite 性能遥测设计，保留其风险依据与原始口径供迁移核对。目标设计以 [外部性能观测设计](../design/performance-observability.md) 和 [ADR 0025](../adr/0025-external-performance-observability.md) 为准；当前代码与旧 Spec 仍描述切换前的实现，不能据此恢复旧采集或存储方案。

## 目标与边界

本设计让系统能回看读模型、SSE、持久化、维护、进程资源和浏览器体验之间的性能因果链。代理请求本身的阶段耗时、结果和重试已经是主库调用明细的职责，不在独立指标库重复采集。性能指标存入独立的本地 SQLite 文件，由同进程的低优先级汇总器批量写入。它不能参与代理请求、P1 terminal ACK、Dashboard 实时投影或主库写协调器的完成条件；指标库故障不得改变业务结果。

已确认的隔离、留存与维度取舍见 [ADR 0019](../adr/0019-isolated-bounded-performance-telemetry.md)。

现有调用记录及 `proxy_perf_stage_hourly` 是调用级历史事实，已有 System Status 是当前健康快照。新库记录跨功能、跨层的运行趋势，不复制调用明细，不替代主库的精确业务统计，也不为采集回扫主库、raw 文件或 archive。系统状态页的内存快照仍是故障时的独立诊断入口。

## 风险依据

| 已知证据                                                                                                                                                                                                                                           | 对指标的要求                                                                                                   |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| [PR #492](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/492)、[#496](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/496)、[#500](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/500) 曾处理 Dashboard 热扫描、重复写和 CPU 压力 | 记录后台构建耗时、构建次数、主库读写压力和活跃订阅数；调用明细继续作为单次请求的性能事实。                     |
| [PR #713](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/713)、[#923](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/923) 引入 terminal journal 和分级写入协调                                                                         | 分开记录 P1/P2/同步写/maintenance 的等待、积压、ACK 与 cursor lag；将 pressure defer 和真实 BUSY/LOCKED 区分。 |
| [PR #730](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/730)、[#731](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/731) 处理 raw 重复存储和内存归因                                                                                  | 记录 RSS、匿名内存、Swap、已知组件估算、未归因量、raw/spool/数据库体积和恢复积压。                             |
| [PR #1011](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1011)、[#1013](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1013) 处理浏览器渲染长任务                                                                                     | 后端指标不能代替真实页面的 data-ready、渲染和 long-task 观测。                                                 |
| [issue #856](https://github.com/IvanLi-CN/codex-vibe-monitor/issues/856) 及 [runtime recovery Spec](../specs/runtime-read-model-pressure-recovery/SPEC.md) 指出恢复与迁移风险                                                                      | Summary hydration 的输入量/结果、pressure 唤醒/延后、actionable 与 blocked backlog、修复进度都应可回看。       |

线上只读核对（2026-09-26，srv-101，运行版本 `2.73.0`）看到 `/health` 返回 200，但 `/api/system/status` 连续返回 503；两次 `docker stats --no-stream` 的容器 CPU 瞬时读数为 301% 和 493%，内存约 712–714 MiB。最近 15 分钟日志约 5,348 行，其中 pressure/defer 关键词匹配约 223 行、lock 相关约 3 行。这些是单点样本和可能重复的文本匹配，不是平均负载、事件精确计数或根因证明。它们表明当前最需要保留下来的证据是 CPU 与诊断接口可用率、SQLite pressure/lock、等待队列年龄以及投影恢复进度；指标库自身必须在状态接口不可用时仍能继续采集。

历史 PR 说明过风险类型，但不能将当时的部署状态推定为当前状态，也不能把当前的 503 归因于某个历史 PR。

## 指标口径

每个指标声明一种类型，并带采样覆盖率：

- **事件计数**：defer、丢弃、build、flush、锁等待和发布失败等系统事件。事件源只更新内存计数；汇总器写时间桶。重启或进程中断造成的缺口标为缺测，不补零。
- **耗时分布**：`count + sum + max + 可合并固定桶直方图`。从原始桶合并出 p50/p95/p99；低样本量同时显示 `count` 和 `max`，不平均各桶 p95。单次调用与上游 attempt 保持不同分母。
- **存量采样**：队列深度/字节、oldest age、RSS、Swap、磁盘占用、活跃订阅。每桶保留 `sample_count + min + max + 时间加权平均/末值`；只有既有计数器或常量开销来源才参与。
- **进度/新鲜度**：最后成功时间、ACK lag、投影 last-good age、cursor lag、最老 actionable backlog age。这里的 `lag` 统一是观测时刻到已完成水位的时间差；纯 row-ID 差单列为 `cursor_gap_rows`，不伪装成秒数。
- **覆盖质量**：每桶记录有效采样数、期望采样数、丢弃数、采样率与数据源状态。`unknown`、`deferred`、`blocked` 和真正的 `0` 分开；零流量时延迟分位数为空。

代理 `请求用时`、TTFT、`响应耗时` 沿用 [CONTEXT.md](../../CONTEXT.md) 的已有定义。上游尝试耗时与对外调用耗时分别聚合；服务自身耗时必须由明确阶段测量，不能用“总耗时减上游耗时”猜测异步/流式路径的开销。

## 采集清单

| 功能 / 层           | 主要指标                                                                                                                                                                                            | 采集点和细分                                                                                                                                                                                                                                        |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 服务运行入口        | 在途请求数和服务进程资源                                                                                                                                                                            | 只维护有界原子计数，不按请求写入遥测事件；单次代理阶段耗时和结果沿用主库调用明细。                                                                                                                                                                  |
| P1 与主库           | P1 入队至 commit ACK、队列条数/字节/最老年龄、短批大小与事务耗时、coordinator 四类写入等待、pool acquire、BUSY/LOCKED、pressure defer/实际 retry、主库 WAL/文件大小                                 | 事件计数与独立队列采样结合；把排队、SQLite 执行和 ACK 发布分开，记录 P1 是否超过业务可见目标。                                                                                                                                                      |
| P2 与读模型         | 派生 flush 次数/耗时/延后、terminal 到 Dashboard/timeseries/long-term 各 consumer 的 cursor age、Summary hydration/rolling/reconcile 耗时与结果、读取行数/接纳字节、last-good age、精确 unavailable | 使用现有 consumer 与 health 计数器，按有限 projection/trigger/source/outcome 分类；单独观测旧数据修复与正常增量。                                                                                                                                   |
| Dashboard 与 SSE    | current/network/terminal 切片排队到发布耗时、cadence miss、topic 构建/序列化耗时与 frame 字节、活跃 subscriber、lag/skipped/replay fallback、首次快照与更新可见时延                                 | topic 使用固定家族名，不包含账号、选择条件、游标或订阅者 ID；区分 owner 无订阅时的正常停工。                                                                                                                                                        |
| 历史 API 与页面读取 | 由现有主库调用明细和 System Status 负责                                                                                                                                                             | 本指标库不重复记录历史 API 请求量、耗时、状态或响应大小；读模型内部开销只在投影/SSE、SQLite 和维护分层中记录。                                                                                                                                      |
| 维护与数据量        | archive/retention/raw compression/backfill/rollup/repair 的运行耗时、处理行数/字节、有效进度、actionable/blocked backlog 数与最老年龄、due/defer 原因、raw/spool/archive/主库容量、剩余磁盘         | 复用现有 inventory 和 cursor；采集器不得为了采样触发目录遍历或主库全表计数。                                                                                                                                                                        |
| 进程与观测器        | CPU core-seconds/采样间隔与 cgroup quota、RSS/anon/file/Swap、cgroup 当前值/上限、线程数、managed/unattributed bytes、指标队列占用/丢弃/写失败/批量 flush 耗时/指标库 WAL 与文件大小                | `/proc`/cgroup 和现有无克隆组件估算按低频读取；CPU 百分比同时标明相对单核或 quota 的分母，观测器自我开销必须能从 System Status 与日志看到。                                                                                                         |
| 真实浏览器体验      | Dashboard/Records/System 页面 data-ready、更新到 paint、长任务总时长与次数、页面可见性、请求和 SSE 首帧/断线时长                                                                                    | 同源、授权用户的低频聚合上报；每次页面进入都观测，但每页最多每分钟发送一份汇总。核心渲染时延按页面家族与设备档保留，长任务、资源请求、SSE 与能力不支持按页面家族聚合，以控制序列预算；不传 URL 参数、用户标识或页面内容。浏览器未上报时标为无覆盖。 |

## 颗粒度与留存

| 时间跨度           | 时间桶 | 目的                                                    |
| ------------------ | ------ | ------------------------------------------------------- |
| 最近 7 天          | 1 分钟 | 对齐请求尖峰、P1 写等待、投影 cadence 与 CPU/RSS 变化。 |
| 第 8 至 30 天      | 5 分钟 | 保留事故趋势和跨层相关性，限制行数。                    |
| 第 31 天至 13 个月 | 1 小时 | 观察发布前后、负载与容量长期趋势。                      |

1 分钟桶在内存中更新，10 秒读取轻量队列/计数器，30 秒读取进程内存与文件 metadata，最长 60 秒批量持久化一次。事件耗时是常量时间的本地直方图加法；浏览器上报在服务端再聚合。跨层关联仅靠同一 UTC 时间桶、进程启动代与发布版本，不保存每请求 trace 或高基数关联 ID。升层 rollup 先合并计数/分布/存量统计，再删除已覆盖的细桶；近期序列在 30 天边界停止进入小时层，长期序列继续保留至 13 个月；保留窗口按 UTC 时刻裁剪。进程异常退出最多可能失去一个 flush 窗口，重启后显示缺口，不回填估值。

长期小时桶仅保留核心序列：CPU/RSS/Swap、P1 ACK 与主库 pressure、P2/投影 lag、SSE 可见性、维护积压、存储用量、采集覆盖质量。代理 endpoint、stage、结果和历史 API 请求明细不进入该库。这样长期趋势不会随着业务调用量累积大量稀疏序列。

当前注册表按排查路径覆盖以下固定序列族：

- 服务入口：HTTP 在途请求数；代理请求阶段、结果、重试和字节数沿用主库调用明细，不复制到遥测库。
- 队列与 SQLite：P1/P2 深度/字节/ACK/延迟/重试/转移、pressure defer、LOCKED/BUSY/连接超时、四类 coordinator 等待者/等待耗时/绕过与 maintenance fairness、写批次耗时/行数/字节、主库和遥测 WAL。
- 读模型与 SSE：投影构建/读库/最近一次健康年龄、reconcile 的次数/耗时/失败/pressure defer、current/network/terminal 三类 revision/cadence miss/publish、快照和帧字节、发布耗时与发送失败、活动订阅数。
- 维护与主库边界：维护积压/运行耗时/处理和归档行数、raw 压缩前后字节/文件处理量；历史/统计/系统/设置接口请求量、耗时和状态结果继续由主库或现有状态接口负责。
- 资源与浏览器：CPU/RSS/匿名内存/Swap/托管内存/线程/磁盘余量、主库/遥测库/WAL 大小、采集队列/丢弃/flush/退避；页面 data-ready、更新到 paint 按页面与设备，long task、同源 API/SSE 资源耗时、断开和切入后台次数按页面聚合。

仅允许静态注册的操作、阶段、结果和少数离散分类。默认不按 account、model、host、proxy 节点、invocation、conversation、IP 或任意 route 参数建时间序列；这些问题由已有调用/attempt 明细在明确时间窗口中定位。高基数标签即使 hash 也不准入。先为每类指标设最大 series 数和典型容量预算，再以代表性规模验证 CPU、RSS、指标库文件/WAL 与主库 p95、P1 ACK 是否受影响。

## 存储模型

- `metric_catalog` 是代码内固定注册表，定义 ID、单位、计量类型、允许的离散维度和直方图边界；数据库不接受任意 metric 名或标签。
- `metric_bucket` 以 `(UTC bucket_start, resolution, metric_id, dimension_code)` 唯一定位汇总行。按类型存计数/总和/最大值、可合并直方图或存量统计；同一指标的三个分辨率不能在同一次查询中重复相加。
- `collector_epoch` 记录进程启动代、部署 revision 与有效观测窗口；`collector_health` 记录 flush、失败、丢弃和采样缺口。部署与重启标记不包含请求身份。
- 不保留原始事件表；按小批量删除过期桶并按需 checkpoint WAL。容量不是靠“独立数据库”自然受控，而要通过 series 白名单、留存清理和代表性负载下的文件大小验收。

## 写入与失效设计

采集器在热路径只提交定长事件或更新无阻塞计数器；满队列时丢弃低优先级样本并累计丢弃数，不能阻塞代理或 P1。独立汇总任务使用单 writer、短事务、固定批量与自己的连接池/WAL；不获取主库的 write permit，也不因指标库错误重试风暴。重复失败后有界退避，System Status 暴露 `healthy/degraded/unavailable`、最后成功写入时间、缓冲占用和丢弃数。进程退出尽力 flush，但不延长业务停机预算。

指标库与主库路径必须不同；文件权限、备份排除/包含策略和清理边界明确配置。查询指标只读本库，并提供系统工作区中的性能总览与按层下钻：先看请求体验和资源压力，再展开 P1/P2、读模型/SSE、维护与浏览器体验。图表显示采样覆盖率，时间轴标记部署/重启，避免把无观测的空档解释为健康。

## 验证与实施边界

验证应覆盖：代表性请求与订阅负载下的采集开销 A/B、主库锁压下的指标库降级、指标库磁盘满/损坏后的业务连续性、重启缺口与 rollup 正确性、30 天数据的查询/清理成本、浏览器无支持或离线时的覆盖提示。生产门槛沿用 [system-design.md](../system-design.md) 的 Dashboard 更新、CPU、RSS 和 Swap 目标；采集器的新增开销需要单独测量，不能从“独立文件”推定为零。

已确认采用 **7 天 1 分钟 + 到 30 天 5 分钟 + 到 13 个月 1 小时**，并维持低基数全局视角。若后续事故排查证明必须按账号/模型持续分组，应先定义有上限的短期诊断机制，再重新估算容量与采集成本；不能直接扩展时间序列维度。
