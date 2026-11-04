# Analytics module 的 PostgreSQL 迁移设计

架构归属与已确认取舍见 [ADR 0054](../adr/0054-analytics-module-postgres-ownership.md)。本文展开迁移协议、历史完整性、模式回切、运维和验收要求，作为尚未实现的设计提案。

PG 接管后停止 SQLite 派生写入、回切接受重建窗口，以及有证明的历史汇总种子可迁移、缺少证明阻断切流，均已获 owner 确认。owner 进一步确认完整 consumer registry（含 dashboard、账户聚合与 prompt-cache 统计）、7 天有界回切保证窗口，以及仅使用认识新 owner/fence 的程序回切并排除任意旧 binary downgrade。容量、重建时长与统计不可用预算仍须 C10 代表性实测确认，不能据此放行生产切流。

## 事实基线与当前缺口

代码核验基线为 `3085f061`（`v4.19.6`）。这里的结论来自源码，不代表对线上实例重新验收。

| 已有能力或缺口                                                                               | 源码证据                                                                                                                                           | 对本设计的约束                                                      |
| -------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| SQLite 写路径在同一事务中发布 invocation、attempt 和 archive coverage 事件                   | `src/analytics_changes.rs:622,807,1090,1160`；`src/sqlite_batch_writer/flush.rs:1062`                                                              | 保留本地事务原子性；逐个写入口核对，不能遗漏修复与维护写入          |
| PG startup migration、归档/当前快照 bootstrap、Journal catch-up 和源端 ack 已存在            | `src/pg_analytics/mod.rs:360,573,2631,2890,2999,3110`                                                                                              | 可复用协议中的幂等、CAS、版本与角色分离，不能用连接成功代替数据验收 |
| timeseries、parallel-work、账户实际用量已有 PG 读取                                          | `src/api/slices/prompt_cache_and_timeseries/timeseries/queries.rs:413`；`parallel_work.rs:306`；`src/upstream_accounts/sync_group_sessions.rs:145` | 当前是混合读架构；需要完整的消费者清单和统一切流                    |
| `AnalyticsReadStore` 只有 overview/series 接口；long-term 路由继续读 SQLite                  | `src/pg_analytics/mod.rs:3698`；`src/long_term_stats/intervals.rs:955,1011`                                                                        | 接口存在不等于模块迁移完成                                          |
| PG consumer 对非 invocation 的 upsert 跳过；coverage 只记事件                                | `src/pg_analytics/mod.rs:2551`                                                                                                                     | attempt、维度修正、归档替换必须实现明确投影语义                     |
| bootstrap 生命周期在首次成功后退出；`resume_replica` 是有界 drain                            | `src/pg_analytics/mod.rs:452,482,3052,3110`                                                                                                        | 必须增加持续消费与监督，不能依赖一次启动 catch-up                   |
| readiness 来自 runtime atomic 状态；请求侧另做 freshness 检查                                | `src/proxy/request_entry.rs:32`；`src/pg_analytics/read.rs:8`                                                                                      | health 必须反映当前 owner、发布进度与新鲜度，不能只反映启动成功     |
| 日汇总永久保存，小时汇总至少 366 天；归档可以在已验证后退役                                  | `docs/specs/5k89c-long-term-usage-analytics/SPEC.md`                                                                                               | 现存 live/archive 全量重放仍可能缺少已退役历史；不能直接清空再建    |
| 应用 Docker 镜像持久化 `/srv/app/data`，health 只探测 core；仓库没有应用级 PG/SQLite Compose | `Dockerfile:118`；`compose.backend-test.yml` 仅供测试                                                                                              | 两模式部署、secret、readiness 与 volume 合同需要正式交付            |

现有 schema 注释和 runbook 中仍有早期 provisioning/read-path 描述；后续实现应按源码核对更新，不能据其文字推导出模块已接管。当前 `service_eligible` 兼容字段在 semantic version 2 中刻意保持 false；新 owner/readiness 合同须使用独立的版本化状态，保留旧读者 fence。

## 领域与模块边界

术语见根目录 [GLOSSARY.md](../../GLOSSARY.md)。`analytics 写入归属` 指派生事实、投影和持久读模型的写入归属；业务请求提交仍发生在 SQLite。

| 状态                                                                                     | PG 接管后的权威与唯一写入入口                                              |
| ---------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| 调用、请求尝试、账户/路由/设置、原始载荷、TerminalJournal、业务任务                      | SQLite 及原有业务写入器；PG 故障不改变其提交条件                           |
| 源 Journal、generation、coverage revision、源 ack、backend control                       | SQLite；业务事实更改与对应事件同一事务提交                                 |
| 归档文件、manifest、删除恢复状态                                                         | 原有源存储；作为事实恢复来源，必须满足新的 analytics coverage 门禁         |
| 规范化 invocation/attempt facts、最小维度快照、parallel-work digest、canonical intervals | PG analytics materializer；禁止 API handler 独立写派生表                   |
| 短窗/summary/hourly/daily/long-term 读模型、覆盖证明、repair queue、publication state    | PG analytics projection worker；全部归属同一模块及 owner epoch             |
| 请求中的运行时尾部、内存缓存、SSE 差量                                                   | 同一 analytics 查询服务组合；有界、按身份去重，持久基线来自所选 backend    |
| 冻结的 SQLite 历史读模型                                                                 | 迁移/rollback 输入；PG active 时没有后台更新，不得作为成功统计的隐式备用源 |

`AnalyticsModule` 提供 source envelope、materialization、projection、query、health/control 五个内部边界。`SqliteAnalyticsStore` 与 `PostgresAnalyticsStore` 实现同一查询合同。业务层只提交事实和源事件；查询层不能任意访问另一 backend 的派生表。

### 查询消费者清单

不能按 `/api/stats` 前缀判断归属，该路由组中也存在业务操作。实现前建立可机读的 consumer registry，并由测试覆盖每个入口的 backend 选择、依赖及不可用语义。

| 消费者                                                                                              | 接管范围                                                                                     |
| --------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `/api/stats`、summary、timeseries、perf、errors、failures、parallel-work、upstream-account-activity | 全部持久基线、tail 组合与缓存生成；包括相应 HTTP、SSE/topic builder                          |
| long-term overview/series                                                                           | 日/小时读模型、canonical interval、完整性状态和精确 tail                                     |
| dashboard activity/recent、network recent/timeseries、forward-proxy stats/timeseries                | 持久历史来自所选 backend；纯进程网络采样和 in-flight 展示保留其运行时来源                    |
| invocation summary、账户列表/详情中的统计字段、公共统计展示                                         | 共享的 analytics 聚合调用必须走同一 backend；业务明细列表、详情和账户操作仍走事实层          |
| prompt-cache conversations 的统计投影                                                               | 数量、时间与使用量派生部分纳入 registry；binding/affinity 操作、原始 key、路由状态仍属业务层 |
| quota/latest、binding 的 POST/PATCH、账户选择与请求路由                                             | 业务合同；不得因路径相似迁移写入归属                                                         |

上线完成条件是 registry 中所有 analytics 消费者均已实现；内部开发可以分批适配，对外不提供按 endpoint 各自切库的长期配置。原始 prompt key、载荷、IP 和错误正文不得进入 PG；需要展示原始业务详情时，必须明确是独立的事实查询，不能借它补齐缺失的 analytics 聚合。

## 写入、重放与发布合同

### SQLite 的提交承诺

每项影响统计的业务更改，都必须与一个可重放的源 envelope 在同一 SQLite 事务中提交。范围包括 finalize、running/terminal 修正、account/model/time/cost 修正、attempt 更新、维度身份变化、逻辑删除、archive publication/replacement 和已允许的 source retirement。Journal append 失败使该事实事务失败；PG 不可达不参与这个事务。

envelope 包含 `source_instance_id`、不可变事件身份、generation、change id、coverage revision、事件/事实/统计语义版本、稳定实体身份、操作语义、规范化 payload 及 digest。parallel-work digest、归属与时间等投影必需字段在源事务中定格；consumer 不能回查“最新 SQLite 行”拼成一个从未存在过的事件快照。现有旧事件通过有版本的 decoder 或重新 bootstrap 过渡；不能原地重写已发布事件的解释。

明确区分三种删除：业务逻辑撤销需要删除相应统计贡献；live→archive 搬迁只改变来源位置；retention 退役不撤销已发生调用的长期贡献。现有 `tombstone` 会删除 PG fact，不能直接拿它表达后两种动作。维护代码中的直接 SQL 和 trigger 也纳入写入口审计，不能只检查公共 helper。

### PG 的唯一派生写入器

在同一 source/backend owner 下，materializer 持有 PG writer advisory lock，并以单调递增的 `owner_epoch` fence 每次提交。session lock 必须由实际写连接持有；连接丢失即停写，不允许从 pool 换连接后继续假定仍持锁。DDL、materializer、reader 使用分离角色；新增 rollup/repair 权限逐表审查，不能将现有 reader 升成 owner。

首轮实现采用一个派生写 actor：事实、projection publication 与 repair 的持久写事务均经同一持锁连接排队，历史任务在批次间让出；读取/计算可以使用独立只读连接。若以后引入并行派生写连接，必须另行证明 fencing 与提交互斥，不能绕过 session lock。

每批采用以下提交顺序：

1. 从一个 SQLite 一致快照读取有界的源前缀和 generation/retention fence；校验完整顺序、版本、payload digest。先检查每个事件 kind 有明确处理器。
2. 在一个 PG 事务内锁定 owner/checkpoint，登记事件身份；已存在且 digest 相同的事件跳过贡献更新，不同则阻断。对新事件应用规范化事实、旧值撤销/新值贡献、canonical interval 和持久 dirty/repair 工作。事实、去重记录、待完成义务与 ingest checkpoint `C` 同时提交。
3. PG 事务确认提交后，才在独立 SQLite 事务中 ack 该前缀，得到源 ack `A`。ack CAS 失败保留 pending 状态，重读源状态后重试；不重算已经提交的贡献。
4. projection worker 在有界批次构建 read model。昂贵的区间并集、日期修复使用 staging 与 last-good publication；完整目标发布时才原子更新 published checkpoint/coverage `P`、publication token 并删除对应 repair obligation。
5. 每个对外响应只使用同一个 publication、owner epoch 与 PG repeatable-read read-only 事务。未完成的 repair 不得与已更新事实拼成部分成功结果。

`C` 表示派生事实及后续工作已持久接收，`A` 表示源端获知这项承诺，`P` 表示查询结果已完整发布。必须满足 `A ≤ C`、`P ≤ C`；三者不能合并成一个 `caught_up` 布尔值。源 ack 不是统计 ready 的证明。复杂投影允许 `P < C`，需要的事件内容已在 PG facts/repair obligation 中持久保存；不能 ack 一个仅写入临时内存队列的事件。

ingest 的最新事实只供内部派生使用。对外 fact/tail/interval 读取必须按 publication 的 `P` 选择可见版本；首轮使用带事件生效区间的版本记录和 publication token，包括修正与删除的旧值。新版本及 read model 构建完成后，才在一个事务中推进 publication pointer。仅使用 PostgreSQL MVCC repeatable-read 不足以把已经提交到 `C` 的最新事实倒回 `P`；现有直接读取 latest `analytics_facts` 的路径需要适配。旧版本只有在 reader/build pin 释放且恢复覆盖允许时才能清理。

重放使用 at-least-once 输入与幂等应用，不宣称跨库 exactly-once。去重键绑定 source instance 和事件身份，冲突必须校验相同 payload digest；相同 key 不同内容阻断。派生事实按稳定来源实体唯一，保留最后事件顺序，拒绝旧批次覆盖新值。汇总修正从旧/新事实差异或受影响桶重建得出，不能对重复事件无条件 `+=`。checkpoint 必须验证整个源前缀，不能仅取最大 change id。

bootstrap、在线 drain、repair 与 rebuild 共享 owner fence；一个 PG target 一次只接受一个 source instance。不同 volume 的数据库副本不构成 HA；复制/恢复 SQLite 到新部署必须显式分配 source incarnation、重新验证 target binding，禁止两个副本并发写入同一 target。

模块 consumer 使用独立的版本化身份，不能继承现有 `pg-analytics` checkpoint/coverage 就视为完整，因为旧 consumer 的处理范围不同。source registry 记录 active、building、retired consumer 及其 retention pin；退役旧 consumer 需要显式状态转换，不通过任意删除 coverage 行解除保护。旧 target 可留存用于比较，新 target 必须重新建立完整模块 baseline。

新模块对 archive publication/replacement 执行按 manifest identity 的增量导入和受影响桶修复。大文件先按页写入带 SHA-256 的 staging，最后一个事务才认证该事件的完整输入、推进 `C` 并登记 publication 工作；不能只存文件路径便 ack 后允许源清理。routine archive 到达不触发全 target 清空；retirement marker 只更新来源覆盖，不能删除已发布历史贡献。

### 持续消费与新鲜度

启动后持续运行有界 consumer，源提交通知唤醒并用低频 catch-up 防止通知丢失；idle 不启动无意义的写事务。按现有 256 facts/1 MiB 批次起步，历史工作单独预算、可暂停、可取消。正常重启先识别可续跑 target；禁止每次重启都清空并全量回放。

health 报告 source/owner/epoch、`C/A/P`、最近源观察、lag、projection version、repair backlog、coverage 摘要和有界错误码。steady-state 不要求任何时刻都与正在变化的源尾部相等，要求发布进度满足 endpoint 覆盖、新鲜度与尾部合同；建议沿用现有 15 秒 freshness 上限，最终值待容量验收。PG 故障或 stale 时，analytics 返回明确 unavailable，core 可继续 ready。

generation 失效、retention gap、source/owner 切换先关闭同进程 read admission，再持久化失效状态；新的请求不得仅凭 PG 中尚未刷新的旧 heartbeat 入场。已入场请求可完成其固定快照。重启在读取源控制状态并核对 PG 前保持 admission 关闭；外部维护也必须取得同一 runtime ownership lock。

## 历史迁移与完整性证明

### 两类输入与覆盖分区

输入一是可完整读取的 live invocation/attempt、已验证 archive 及其修正 Journal；输入二是原始来源已按既有政策退役的有证明历史汇总种子。种子保留原有日/小时粒度、维度、指标、样本数、statistics start、source retirement boundary、版本和 provenance，不能被宣称为完整 normalized facts。

为每个 `时间范围 × 指标 × 维度 × 粒度` 建立 coverage ledger，取值为 `exact_facts`、`certified_seed`、`unavailable`。边界按完整桶确定；墙时跨午夜/跨小时的 canonical interval 与 carry-over 一并处理。不能以一个全局最小日期证明所有指标完整。

同一统计单元只能选择一份基线。`certified_seed` 桶不得再叠加其中的 archive/live 全量数据；种子之后的修正只有在能确定旧贡献或完整重建该桶时才可发布。其余修正持久化为 blocked repair，保留 last-good 并撤销受影响范围的可用性。不同粒度/来源相加前必须证明没有重叠。

若日汇总只有 overall calls/token/cost 的证明，不能由此认证 model/upstream、墙时、性能样本或错误分布。每项原有能力都需要对应证明；有 row/count/hash 仅证明拷贝一致，不能替代原统计正确性证明。已经按策略消失的明细不属于虚构的可重建能力；unexpected missing archive、unknown legacy history 和来源缺列仍阻断切流。现有 `--acknowledge-unverified-history` 不得作为本模块迁移完整门禁的绕过方式。

### Snapshot 与 archive identity

每个 build 分配 `build_id`，在 SQLite snapshot 记录 source instance、generation `G`、高水位 `H`、coverage revision `R`、retained floor `F`、数据语义版本、种子与 archive manifest digest。SQLite 的一致快照不自动固定文件系统中的归档：读取前后核验 SHA-256/size/manifest identity，并在来源处建立有租期的 retention/replacement pin。pin 到期或身份变化后，该 build 不能继续认证旧文件。

live 与 archive 重叠按稳定来源实体消重，定义 snapshot 时最新有效来源优先级；检测同身份不同内容，不能靠 `ON CONFLICT` 静默覆盖。需要 attempt archive 才能确定归属/时长时，必须把它纳入同一 manifest。扫描与 PG staging 都使用有界页；SQLite snapshot 或 WAL 增长超过预算即暂停重建。中断后只有仍有效的 snapshot/recovery artifact 可续跑；不能换一个源快照接着旧 cursor 假装同一 build。

Journal retention pin 从 snapshot watermark 开始保护未消费后缀；容量不够时明确中断该 build，继续事实写入并标记 target 不可用，禁止静默删掉后缀仍宣称完整。源 generation 改变或后缀已过 retained floor 时，废弃本次证明，保留失败 build，再从有完整来源的新 build 重建。generation ack 不能用来跳过未证明的缺口。

### 验收证书

每次切流产生 source-bound certificate，至少包含：

| 证明         | 必须验证的内容                                                                                                                                          |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 来源清单     | 每份 live dataset、invocation/attempt archive、历史种子都有身份、版本、完整性/退役证明和覆盖范围；没有悬空 delete_pending 或未知缺失                    |
| 事实等价     | 在同一 `G/H/build` 下，对稳定排序、规范化编码的实体与内容生成分桶 SHA-256、行数、null/sample 计数；逐桶比较源/PG；冲突/漏行/多行任一非零即失败          |
| Journal 前缀 | 从 baseline 后到目标 fence 的所有预期事件都有身份/digest；generation、retained floor、CAS、`C/A` 对齐；只比较 checkpoint 最大值不合格                   |
| 历史汇总     | seed 各维度/指标/粒度的行数、内容、样本和 coverage 与已证明 SQLite 输入一致；导入区间与事实区间互斥                                                     |
| 投影发布     | 所有消费者所需 projection version/coverage 已发布至切流 fence，`P` 覆盖该 fence，dirty/repair 无未解决缺口；区间并集、时区边界和多粒度交叉对账通过      |
| 查询等价     | 固定 now、时区、过滤、尾部和同一逻辑 fence，比较 HTTP 与 SSE 的状态/shape/null/order/数值；整数和 recorded decimal 精确一致，浮点只用逐指标规定的窄容差 |
| 运维恢复     | 断点重入、PG/source ack 故障、PG 重启、模式往返和 restore 演练通过；backup 身份与恢复来源覆盖被记录                                                     |

种子 digest 证明迁移保真，来源证书证明历史覆盖，二者都需要。全局总数一致不能替代各维度对账；shadow parity 不能通过同时漏掉一段历史获得认证。

SQLite 旧投影的 durable cursor 未必等于 Analytics Journal watermark。bridge 必须建立可验证的 publication→源 fence 映射，或以同一冻结 source snapshot 重建独立 oracle 再比较；不能拿正在变化的旧 API 响应与 PG 的另一个时点直接对账。baseline 证书从 `H0` 延伸至切流 `H` 时，需要源事件前缀 digest、受影响桶和 publication 的增量证明，并重新检查 manifest/pin；不只替换证书上的最大 checkpoint 数字。

## 切换状态机与 fence

源端新增版本化 `analytics_backend_control`，保存 source instance、单调 epoch、desired backend、active backend、phase、build/certificate 引用与切流 fence。PG 保存对应 owner/epoch/build/publication。它们组成可恢复的顺序协议，不是跨库原子事务；任一中间态均禁止新 analytics 请求入场。

保留已有 SQLite runtime/inode ownership lock，约束 service 与维护命令；PG writer lock 保护 target。控制器串行执行转换，数据库 CAS 拒绝旧 epoch 的 batch 和 publication。service 的业务事实写入可持续；analytics worker 的启动、关闭、drain 和请求 admission 由同一控制器管理。

| 阶段                | SQLite 派生写/读                                   | PG 派生写/读                            | 退出门禁                                              |
| ------------------- | -------------------------------------------------- | --------------------------------------- | ----------------------------------------------------- |
| `sqlite_active`     | 当前模块写/读                                      | 无 active owner；可保留旧 target        | source 与 SQLite projection ready                     |
| `pg_building`       | 继续正式写/读                                      | 新 build 影子 materialization；不对外读 | 来源、容量、schema、历史 seed/archive 验证及 catch-up |
| `pg_prepared`       | 继续正式写/读                                      | 影子持续追尾；内部 parity               | certificate、全部消费者、rollback 预检通过            |
| `switch_pending`    | 先停止新派生任务并 drain；关闭 analytics admission | 取得新 epoch，追至切流 fence；读关闭    | 源控制、PG owner、publication 与证书一致              |
| `pg_active`         | 派生写关闭；对外 analytics 不读 SQLite 投影        | PG 唯一正式派生写/读                    | 持续 coverage/freshness 检查                          |
| `rollback_building` | 对选定新 epoch 重建；读关闭                        | 原 epoch 被隔离；不服务 analytics       | SQLite 全部读模型达到 rollback fence 并通过验收       |
| `sqlite_prepared`   | 新 publication 已完成；等源端 active CAS           | 保留 target 诊断；旧写 epoch 无效       | 源控制发布 SQLite active，再开放 analytics            |

允许并行的“双写”仅是迁移期两个独立 materializer 从同一事实/Journal 派生，SQLite 仍为正式 backend；业务 handler 不执行 SQLite+PG 独立同步提交。owner 已选择不在 `pg_active` 保留 SQLite shadow 派生写。

### 正向顺序

1. 上线认识两种 backend 和 control schema 的 bridge release，保持 `sqlite_active`；完成源事件覆盖审计和 schema compatibility 门禁。部署 PG、容量与恢复来源就绪后，控制器进入 `pg_building`。
2. 执行独立 schema migration，再执行 archive/live/seed build、Journal drain 和 read model publication；每项都有自己的 ledger、进度和暂停点。现有正在使用的 target 不被全局 truncate；采用 build-scoped staging 或新 namespace。
3. 在原 SQLite 读面服务期间运行固定 fence shadow parity，达到 `pg_prepared`。迁移模式为临时控制阶段，不增加永久的 `read_backend`/`write_backend` 独立配置。
4. 关闭 analytics admission，停止并 drain 所有 SQLite analytics 派生 worker/维护任务，排空已发起的派生事务，捕获新的切流 fence `(source_instance,G,H,R)`。源事务可继续追加 `H` 之后的 Journal。
5. 源控制 CAS 进入 `switch_pending` 并推进 epoch；PG 在 writer lock 下认领该 epoch，处理至 `H`、ack 源端并发布所有必需 read model。再次验证 certificate 和覆盖到 `H`，以及当前 lag 仍在预算内。
6. 源端 CAS 发布 `pg_active`，PG/read gate 确认相同 epoch/build 后，统一将 analytics 查询服务、SSE 与缓存切至 PG 并开放 admission。缓存键带 backend/epoch/publication；旧订阅重建基线，不能继续拼接 SQLite delta。
7. 持续消费 `H` 之后的 Journal，health 开始报告 active PG 状态。只有全部 registry 消费者与实际查询 probe 通过才算模块切换完成。

“写切换先于读切换”通过停止 SQLite 派生 worker、认领 PG owner、统一发布读门禁落实。跨库转换中崩溃时可以出现两个已持久化阶段不一致，但该状态不可服务；重启读取 control、PG owner 与 checkpoint 后幂等补齐或进入有理由的 blocked 状态。禁止只依据 env 设置直接开放某一半 endpoint。

## Rollback、故障恢复与来源保留

### 模式回切

PG→SQLite 采用同一套认识 control/epoch 的程序，以 `desired_backend=sqlite` 触发恢复；不得直接启动 `v4.19.6` 等不理解新 owner/fence 的程序承诺安全 rollback。程序版本回退和存储 backend 回切是两项独立合同。

1. 关闭 analytics admission，停止 PG 派生调度，drain 当前事务；停止/排空原 service 并由 runtime lock 保证接班过程唯一。PG 可达时废止旧 writer epoch；PG 不可达时由原进程/连接退出和新进程完全不创建 PG writer 保证隔离，源控制记录新 epoch。旧实例禁止自动同时重启。
2. 取得源端 rollback fence，验证冻结历史种子、保留的 live/archive、Journal 后缀和 retirement proof 足以重建每个原有消费者。来源不全时保留业务服务和统计 unavailable，显示准确缺口。
3. 在新的 SQLite staging/publication 重建至 rollback fence，处理持续到来的事件；完成与正向相同的历史覆盖、事实、指标和 publication 检查。
4. 源控制 CAS 切为 `sqlite_active`，开放统一 SQLite analytics admission；PG 数据、ledger、失败 build 和 bounded error 留存。以后重回 PG 使用新 epoch，先验证现有 build 可续跑，否则新建 build。

重建不把 PG 派生数据覆盖回 SQLite 业务事实，不自动执行 down migration 或 backup restore。owner 已接受统计在重建期间不可用；只要 SQLite core 健康，业务事实写入继续。故障造成的已落盘状态由新程序前向修复。

### 回切保证的保留边界

冻结的 SQLite rollup 加有限 Journal 并不天然支持任意时刻回切。例如 PG 运行一个月后，某日 raw/archive 已过期而 SQLite rollup 仍停在切流前，该日不能凭空重建。

每次源数据退休前，必须证明仍存在从已认证历史种子到当前 fence 的完整恢复覆盖，包括需要的 invocation、attempt、canonical interval 与修正。PG/source ack 只证明事件已接收，不能授权删除唯一 rollback/rebuild 来源。保留 pin 必须有容量预算和到期处理，不能无限挂起磁盘清理。

建议先声明一个从成功发布 `pg_active` 开始的有界 rollback 保证窗口（建议 7 天，待 owner 确认），窗口内禁止删除唯一重建来源；重启不重置期限，续期必须重新验证恢复覆盖与容量。窗口外仍可选择 SQLite 模式，但只有来源验收通过才开放统计，不能承诺无条件回切。若 owner 要求长期任意切换，需要另行选择长期保留 source recovery archives，或设计可独立恢复的 analytics recovery package；后者是独立设计决定，不暗含在本提案里。

### 故障矩阵

| 故障点                             | 可恢复状态与处理                                                                        |
| ---------------------------------- | --------------------------------------------------------------------------------------- |
| 源事实事务未提交                   | 事实与 Journal 均不成立；正常业务重试                                                   |
| 源提交后 PG 离线                   | 事实/Journal 已成立；consumer 退避；统计超过 freshness 后 unavailable                   |
| PG batch 提交前退出                | 整批回滚，源 ack 不推进；相同事件重放                                                   |
| PG commit 结果不明                 | 重连先查询事件身份/digest/checkpoint 决定续跑，禁止盲目重复累加                         |
| PG 提交后源 ack 失败               | 目标保留 `C`，重试同一源前缀 ack；PG publication 仍受独立 gate 控制                     |
| ack 完成但 publication 前退出      | durable dirty/repair 继续；读取 last-good 或 unavailable；不能把 `A` 当 `P`             |
| generation/retention/semantic 变化 | 关闭 admission，拒绝旧 epoch/build；只有完整来源重建后恢复                              |
| archive 替换或 seed proof 失效     | 使受影响 coverage/build 失效；不可将 coverage marker 当作已处理历史                     |
| 切流任一步骤退出                   | `switch_pending` 不服务；同一控制器根据持久阶段幂等 reconcile                           |
| PG active 后 PG 故障               | 保留业务服务；显式选择 SQLite 才进入 rollback，不做请求级 fallback                      |
| PG target 丢失                     | 从完整 source/seed 重建，或独立批准的 PG backup restore；ack 不能充当备份               |
| SQLite/source Journal 丢失         | 停止 source 写和 analytics admission，进入业务数据灾难恢复；PG 不自动升格为业务事实权威 |

## Docker 运维合同

目标交付是一个应用镜像、两个明确 backend 选择，同一个 SQLite data volume；PG 模式增加 PG18 volume/容器和 secret 输入。保持 `ANALYTICS_BACKEND=sqlite|postgres`、默认 sqlite，避免永久 `read_backend`/`write_backend` 组合。迁移进度保存在 control state，env 表达 operator 的 desired backend。

在线预热是可选优化：operator 仍选择 SQLite 时，显式 prepare 才允许 `pg_building/pg_prepared` 继续提供已激活的 SQLite 统计。一旦应用以 postgres 选择启动，在 PG 接管门禁通过前 analytics admission 关闭，不能因 bootstrap 失败自动回到 SQLite。未预热也能仅靠启动 PG 模式自动完成 schema、build、追尾和切换，代价是首次构建期间统计不可用；core 继续运行。state 表中的 SQLite 正式读仅指可选预热阶段。该合同保留显式 PG 选择的 fail-closed 语义。

仓库需新增正式应用 Compose 与运行文档；下列是待实现的合同，不是当前可执行命令：

- SQLite 启动只依赖应用和 SQLite/source volume，不要求 PG DSN 或容器存在。
- PG 启动先确认目标 PG 数据库/角色/连接，再由有序 migration、bootstrap、publication 控制器推进；DDL secret 生命周期只覆盖迁移任务。
- 初次大迁移可用同镜像的显式 prepare 控制操作进入临时 `pg_building`，继续服务 SQLite；正式切换只选择 postgres 并重建应用容器。prepare 不是启动前置操作；首次直接启动 PG 模式由控制器自动构建。无需手工 SQL、搬文件、逐 endpoint 切换或定时运行 consumer。
- 常规重启识别既有 ledger/owner/build，幂等续跑；不清空已完成 target。SQLite 模式回切自动进入已描述的 rebuild gate，PG 容器可暂时保留。
- 只有一个 app service 挂载同一个业务 data volume；更换 backend 不换 Compose project/volume 名，不同时起两个 app。Compose recreation 必须先 drain 原 app。
- 容器 health 保持 core 语义；部署验收额外探测 analytics health 和一个实际查询。`up -d`、PG TCP ready 或单独 `/health=200` 均不足以宣告 analytics 切流完成。
- DSN 用 secret 引用，日志和诊断脱敏；固定 PG major 并声明 backup/restore 操作。应用不隐式创建/删除 PG 数据库，也不自动执行跨 major upgrade。

容量预检覆盖 PG facts/index/WAL、staging、原 target、归档解压、SQLite WAL、source pin、备份和 rollback 保留。至少应容纳估算峰值再加明确安全余量，并在每阶段观察剩余量。已提供的现场信息是磁盘约 98%、仅余约 7.4 GiB；本任务没有重查现场，这个历史读数既不构成当前容量结论，也不能通过容量门禁。扩容或经单独授权的精确清理先于生产迁移；不得通过删除 SQLite 业务事实腾空间。

## Cleanup 条件

cleanup 是独立、可暂停的操作，不随模式切换自动发生。停止 SQLite 派生写入不等于删除 SQLite 派生表。

只有在完整模块验收、rollback 保证窗口结束、source/recovery coverage 已核实、backup restore 演练通过、无活动 build/repair/reader pin、支持版本兼容性允许之后，才可在单独批准的变更中逐批删除过期 staging、旧 PG build 或冻结 SQLite 派生状态。清理精确按 build/table/range 标识，保留必要 ledger/provenance；不全库 truncate，不删除业务事实、TerminalJournal 或源 Journal。

清理旧源 archive 必须同时通过现有长期统计来源门禁与新的恢复覆盖门禁。删除失败留下 pending 状态，不能提前移除 manifest/proof。删除 SQLite 表不会保证文件立刻缩小；压缩/重写文件涉及额外停机、空间和恢复合同，需要独立授权。删表也不能提前移除 sqlite backend 的 schema/适配器，否则破坏两模式合同。

## 验收要求与兼容性

以下是候选实现必须满足的验收要求，尚未执行。工作包、执行顺序及命令编排在设计边界确认后另行制定。

最小验收清单：

- [ ] 写入口与 consumer registry 审计覆盖完整；旧程序/新源协议的桥接兼容经过验证。
- [ ] PG18 schema 与角色在隔离目标上通过重复执行、超时、中断和前向修复验证；reader 无派生写权限。
- [ ] 在每个业务写入口注入事务 rollback，证明没有事实已提交但事件丢失的状态。
- [ ] 同一源事件处理两次、PG commit 结果不明、源 ack 重试，事实/汇总/interval 仍只贡献一次。
- [ ] PG 慢或离线期间源写继续；retention 逼近边界可观测，发生缺口后始终 unavailable，不能通过 ack 清掉缺口。
- [ ] invocation/attempt 更改、account 软删除与归属、时间跨上海午夜、recorded cost 修正、逻辑删除与 retention 搬迁均对账。
- [ ] archive identity 重写、文件消失、source snapshot 失效、旧 journal semantics、unknown event、stopped release state 均可重入或明确阻断。
- [ ] 已退役但有证明的历史 seed 保真；seed/facts 重叠被拒绝；不能重建的性能或维度不被 overall proof 误认证。
- [ ] PG 已 ingest/ack、投影只发布一半时不能读到部分成功；last-good 与新 publication 原子切换。
- [ ] `C > P` 时，fact/tail/interval 查询仍读取 `P` 的版本；更新、删除和 reader pin 下旧版本清理均通过验证。
- [ ] 旧 `pg-analytics` 已 ack 的 attempt 事件不会使新 consumer 跳过 baseline；旧 SQLite cursor 与源 fence 无法对应时不伪造 parity。
- [ ] 每个 consumer 的 HTTP/SSE 与精确 runtime tail 在统一 epoch 下等价；切流前后缓存/订阅不混库。
- [ ] `sqlite_active → pg_active → sqlite_active → pg_active` 完整演练，逐点 crash 与 PG 不可达 rollback 都通过。
- [ ] Docker 纯 SQLite 无需 PG；PG 模式无手工 schema 操作；重启不会无条件重放全部历史；core 与 analytics health 分别验证。
- [ ] 每个声明支持的旧 source state 和前向 repair 路径测试；明确不认识新 fence 的旧程序如何被排除。
- [ ] 容量、保留窗口与实际重建时间经过代表性数据验收；本地文档检查不替代这些证据。

测试归属沿用仓库 resource contract：数据库逻辑放对应轻量/stateful 桶，archive、gzip、真实文件与锁测试放 archive-file-io；PG/Docker、集成和重型验证在 session-owned Agent VM 执行。实施时运行最小匹配测试及 Rust fmt/check/clippy，完整回归使用 `.github/scripts/run-backend-tests.sh`。当前仅新增文档，不运行 backend 套件。

候选实现计划按 public API compatible、persistent state minor 评估；state 支持范围至少明确包含 `v4.19.5` 和 `v4.19.6` 的现存 PG18/SQLite 状态，其他 earlier-Minor 支持范围由实现前的 release policy 锁定并逐个验证。不能标为 patch 而又要求同一 Minor 的受支持旧程序拒绝新 state。本文档交付本身不决定 release number/label。

迁移记录见 [persistent-state-migration-record.json](assets/analytics-module-postgres-ownership/persistent-state-migration-record.json)，候选兼容性记录见 [version-impact-record.json](assets/analytics-module-postgres-ownership/version-impact-record.json)。二者均为 proposed，验证尚未执行。

## 待确认的设计边界

已确定的根节点是 SQLite 业务事实/源 Journal 权威、PG analytics 派生写/读归属、无请求级隐式 fallback。第一轮已确认的分支是：PG active 停止 SQLite 派生写，rollback 接受重建；有证明历史 seed 可迁移，证明缺失阻断。

以下建议尚未确认，不作为已接受的要求：

1. **模块范围（已确认）**：采用完整 consumer registry，包括 dashboard、invocation/account 聚合与 prompt-cache 的统计部分；业务 binding/路由/明细保持事实合同。
2. **回切保证期限（已确认）**：采用 7 天有界保证窗口；窗口外只有来源验收通过才开放 SQLite 统计。若要求长期任意切换，须先决定 recovery archives/package 及容量，才能设计 source cleanup。
3. **运维预算（部分待实测）**：C10 以代表性数据确认 analytics 不可用上限、rebuild 总时长与 source pin 容量；freshness 沿用 15 秒合同草案，最终证据仍由验收确定。未取得容量/重建证据前阻断生产部署和切流。
4. **兼容性范围（已确认）**：使用认识新 owner/fence 的候选程序切换 backend，排除任意旧 binary downgrade；实现前补齐 earlier-Minor source fixtures 和 stopped-release forward repair 测试名单。

这些问题只决定后续实现边界，不阻碍审阅本提案；未回答的项不视为同意。接受后应先同步长期 topic Spec，再独立授权实现及上线。本文没有授权生产修改、删数据、PR publication、merge 或清理。

## 取舍

独立同步双库提交会把 PG 故障带入事实提交，且缺少跨库原子性；采用事实事务加 Journal。永久 SQLite shadow 虽可缩短 rollback，却保留持续争用和双投影成本，owner 已选择接受重建窗口。仅复制 invocation facts 无法承接 attempt、长期汇总和历史退役能力；采用完整模块 registry 与带证明 seed。微服务化/消息中间件和自动双主提升不属于本单实例 Docker 运行合同。

## 参考

- [ADR 0053：PG18 managed migration](../adr/0053-pg18-analytics-managed-migration.md)
- [PG18 analytics Spec](../specs/pg18-analytics/SPEC.md)
- [长期统计 Spec](../specs/5k89c-long-term-usage-analytics/SPEC.md)
- [PG18 runbook](../runbooks/pg18-analytics.md)
- [源 Journal](../../src/analytics_changes.rs)、[PG runtime/consumer](../../src/pg_analytics/mod.rs)、[PG read fence](../../src/pg_analytics/read.rs)
- [统计路由注册](../../src/maintenance/hourly_rollups.rs)、[long-term 读取](../../src/long_term_stats/intervals.rs)、[long-term source proof](../../src/long_term_stats/schema.rs)
