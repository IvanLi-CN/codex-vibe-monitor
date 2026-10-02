# 有预算、可恢复的 Retention：实现映射与具体方案

## Current Status

- Implementation: REQ-BRR-001..015 的基础实现已随 v2.82.0 发布；REQ-BRR-016..021 已形成当前候选实现，仍需 shared-testbox 容量实测、迁移回归和最终视觉/审查证据。
- Lifecycle: active。
- 当前候选从 `origin/main@b68295595d875aead33ef31514eaf602279ece94` 建立于 `th/retention-catchup-observability`；交付 commit 在 Step 5C Ready 时补入本记录。
- 原交付的 shared-testbox 三个资源 profile、旧状态前向修复与 Demo 视觉确认已完成。旧百万行试验验证 Prompt 统计分页收敛，不是 retention 端到端吞吐或 24 小时追赶达标证据。

## 调查证据与限制

只读调查对象为 2026-10-01 的生产 v2.80.2，以及与该版本相关源码一致的本地 `a4270dac42d6996497ececdd69bbc07ba50ec45c`。

| 观察                   | 证据                                                                                                                               | 含义                                                             |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| 最新手动运行           | run 88730，03:11:10.555Z 开始，记录耗时 1040.643 秒；invocation 归档 4 行、上游尝试归档 6 行、触及 archive 批次 8、移除 raw 文件 6 | 有实际成果，页面摘要未展示全部计数；约 17 分钟不能由产出数量解释 |
| 同步统计刷新           | 两次会话聚合 SQL 分别约 192.196 秒、97.167 秒；对应 archive 微批次事务约 193.4 秒、97.5 秒                                         | 聚合直接延长归档写事务                                           |
| 孤儿候选搜索           | 会话孤儿 SQL 约 714.057 秒，返回 0 行                                                                                              | LIMIT 400 未限制寻找符合条件记录的扫描成本                       |
| 主要耗时               | 上述三条 SQL 合计约 1003.42 秒，为本轮耗时约 96.4%                                                                                 | 优先处理这两条会话派生链路有直接证据                             |
| 页面总量、进度、检查点 | 详情接口 `progress: null`；retention 没有发布进度记录                                                                              | 显示空值并非积压为零                                             |
| 页面性能显示 0         | 原始 6h/24h maintenance 指标可查到当轮 1 次成功及耗时，任务摘要却是 0                                                              | 24h/1h 查询遗漏当前小时桶，另有空桶覆盖最新耗时问题              |
| 默认计划显示空值       | registry 没有 interval 值；实际 legacy worker 使用默认 3600 秒及恢复资格                                                           | 页面未展示有效默认计划，不能据此判断任务未调度                   |
| 启动失败显示 0ms       | legacy 路径在执行失败之后才创建运行计时                                                                                            | 旧记录不能恢复原始耗时，只能显示未知证据                         |

调查时观测到约 122.8 万条过期 invocation 积压，归档恢复状态 degraded，且有 SQLite busy/locked 和维护写准入等待。未取得生产 SQLite `EXPLAIN QUERY PLAN`，因此不把具体全表扫描或缺索引结论当作已验证事实。为何本轮只归档 4 条的完整停止原因尚无足够阶段日志；方案补充阶段结果，不能靠推测填补。

## 已收敛的需求

范围是 retention 及其共享会话慢链路和观测；保持精确引用和 archive 证明。采用每轮 60 秒工作预算、每个 Prompt SQL 页 2 秒预算、可恢复检查点；归档总量以运行开始时的过期 invocation 积压为口径。统计刷新走既有队列，孤儿清理有独立游标；运行报告 completed/partial/deferred，并保留 fatal failed。统计未收敛时沿用精确读契约，显示暂不可用、阶段和积压。

## 积压治理与小时观测

### 线上事实与诊断边界

2026-10-02 的只读 API 调查显示，下列连续运行确实有归档提交，但没有形成可接受的存量消化速度。时间使用 UTC，数量为各轮起点观测，不是同一原子快照中的净变化证明。

| 运行起点            |   耗时 | 本轮 invocation 成果 | 起点待归档量 | 停止观测                              |
| ------------------- | -----: | -------------------: | -----------: | ------------------------------------- |
| 2026-10-01 23:30:50 | 12.9 s |                    4 |    1,257,051 | partial，预算未耗尽，wait reason 为空 |
| 2026-10-02 00:30:50 | 61.7 s |                    4 |    1,257,047 | partial，工作预算耗尽                 |
| 2026-10-02 01:30:50 |  4.4 s |                    3 |    1,257,043 | partial，预算未耗尽，wait reason 为空 |
| 2026-10-02 02:30:50 |  2.0 s |                    0 |    1,257,040 | deferred，`sqlite_pressure`           |

这些观察总计下降 11 行，约为 125.7 万存量的 0.0009%；不能把“部分成功”解释为积压问题已解决。后续核实的线上 v2.82.1 未改变本表对应的 retention、runtime、维护库和压力准入代码。统计刷新队列同时有 420 个待办且物化所有者禁用；这是会话派生状态，不能并入 invocation 积压，也不授权自动启用物化所有者。

源码核实的限制与待证明原因：

- `src/maintenance/retention.rs` 的 invocation 初始批次为 4 行，最大 64 行；自适应预算可调整，但 `archive_old_invocations` 在循环前取得 candidate limit，后续提交的调整未在本次循环逐批重新读取。该函数允许多个批次，因此初始 4 行不能独自解释每轮只提交 3–4 行。
- `src/db_pressure.rs` 的全局后台槽只有一个；后台槽占用、优先等待者与真实 SQLite busy/locked/cooldown 是不同的准入原因。retention 的部分延期路径将不同拒绝统一记录成 `sqlite_pressure`，现有记录不能证明过去每次延期都是 SQLite 锁冲突。
- `src/proxy_sqlite_write_coordinator.rs` 的维护写公平机制不能自动保证另一个后台槽的取得；不能只增加单轮时长或批次上限就宣称消除了后台饥饿。
- `src/runtime.rs` 的 managed 调度按计划 occurrence 再次触发；partial/deferred 本身没有保证快速接续。`src/maintenance_store.rs` 的覆盖计划会使 legacy 默认入口跳过，健康字段的 `nextRetryAt` 不等于调度器已经安排了该时刻的运行。
- 原逐阶段成果不足以复原 4.4 s/12.9 s 运行的具体停止边界。需要阶段、后台槽、协调器、SQL、archive 文件准备/证明/提交和实际候选批次的分解证据；后台 inventory 是可能的竞争者，不能当作已证实的历史占用者。

本轮没有执行生产全库 COUNT、EXPLAIN、修复、重启或控制变更；线上停止根因仍需实现后的定向观测与可复现测试进一步确认。

### 已确认的产品契约

| 决策          | 契约                                                                                                  | 需求映射                  |
| ------------- | ----------------------------------------------------------------------------------------------------- | ------------------------- |
| Q1 最长滞留   | 展示最长归档逾期时长；例如已存在 12 天而保留 5 天，产品口径为超期时长，实际计算遵循现有自然日资格边界 | REQ-BRR-020               |
| Q2 待处理范围 | 主图只计待归档 invocation 行；上游尝试、Prompt 会话、孤儿和文件分别按自身单位展示                     | REQ-BRR-021               |
| Q3 消化目标   | 普通在线负载下，24 小时内完成起点固定存量，持续处理能力超过新增过期速率，保留在线优先和安全边界       | REQ-BRR-017               |
| Q4 自动接续   | 有积压自动追赶，轮间让行和压力退避；自定义 interval/cron 控制巡检，禁用阻止后续追赶，清空后回巡检     | REQ-BRR-016/018，ADR 0027 |
| Q5 小时点     | 每个 UTC 小时最后一次成功的准确快照；两项指标同次观测，保留实际时间，独立采样，失败及升级前缺测留空   | REQ-BRR-019/020/021       |

1,257,040 行固定存量在 24 小时内归零，至少需要全天平均 14.55 行/秒的有效提交能力，新增过期量另算。这是目标的量级，不是已经测得的容量。如果每天只执行 24 个 60 秒工作轮次，活跃时间内需要约 873 行/秒，仅观察 4 行成功不能支持这个承诺。

### 执行与吞吐方案

复用现有 retention owner 和 managed 调度边界，把“下一次巡检”与“当前积压的继续资格”分开。每轮发布真实终态后，启用且还有可推进积压则在让行或退避后请求下一轮；该请求与巡检/手动请求在同一单实例入口合并。禁用代次在每次新工作准入前检查，重启从业务检查点重新确认待办。不得增加并行归档 worker、补跑 occurrence 或绕过管理员控制。

归档微批次在每次候选读取前取得最新自适应限制，结合实际提交、文件准备和写锁时间控制工作量。先通过分段计时确定归档准备、已有批次证明、raw 引用检查、全局后台槽或写协调器哪一项限制吞吐，再改动对应瓶颈；提高上限须有短锁、文件边界和在线延迟实证，不能将初始 4 行直接改成无界大事务。阶段轮转保留可恢复检查点，统计物化仍由原所有者消费。

对后台槽采用已有有界排队/公平能力，等待发生在全局 mutex 外；真实压力保持退避，前台和交互写保留优先。诊断保留低基数的阶段和原因，至少区分 `background_slot`、`maintenance_write`、SQL/事务与归档阶段，以及 `background_busy`、`coordinator_wait`、busy/locked/cooldown、预算、禁用/关闭、异常。计划显示提供 next inspection 和 next eligible catch-up；资格时间不保证实际执行时间。归档核心的剩余量决定追赶，Prompt 暂不可用或其所有者禁用不能使已无 invocation 积压的核心无意义空转。

### 小时观测与图表方案

独立后台采样器测量 live invocation 的过期归档范围，不依赖 retention run 是否发生；任务禁用时仍采样，服务离线时留缺口。通过已有时间索引在同一读快照取得准确 COUNT 与最老候选，记录 `observedAt`、cutoff、保留策略及指标。采样使用有界 SQL 执行和真实取消/连接清理，复用已有预算能力；预算拒绝或失败只报告本次采样状态，不把陈旧 health 值改名为新快照。实际采样频率与预算需由百万行成本验证，至少提供逐小时成功观测的正常负载能力，不承诺整点零延迟。

最长逾期不能复用当前 `oldestBacklogAgeSecs`，该字段是记录年龄；其旧 API 含义保持不变，新增逾期指标单独表达。当前规则为 Shanghai 当日午夜减 N 个自然日，候选比较为 `occurred_at < cutoff`；发生在本地日期 D 的记录首次符合规则的时刻是 D 加 N+1 个自然日的午夜。应复用相同时间解析和资格判断，在观测时间计算最早资格时刻的逾期；不能简单做 `now - oldestOccurredAt - N * 86400`。历史桶记录当时策略，策略更改时在图表提示，避免把策略收缩误读为吞吐退化。

小时历史属于维护库，而不是可清空的性能遥测缓存。每个 UTC 小时幂等保留 `observedAt` 最晚的成功观测，两项指标一起发布；失败不清除较早成功。后台发布失败可重试同一观测，不能改变采样时间或覆盖更新的观测；维护库丢失不退回 HTTP 中同步扫描主库。历史保留覆盖滚动 7 天所有相交小时桶，清理采用有界批次，结构安装不回填历史。

任务页增加一张“最近 7 天归档积压”趋势卡，建议使用现有 Recharts 绘制上下两个共享时间轴的图：上图为待归档条数，下图为最长逾期，避免两种单位共用同一纵轴。UTC 分桶、Shanghai 标签；当前未结束小时可显示最近成功样本，tooltip 显示实际采样时间和策略。缺测断线不补值；准确的 0 行显示为 0，最长逾期为空并说明“无待归档记录”；过期观测与未知独立标识。其他阶段列出各自 backlog、单位和原因，尚无精确测量的阶段显示未知。升级后只有已采集时段有曲线，不生成虚构的完整 7 天趋势。

API 以可选字段或有界历史接口提供已持久化观测、覆盖范围与采样状态，保持现有字段和 status。任务页读取维护库快照，不启动 COUNT、统计物化或归档。具体字段结构、采样 cadence、轮间让行/backoff 常量与公平实现由实现计划和成本证据确定，本节不锁定尚未验证的性能参数。

### 新需求的验证缺口

REQ-BRR-016..021 已有代码候选和定向单元覆盖，但尚无 shared-testbox 的真实 retention 容量卡、完整迁移实证或当前候选的视觉证据。原百万行 benchmark 只运行 Prompt 统计刷新，不执行真实归档链路；其中在线 p95/p99 还高于基线，不能用于通过 VER-BRR-009。

新实测必须运行真实 retention 源行转换、archive 文件准备/证明/发布及 raw 所有权路径，固定百万行存量、倾斜 key、在线读写、文件特征与实际后台竞争，先定义可复现的普通负载；开发基线和候选分别重复三次。每次记录固定 cohort 的剩余量直到实际归零及新增过期量，不能仅用几分钟速率推算 24 小时达标。对线上负载的校准仅使用不含 payload/身份的聚合信息，不复制生产数据库。持续高压另作压力场景，不能偷偷减少代表性在线负载以通过目标。

采样验证覆盖同快照精确对照、百万行读取成本、取消后的 SQL/锁状态、停用仍采样、同桶失败保留、跨小时/重启、策略变化与自然日边界。页面验证覆盖两条趋势、缺口/未知/零/过期、升级初期、其他阶段各自单位、追赶与巡检、桌面及移动端。历史表增加的维护库结构仍须幂等迁移、保留旧未知值且 ready 不扫描历史。

## 模块边界与执行方案

### 同一入口与预算

在 `src/maintenance/retention.rs` 提供一个 retention 运行服务，由 `src/runtime.rs` 手动/自定义调度入口和默认/启动/恢复 worker 共用。它持有单实例 gate、真实运行起点、控制代次、预算、活跃会话保护及阶段结果；不能再用参数不足的 bare maintenance 调用作为手动入口。

运行预算从执行起点计算，包括准入等待。每个阶段在等待前、每次候选读取前、每个业务提交后检查剩余预算；预算用尽停止准入新批次。已准备的文件走现有 prepared archive 恢复协议，已提交的批次保留，未提交的事务回滚。文件读取和压缩按块检查取消，文件发布与业务提交保留其不可拆分的安全边界。

60 秒是准入/工作预算，真实结束时间允许当前安全收尾；结果记录 `budgetMs`、`elapsedMs` 和 `settlementMs`。2 秒查询预算必须在 SQLite 执行层生效，并确认停止、回滚和连接释放；实现前先验证当前 SQLx 的实际取消能力，不能只加 `tokio::time::timeout` 就宣告满足。

### 归档提交与统计队列

删除 invocation 的事务仅保留 archive 证明、源行转换、统计失效及 durable key enqueue；移除其同步调用 `refresh_prompt_cache_conversation_stats_on_connection` 的路径。prepared archive 恢复、详情裁剪等同类发布路径也使用同一规则，避免普通归档变快而恢复路径仍同步聚合。

既有物化 worker 继续拥有统计刷新及其启用控制。归档提交后发出唤醒提示，durable queue 是恢复依据，唤醒丢失可由已有调度探测恢复。物化被禁用时不绕过控制，归档结果记录统计待办及禁用原因。

统计刷新优先保持现有一事务的刷新/queue-clear 原子性。对于无法在 2 秒页预算内完成的单 key，引入 key 内部的 invocation keyset 分页和 durable 暂存聚合：记录 source 范围、mutation 代次、页游标及累计统计，在短提交中保存每页进度；最终仅在代次未改变时原子发布完整统计并清除相同代次的 queue 项。发生新 mutation 则失效本轮暂存、保留待办并重建，绝不展示暂存结果。持续写入时允许暂不可用；静默后必须收敛，避免固定重试同一条超时全量 SQL。

查询复用项目的规范化 Prompt key 表达式，优先使用已有 `(normalizedPromptKey, occurred_at)` 索引，按 `(occurred_at, id)` 顺序分页并保存双字段游标；`source_max_invocation_id` 只作本轮 source 范围，不作 mutation 证明。以 fixture 的查询计划和实际耗时验证定位、排序和取页均有界，不能只验证返回行数。需要补索引时不得把大表建索引隐含进快速 readiness 保证，须单独验证结构安装的写入/启动影响。调整聚合物化的批次控制须同步更新 ADR 0022 的实现描述：保留 400-key 逻辑页与自适应思想，但 key 内分页/真实超时不能被 32-key 下限阻挡。

### 独立的孤儿候选页

把“寻找至多 400 个孤儿”改为“按 key 游标读取一小页会话候选，再逐个作精确引用检查”。候选页查询不包含无界的 correlated OR 搜索；key 引用与 invoke-id 范围引用分别使用可验证的索引 probe。已有 grace、保留规则、活跃占用和 invoke sequence 安全条件须一起保留。

候选范围及已检查位置持久化；无孤儿的页也推进游标。游标完成后开始新的扫描 epoch，覆盖先前游标之前新出现或重新符合清理条件的会话。超时、锁冲突和压力只结束本阶段，归档继续；失败页不能推进到未检查的候选之后。

先在全局缓存锁外进行候选扫描；删除前用短暂的 key 占用/代次保护和事务内引用再验证保证安全。不能把活跃 key 的一次快照当作删除授权，也不能在全局缓存 mutex 内等待慢 SQL。删除后缓存 eviction 必须校验 conversation 身份/代次，不能移除同时重新创建的身份；已有 invoke sequence 下限不能被统计重建重置。具体保护复用已有引用/序列 reservation 路径并通过并发 fixture 验证。

### 独立阶段与恢复

运行服务组合 archive reconciliation、invocation archive/prune、相关 archive 清理及会话派生阶段，分别记录 committed count、pending、elapsed、wait、checkpoint、nextRetry 和 reason。耗时阶段之间使用公平、有界的份额，不能因最前面的慢阶段反复耗尽预算而饿死其他可安全推进的阶段。

归档核心完成不要求物化 worker 在同一轮完成。页面进度可继续显示最新后台状态；本轮终态是结束时的不可变审计快照。部分完成采用既有恢复机制的正常 eligibility；压力退避遵循既有策略，内部重试不得伪装成新的 cron occurrence 或累积错过的小时运行。

## 观测与接口方案

保留 `TaskRun.status` 原有值，在运行详情中增加可选的 `completion`、`coreCompletion` 和有界 `stages`。成功执行但仍有待办可为 `status=success, completion=partial`；预算耗尽不是 fatal failure。纯压力/准入拒绝且无进展为 deferred；可恢复异常即使无进展也为 partial 并记录原因。致命错误仍为 failed，已提交成果保留。

| 页面信息    | 来源与口径                                                                      |
| ----------- | ------------------------------------------------------------------------------- |
| 总量        | 运行起点的过期 invocation 观测，带 `observedAt` 和 source 范围；范围不足则未知  |
| 已完成      | 同一 source 范围内、本轮成功提交的 invocation 行；不加上 attempt 行、key 或文件 |
| 进度/ETA    | 仅在同口径覆盖有效时计算；partial 不显示总体 100%，统计积压单独展示             |
| 阶段/检查点 | 业务 checkpoint 的异步维护库镜像，含游标摘要、等待原因和下次资格                |
| 完整成果    | invocation/attempt 行、归档批次、raw 文件/字节、裁剪等现有 summary 全字段       |
| 耗时        | 本轮阶段耗时、准入等待、SQL 执行与预算收尾；不使用失败后新建的计时器            |
| 调度        | 实际生效的默认/管理员覆盖来源及下一触发；恢复重试另行标识                       |
| 观测新鲜度  | 真正的 snapshot 时间；无更新标记 stale，数据缺失标记 unknown                    |
| Prompt 统计 | 未收敛为暂不可用，同时展示刷新阶段、pending 和禁用/等待原因                     |

运行开始时固定过期 cutoff 和 `MAX(codex_invocations.id)` 上界，复用匹配范围的 backlog 观测，不为页面或每个 batch 执行全表 COUNT。`id` 是自增整数，因此后插入的回填或 backdated invocation 不纳入本轮计数范围，由下一轮处理；既有行发生 eligibility 变更时必须重新核对范围覆盖，不能仅以 MAX(id) 证明总量仍精确。如现有观测不能匹配该范围，显示观察数和未知百分比。业务进展时间与等待心跳时间分开记录，约 1 秒合并快照仅写维护库。

有效计划由运行调度器提供，优先管理员 interval/cron 覆盖，否则是默认 3600 秒。页面展示 runtime 的下一次资格，禁用后无下一次执行；不通过迁移给所有 registry 行盲写默认 interval，也不启动第二个同任务调度器。

性能摘要修正共享时间桶计算：对 `[from,to)` 使用从 `floor(from/step)` 到 `floor((to-1)/step)` 的全部相交桶；覆盖信息使用同一边界。最新耗时只取有 duration 样本的桶。历史详情即时展示运行结果，小时聚合作趋势；维护库或遥测无覆盖时不能用“0 次”遮盖缺失。

## Implementation Coverage

| 要求                            | 主要源码映射                                                                                                                                            | 当前状态 / 剩余证据                                                                                                                                      |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| REQ-BRR-001/002/004/008         | `src/maintenance/retention.rs`, `src/runtime.rs`                                                                                                        | 已实现统一调度入口、60 秒工作预算、独立收尾计时和结构化阶段结果；shared-testbox 三 profile 及历史状态恢复夹具已通过                                      |
| REQ-BRR-003/005/015             | `src/prompt_cache_conversations.rs`, `src/maintenance/retention.rs`                                                                                     | 已实现 SQLite progress handler、2 秒共享剩余预算、单 key 分页、代次安全发布和暂不可用契约；百万行候选已收敛并释放队列                                    |
| REQ-BRR-006/007                 | `src/prompt_cache_conversations.rs`, `src/maintenance/retention.rs`                                                                                     | 已实现持久化孤儿 epoch/cursor、候选精确 probe、删除前身份复核和事务 checkpoint；archive-file-io profile 已通过                                           |
| REQ-BRR-009/010/011/013         | `src/maintenance_store.rs`, `src/runtime.rs`, `src/api/slices/system_routes_and_tasks.rs`                                                               | 已实现固定 source 范围、完整 run details、阶段进度、默认/覆盖计划和 stale 观测；API 兼容及历史状态恢复回归已通过                                         |
| REQ-BRR-012                     | `src/performance_telemetry.rs`                                                                                                                          | 已修正半开相交桶、覆盖率和最新有样本桶选择；非整点窗口回归已通过                                                                                         |
| REQ-BRR-008/009/010/011/013/015 | `web/src/lib/api/core-foundation.ts`, `web/src/pages/system/SystemTaskDetailPage.tsx`, `web/src/pages/system/taskLabels.ts`, `web/src/demo/handlers.ts` | 已发布可选字段兼容、成果/阶段/单位/计划、未知/过期/统计待刷新显示；原交付的 Storybook、Web 单测、typecheck、lint、build、任务页 E2E 及视觉确认已完成     |
| REQ-BRR-014                     | `src/maintenance_store.rs`, `src/prompt_cache_conversations.rs`, `src/tests/stateful_sqlite/*`                                                          | 已实现幂等 DDL、主库续作结构、维护库 nullable 字段和代次回归；v2.71.45/v2.80.2 前向修复、重复 DDL、中断 staging/generation 保留及 ready 不回填回归已通过 |
| REQ-BRR-016/018                 | `src/runtime.rs`, `src/maintenance/retention.rs`, `src/db_pressure.rs`, `src/proxy_sqlite_write_coordinator.rs`, `src/maintenance_store.rs`             | 已实现单一 owner 的 catch-up 资格、原因退避、禁用清除及逐批自适应限制；仍需压力/恢复和公平性 shared-testbox 证据                                         |
| REQ-BRR-017                     | retention 真实发布链路与 shared-testbox 端到端实验                                                                                                      | 未验证；普通负载的固定存量 24 小时归零和在线 p95/p99 对照不能由旧 Prompt benchmark 代替                                                                  |
| REQ-BRR-019/020                 | `src/maintenance_store.rs`, `src/maintenance/retention.rs`, runtime observer                                                                            | 已实现 2 秒 SQLite 进度取消、同快照 COUNT/最早资格候选、UTC 小时幂等历史和缺测填充；仍需自然日/故障/迁移及锁释放实证                                     |
| REQ-BRR-021                     | 任务 API、`web/src/pages/system/SystemTaskDetailPage.tsx`, `web/src/lib/api/core-foundation.ts`, Demo/Storybook                                         | 已实现可选 trend API、7 天双图、观测时间/策略 tooltip、缺测断线和追赶字段；仍需桌面/移动视觉证据与交互回归                                               |

## Compatibility and Migration

当前候选仍按 minor 评估：public API 保留原字段及 status 并只增加可选字段；维护库新增小时观测表和任务追赶状态采用幂等前向迁移，旧 Minor 程序不承诺维护新状态。记录见 [version impact](assets/version-impact-record.json) 和 [migration record](assets/persistent-state-migration-record.json)；24 小时容量和最终 release 分类仍未验证。

主库只增加业务正确性需要的 cursor、source 范围、刷新代次/暂存和索引。维护库增加 nullable 结果/快照字段。结构安装幂等且不历史扫描；运行时逐页产生 DML；旧进度、历史耗时和完成度保持未知。既有 archive artifact、保留天数和 wire 格式不变。

支持状态以既有 retention 的 v2.71.x 旧 schema 和本次生产 v2.80.2 schema 为必测边界；当前仓库没有额外可复现的中间 retention schema 快照，因此不把未验证的中间结构写成已覆盖事实。候选读取旧状态，而旧 Minor 程序不承诺维护新续作语义。停止发布后的恢复采用新版本前向修复，备份恢复单独处理。`ensure_schema_repairs_v271_and_v2802_retention_fixtures_without_historical_backfill` 固定了两个声明边界的旧 invocation/队列形状，重复运行 DDL，并验证历史行、未完成队列和 staging generation 不丢失；新增结构仍由幂等 DDL 和缺列修复覆盖。

## Verification and Remaining Gaps

当前候选已完成 Rust fmt/check、维护库 24 项定向回归和 Web typecheck；完整 Clippy、三个 shared-testbox profile、Web 单测/lint/build、Storybook、任务页 E2E、迁移夹具及容量卡仍待当前 head 验证。profile 耗时不作为吞吐结论，旧百万行对照单独记录在 benchmark card；这些旧结果不覆盖新增需求。

上一轮百万行同种子对照只覆盖 Prompt 统计物化：候选 130 万行/热 key 50 万行的刷新耗时为 370.8s，开发基线为 101.7s，在线读 p95/p99 也有差异。它没有执行真实 retention 归档、Verified Archive 提交或固定 cohort 清空，因此不能作为本计划 A7 容量证据；新的 retention capacity card 仍待 shared-testbox 取得有效三次基线/候选结果。

旧 Demo 视觉已获确认。当前候选仍需证明自动追赶、公平性/真实停止原因、24 小时存量归零、在线延迟不劣于基线、独立小时采样与新图表；对应 VER-BRR-008..011。任何缺少该证据的验收项保持未验证，不降低门槛。

实施验收对应 SPEC 的 VER-BRR-001..007。代表性大表至少包含超过百万 invocation、稀疏孤儿和单 key 倾斜；同时施加在线读写，验证有界查询、主库锁释放、公平推进及静默后统计收敛。性能门槛不能用 4,000-key/40,000-invocation 小 fixture 代替，也不能从生产的历史时间倒推保证。

Rust 回归按 `lightweight`、`stateful-sqlite`、`archive-file-io` 合同分桶；真实 archive/file/锁行为留在 archive-file-io。重型及集成验证直接在 shared-testbox 运行。Web 覆盖接口可选字段、结果状态、默认调度、未知/过期和暂不可用。渲染改动已在本地 Demo 生成桌面/移动端状态证据；当前候选截图等待主人确认，确认前不作为 owner-facing 视觉证据。

## Visual Evidence

原交付 Demo 桌面状态已显示五项同口径指标、默认 `3600s` 有效计划、阶段检查点、运行完成度、Prompt 统计“暂不可用（积压 3）”和性能覆盖率；移动端状态已显示按钮、核心指标和可滚动任务内容。当前候选新增了追赶字段和 7 天双图，Storybook 与任务页 E2E 已覆盖 completed、partial、deferred、failed、未知/过期和统计待刷新；新的桌面/移动截图已生成，待主人确认后再作为 owner-facing 证据写入规范资产。

- [桌面任务详情](assets/retention-task-desktop.png)
- [移动端完成状态](assets/retention-task-mobile.png)
- [移动端部分完成状态](assets/retention-task-partial-mobile.png)

## References

- [长期需求](SPEC.md)
- [主题历史](HISTORY.md)
- [ADR 0025](../../adr/0025-retention-core-and-conversation-derived-maintenance.md)
- [ADR 0027](../../adr/0027-retention-catchup-independent-of-inspection-schedule.md)
