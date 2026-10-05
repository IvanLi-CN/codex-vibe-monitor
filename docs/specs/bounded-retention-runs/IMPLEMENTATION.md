# Retention：实现映射与验证

## Current Status

本轮实现 REQ-BRR-022..023 / VER-BRR-012，容量对照基线为 `162253adecf9dba913202201041846f9062e713c`；开发分支为 `th/retention-task-local-monthly-batches`；候选的实际 main 基线与 head 绑定在当前交付证据中。主人授权将唯一直接 PR 推进到合并；不包含生产部署、生产修复或本地清理。

代码实现任务内月度文件批次、短事务源记录转换及可选吞吐展示。50 倍新增速率、固定存量 24 小时归零及在线延迟不劣仍需当前候选发布构建实证，尚未签收。主人明确以其他验证通过、推送后 GitHub Actions 必过检查通过及 PR 可合并作为本次交付条件，暂不在本地运行 stateful profile。性能试验只允许在 GitHub Actions 单独执行，不得在本机或 Agent VM 运行，也不得将不必要的性能试验加入通用 PR 必过测试集；历史容量卡不能冒充本次通过证据。

## 任务内批次与月度目标

每轮按数据集选取最多 1000 条符合保留策略的 live 候选，按月份分组，结合实际 payload 字节、现有 gzip 文件大小、可用磁盘和 60 秒设计预算决定是否开始文件批次。正常目标为数百到数千条；尾部、管理员配置或 payload 上限造成不足 512 条时记录原因。规划拒绝不通过缩成单条伪装成果。

同一 dataset/month 继续使用 `archives/<dataset>/<year>/<dataset>-YYYY-MM.sqlite.gz`。迟到记录写入相同路径。月份决定目标文件，不要求任务完成整个月份的全部积压。下一轮重新选择剩余 live rows，不读取上一轮临时文件、累计值或候选游标。

当前任务创建带 PID/唯一序号的临时 SQLite 副本，批量写入已选记录，一次校验内容和身份、一次压缩、一次原子替换目标文件。临时 SQLite、journal/WAL/SHM 和压缩中间文件有任务内清理 guard。工作文件的内核锁识别实际活跃所有者，不依赖可复用或跨容器命名空间的 PID；创建及废弃清理使用短暂、非阻塞的目录锁，文件准备期间只保留工作文件锁。清理只接受规范目标下三个数字组成的任务文件名，在磁盘预检之前丢弃已无内核所有者的临时文件及孤立 sidecar；不读取其中数据，不续作它们。

文件发布后按现有 manifest、摘要及 Verified Archive 证明转换源记录。独立源连接不回收到连接池；归档 ATTACH 的文件 URI 显式使用磁盘模式，避免共享内存源把目标文件也建到内存。主库写入每次最多 64 条；全部内容证明在事务外准备，短事务复核源身份、证明元数据和 raw 引用后提交。已提交微批次保留其完整事实；失败后未提交源行仍为 live，下一轮重新选择。小时物化进度的前缀复核使用有界存在性检查，避免持有 writer permit 时 COUNT 百万行；运行详情只保存阶段错误与指纹，不输出源错误中的路径或敏感内容。已发布文件中的 live 副本按身份重新覆盖，不能作为已删除源行的证明替代品。旧目标的恢复备份只保护文件替换异常，不用作下一轮数据 staging。

普通任务将旧 preparing/published prepared 行隔离为现有 `quarantined`，不解析其 source IDs 作为正常归档输入。既有 manifest、completed archive 和 cleanup 事实保留；历史恢复工具仍可读取旧状态。主库不增加表、列或状态枚举值。

准入等待受本轮剩余超时约束；取消只移除尚未开始 SQLite 工作的协调器 waiter，不抢占 P1 或改变公平规则。兼容元数据的压力旁路也使用同一约束，避免归档未开始时等待超过整轮兜底时间。没有旧 prepared 行时不提交空隔离事务。

每轮通过 task-local 保存首次实际准入拒绝原因，供不可变运行结果使用；后续阶段成功提交不会擦除该原因，下一轮也不会继承历史恢复或共享写状态的原因。后台槽占用保留 `background_busy`，不能误报为 SQLite 压力。

主库的可选 `PRAGMA optimize` / checkpoint 使用独立连接、至多 2 秒且不超过整轮剩余时间的执行期限、匹配的 busy timeout 和每 1000 VM 指令检查的真实 SQLite progress handler。执行完成后移除 handler 并等待连接关闭，再释放后台准入；不把取消中的连接还池。维护语句的取消不撤销已完成归档，也不伪装成整轮超时，日志记录 `sqlite_maintenance_query_budget`；连接清理失败不能被预算取消掩盖。回归测试验证慢写语句取消后原值保留、独立 writer 能立即取得锁。

容量排查发现在线 terminal 写入复用大历史 Prompt key 时，原工作集触发器的 OR 条件会对历史正文反复计算展示状态，持续占用 P1。查询改为现有 key/时间索引上的近期范围，以及 `invocation_in_progress_live` 的旧活跃 ID 和当前变更 ID；仍回到源记录复核 key、时间和展示状态。当前 ID 覆盖 INSERT/UPDATE/DELETE 的触发器执行顺序，两个时间范围不重叠。此优化保留实时工作集口径，不调用会话历史统计物化。

## 模块映射

| 需求                | 当前源码                                                                                       | 验证口径                                                                               |
| ------------------- | ---------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| REQ-BRR-002/022     | `src/maintenance/retention/batch_plan.rs`, `task_batches.rs`                                   | 预算参与批次规划；运行超时只作安全兜底，重复超时是缺陷信号                             |
| REQ-BRR-022         | `src/maintenance/archive/writers.rs`, `quota_compaction.rs`, `retention/archive_identity.rs`   | 同月规范路径、任务临时文件清理、身份冲突和内容摘要、未证明源行保留                     |
| REQ-BRR-022         | `src/summary_source_change.rs`, `retention/batch_plan.rs`                                      | 完整 V2 Snapshot 验证在写事务外；短事务比较元数据后发布，Summary 不重复计入 live 副本  |
| REQ-BRR-023         | `src/maintenance/retention.rs`, `src/runtime.rs`                                               | batch rows、已提交行、文件准备、锁等待、整轮有效服务速率、24h 到达速率、倍率及超时次数 |
| REQ-BRR-023         | `web/src/lib/api/core-foundation.ts`, `RetentionRunThroughput.tsx`, `SystemTaskDetailPage.tsx` | 新字段可选；真实零与缺失区分，旧响应显示未知；Prompt 统计仍暂不可用                    |
| REQ-BRR-016..021    | 既有 runtime、maintenance_store、小时 observer 和趋势组件                                      | 单一 owner 自动追赶、禁用控制、准确小时双指标保留；本轮不新增调度器或改动小时观测定义  |
| REQ-BRR-003/005/015 | 既有 Prompt cache 物化所有者                                                                   | 独立持久化队列继续异步处理；不放回归档事务，不改变精确统计读契约                       |
| REQ-BRR-002/004/023 | `src/schema.rs`, `schema/prompt_cache_working_set_triggers.rs`, `retention.rs`                 | 大 key 在线工作集不扫描历史正文；准入超时取消 waiter，P1 优先和现有迁移事实保留        |

## 观测与 API

维护库原有 run details JSON 可选增加 `archiveBatches` 和 `timeoutCount`，不新增维护库结构。每条批次记录 dataset/month、batchRows/committedRows、filePrepareMs、lockWaitMs、elapsedMs、committedRowsPerSecond、arrivalRowsPerSecond、serviceRateMultiple 和 smallBatchReason。

服务速率使用本数据集实际提交行数除以整轮 elapsed，包含其他阶段、准入和收尾，避免只用压缩的短时速度。到达速率在独占连接上以真实 SQLite progress handler 的 2 秒预算读取最近 24 小时行数；不可观测则为未知，真实零到达时倍率未知。运行结果一经持久化，后台统计刷新不改写它。

既有工作量观测继续使用本轮起点的固定 cutoff/MAX(id) 范围。新批次路径累计去重后的 invocation 候选 ID，并在每个主库微事务提交后增加 processed；文件发布本身不计作源行完成，attempt 不混入 invocation 指标。批次中途失败仍保留已提交数量。Demo 和 Storybook 的工作量与吞吐取自同一运行样本及实际运行时长。

主页面继续显示 7 天逐小时待归档 invocation 数量及最长逾期。attempt、Prompt 会话、raw 文件按原单位独立计量。页面读维护库和运行结果，不同步扫描主库或触发物化。

main 的外部观测系统拥有指标历史与 Grafana 入口；任务页保留其 `ObservabilityTaskLink`，不恢复已退役的 `ManagedTaskDetail.performance` 或旧性能指标库。本轮 `archiveBatches` 和 `timeoutCount` 仍属于维护库中的不可变业务运行结果。

## Compatibility and Migration

本轮不增加主库表、列、状态枚举或迁移标识。三个现有工作集触发器通过事务替换定义；启动只检查 sqlite_master 中的定义是否符合优化形式，该定义更新不扫描历史或重建投影行，也不改写已部署的迁移完成事实。已有月度 gzip SQLite、manifest、V2 Summary Snapshot、raw 链接和完成状态可读；归档文件内部增加查询索引不改变格式。旧 prepared 状态隔离和当前任务源行转换属于运行 DML。历史吞吐缺失保持未知。

新增任务 JSON 字段和 Web 归一化向后兼容。API 与持久化影响分开评估；本轮记录见 [version impact](assets/task-local-version-impact-record.json) 和 [state compatibility](assets/task-local-persistent-state-record.json)。最终分类由当前候选兼容验证决定；旧 PR 的 minor 记录仅作为历史。

当前同 Minor 兼容来源为 v3.1.0：现有 manifest 模块以及五个编解码、证明和查询函数与其源码逐一比较；v3.0.0 指纹保留为较早 Minor 证据；v2.85.0..4 与 v2.86.0..2 的指纹保留为历史较早 Major 证据。源码等价性不能替代当前候选的文件往返、状态升级和运行验证。

## Verification

- 发布构建试验使用 1,270,000 条过期 invocation、1.2 倍 attempt、500,000 行倾斜 key、十个月份、约 3 KB payload、共享 raw 链接和稀疏孤儿，执行真实文件发布及主库转换。
- 普通新增负载采用 30,000 invocation/day 与 36,000 attempt/day；基线与候选使用相同 fixture 和请求序列，各重复三次。请求按固定时钟独立发出，包含排队延迟并等待已发请求完成，不因慢写入跳过计划到达。窗口模式的负载发生器独立截止，避免旧基线的准入等待无限延长发压。cohort 计数使用现有覆盖索引，避免验收脚本扫描大行正文。要求真实固定 cohort 归零，不能仅按短时速率外推 24 小时。
- `retention_task_local_service_rate_release_benchmark` 是独立的 GitHub Actions 发布构建长时试验，保持 ignored，不进入通用 PR 必过 profiles。单次命令最长 25 小时，其中容量计时上限 24 小时，额外时间仅用于建数和最终文件证明。每次保存逐轮 JSON、最终 cohort、实际文件摘要和在线延迟；其容量结论不由普通功能测试替代。
- 在线探针覆盖聚合、列表、详情和 P1 terminal 写；生产 HTTP 方法比例暂无准确观测，当前 3:1 读写重放是显式保守假设，不能写成生产实测比例。持续峰值用相同发布构建和序列的 20 倍到达速率观察让行与在线等待；磁盘边界在独立 32 MiB tmpfs 中运行拒绝夹具，确认 1000 条未证明源行保留且没有归档发布。锁释放由取消后的独占连接与写锁回归验证。
- 容量夹具同时在同一主库运行真实 Prompt 会话物化所有者，共享后台准入槽和 P2/P1 写协调器，使用生产的 2000 行扫描上限、3 秒运行上限与 15 秒续作间隔。锁竞争按现有压力冷却及 active interval 重试，非压力异常仍使试验失败；结果记录实际扫描/更新、延期及压力失败次数；只重放在线探针的试验保留为诊断证据，不能替代含后台竞争的当前候选验收。
- 容量通过要求 invocation >=17.4 rows/s、attempt >=20.8 rows/s、普通负载不频繁超时，以及在线 p95/p99 中位数不劣于基线。
- `retention_task_local_batches` 覆盖同月重复更新、主库结构不变、取消后的 ATTACH 连接/临时文件清理、坏文件保留源行、旧 prepared 隔离和跨数据集失败计数。第二个源数据事务失败夹具验证已提交 64 行准确报告、工作量样本发现 1000 行但仅完成 64 行、未提交源行保留、Summary 精确总量及下一轮重新选取；不创建续作 journal。
- 工作集回归比较近期/旧活跃/变更身份/失败状态/删除与准确全量参考；万行历史下用真实 SQLite VM 指令预算拒绝退回历史正文扫描。触发器定义识别、迁移事务中断、重启及原完成事实不改写分别验证。
- 工作文件回归用真实持锁子进程验证活跃所有者保留、终止后内核释放、PID 复用下废弃清理、目录竞争立即延期及非规范文件名保留。受限磁盘夹具还验证废弃工作在空间预检前清理，源行和 manifest 不变。
- 后端按仓库 runner 顺序执行 lightweight、stateful-sqlite、archive-file-io 三个资源 profile，并验证 fmt/check/Clippy 和 source-quality。CI 和实测绑定候选 SHA，不能用旧分支结果替代。
- Web 验证包括旧 API 字段兼容、Demo 真实零/未知值、全量 unit/typecheck/lint/build、六个吞吐状态及 SystemWorkspace Storybook、任务页桌面/移动交互 E2E。视觉确认不代替功能或容量验收。
- Prompt 会话事件过滤的既有单测等待实际筛选内容完成渲染后检查原断言；单靠两次 Promise flush 不代表异步事件请求已完成。此测试同步修正不改变产品逻辑、超时或 retention 的验收口径。主线整合后的后端输入使用逐文件摘要证明，文档或 Web 测试提交不得冒充重新编译的后端提交。
- Prompt 统计代次与重启回归在让行检查时读取已提交 staging，确定性地停在首个 256 行页；独立异步观察者可能在负载下晚于数页提交，不能保证该测试所需的精确边界。原聚合、代次、恢复和页游标断言保持不变。
- main 的 SQL 观测序列化夹具使用实际工作集 SQL 生成器及 `NEW.id`，与优化后的触发器身份参数一致；保留原长语句、脱敏和序列化断言，不更改生产观测或归档行为。
- 正式 Tier 4 四固定 lane + database-migration 只读审查在当前候选的 GitHub Actions 必过验证和适用视觉门禁完成后启动。本次交付不将尚未完成的容量实测标为通过；后续性能验证独立记录。

## Visual Evidence

本次 Task Evidence Set 是纯前端 Web Demo 的吞吐卡片：桌面 1440×1050 和移动 393×852 viewport。截图保留 invocation/attempt 单位、整轮服务速率、到达速率、倍率、超时、准备和锁等待，并显示 Prompt 暂不可用。图片已裁掉外围空白后通过 owner-facing 快照展示。

同路径基线不存在，比较为 current-only；主人明确确认「截图准确，接受此布局」。此确认只证明布局和模拟状态，不能替代容量实测。旧 retention-task / retention-catchup-trend 图片保留为历史，排除于本轮 Task Evidence Set。

截图使用同一模拟运行的 898 invocation、1000 attempt 和 48 秒耗时，分别展示 18.71/20.83 rows/s 与 53.9/50.0 倍到达速率。工作量图表与吞吐卡片使用一致样本；这些数值只用于可重复的界面验证。

![Desktop task batch throughput](assets/retention-monthly-throughput-desktop.png)

![Mobile task batch throughput](assets/retention-monthly-throughput-mobile.png)

## References

- [长期需求](SPEC.md)
- [历史与此前容量限制](HISTORY.md)
- [ADR 0031](../../adr/0031-retention-task-local-batches-and-monthly-archive-targets.md)
