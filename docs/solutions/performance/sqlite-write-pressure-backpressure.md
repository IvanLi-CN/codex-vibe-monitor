# SQLite 写入压力与后台背压模式

## Read-side status and terminal projection

- Long-term projection must distinguish normal terminal finalization from fact correction. The former advances the terminal cursor and additive rollups; marking a natural-day repair for every status transition recreates a write-pressure feedback loop.
- Raw disk metrics are not a request-path filesystem inventory. Persist a bounded, pressure-gated inventory snapshot and expose its readiness explicitly so System Status can remain responsive without representing an incomplete scan as authoritative.

## 适用场景

- 单进程服务使用 SQLite 作为主库，同时存在前台 HTTP 写入、请求收尾写入、后台维护写入与历史回填写入。
- 线上故障表现为 `database is locked`、连接池 acquire timeout、后台任务堆积，并最终把前台请求放大成 5xx。
- 短期不迁移数据库，也不引入独立队列。

## 核心结论

- SQLite 单写者约束下，后台任务必须是可跳过、可退避、可重试的低优先级工作。
- retention/archive 不能只在整轮开始前检查一次 pressure gate，再以固定大 batch 连续写入。每次主库 mutation 都必须重新经写协调器和 gate 准入；archive 文件准备放在 permit 外，manifest 与源行 mutation 使用可恢复短事务。
- 对持续写流量，maintenance 可以使用固定间隔的单次 fairness token 防止永久饥饿，但 token 不得绕过 busy/locked cooldown，且单事务必须受行数、估算字节和实测耗时反馈共同约束。超过预算时缩小后续 batch 并将健康状态降级，不能以更长锁窗口追赶 backlog。
- fairness 只能绕过低优先级等待，不能越过已排队的 P1 terminal；若 pressure gate 在事务开始前拒绝准入，必须归还 token。归档写入使用由源行集合导出的稳定 artifact identity，并在 manifest/source mutation 前同步 artifact 与目录，避免并发覆盖或崩溃后删除唯一源数据。
- maintenance 任务名不构成绕过仲裁的理由：archive expiry、manifest 重建、raw owner reference 替换、backfill wake 与系统 raw 指标重置都必须有候选上限和独立短事务。多批指标重置期间要暴露 preparing，避免页面把不完整盘点当成健康基线。
- 前台关键路径不应该和 rollup/backfill/retention/maintenance 使用同等重试预算。
- 连接池等待超时本身就是 pressure signal，应触发后台 cooldown，而不是继续并发重试。
- `proxy capture follow-up` 也必须遵守这个分级：没有 SSE 订阅者时，不得再消耗 summary/quota 或 hourly rollup refresh 预算。
- 即使存在 active SSE 订阅者，terminal follow-up 也不应强制 `flush_now` 式 SQLite barrier；terminal overlay 是 UI 的立即收敛来源，summary/quota 可以在 write controller 后续 flush 后最终一致。
- 对请求收尾里的同一实体写入，要优先消除“重复唯一键探测 + 紧跟二次更新”“先重算 rollup 再补 timing 再重算一次”这类单请求内自我放大；SQLite 压力常常不是来自单条大 SQL，而是来自几条语义重复的写语句连发。
- 对 owner-facing 聚合读路径，同样要先消除“整窗扫明细再丢弃大部分结果”和“同参请求因 cache key 掺入 runtime 抖动而无法合并”这两类读放大；它们会和写侧单写者争用同一 SQLite 预算，并以 `database is locked` 的形式回灌到前台。
- 对 summary open-range 这类总览接口，`usage_breakdown` 与 `non_success_tokens` 不能再为了 account/model 维度或 archive overlap 去扫整窗 raw preview rows、也不能先枚举整段 live invocation id 再和 archive 去重；这类明细必须优先落到 live aggregate + archive aggregate merge，否则 7d overview 会在看似“只读总览”时重新制造数据库压力。
- Summary topic 还要区分当前态与累计态：live/in-progress 字段应直接由内存 overlay 更新，terminal 只安排固定 deadline 的 totals refresh；不要让每条 terminal event 都重新构建 summary，尤其不要为无 owner subscriber 的 topic 执行 refresh。没有 owner subscriber 时应标记 dirty，重新订阅后再生成 fresh snapshot。
- `stats.summary.current` 的 open-range terminal refresh 如果失败，应保留 last-good payload，并以有界退避重试；初始 snapshot 失败仍应保持初始化失败语义。这样可以避免锁压力下发布字段缺失 payload，同时让后续健康事件恢复累计 totals。
- 当 owner-facing breakdown 需要 `model + reasoning` 粒度时，账号 totals/hourly 级 rollup 不够用；必须补独立的 breakdown rollup（例如 `upstream_account_usage_breakdown_hourly`）并走 `full-hour rollup + exact boundary tail + archive-hole fallback`，否则“只差一个 breakdown”就会把 7d/previous7d 重新打回整段 raw aggregate。
- 对新增 rollup target 的 rollout，还要显式处理“老 archive batch 已 materialized、但新 target 从未 replay”这一迁移态。不要把新 target 直接塞进旧的 materialized shortcut；正确做法是只保留已验证安全的 legacy target shortcut，并把缺新 rollup rows 的历史 batch reopen 到 backfill backlog，否则既会漏数，又会让 telemetry 误报为 healthy。
- 对已裁剪 payload 的 legacy archive，要逐个 target 判断是否真的需要完整 payload。`usage_breakdown` 这类可由结构化列和保留的最小 payload 回放的 target 应允许 materialize，并把不可恢复的 `reasoning_effort` 归入空/unknown；`prompt_cache_*`、`sticky_key` 这类 keyed target 仍应保持 blocked，避免为了清慢读而制造错误维度。
- 当某个 closed-range owner-facing 窗口本来就只需要 exact 结果时，不要为了“统一实时”把它硬塞进 pure SSE。`previous7d` 这类 comparison summary 若继续长期订阅 `stats.summary.current`，会把 archive fallback 重新钉在高频推送链路上，读压再怎么优化都会被不必要的订阅频率放大。
- 如果 read-side 仍需要 legacy archive 的新 rollup target，自愈调度也要把“可修复 backlog”和“永久 blocked target”拆开。像 `upstream_account_usage_breakdown_hourly` 这种缺 replay marker 但可结构化回放的 backlog，应在 startup/backfill 中单独优先 drain；不要继续和 `prompt_cache_*` / `sticky_key` 共用一个 `legacy_archive_pending` 信号，否则闭区间读会长期误判成“还有 backlog”，并反复打开 fallback。
- 对账号卡这类需要 totals、non-success/failure 拆分和 latest latency 的长窗口聚合，已有粗粒度账号 hourly 表仍可能不够。可在原表增加独立版本字段与 coverage target，并按“covered full hours rollup + uncovered contiguous hours exact fallback + boundary exact tail”迁移；新 target 的 live/archive cursor 必须独立，且只有 cursor 追平后才能写 coverage marker，避免默认零值被误当成历史真值。
- 当并发不能降低且业务成功率高于观测记录完整性时，用进程内短窗口 write controller 承接所有观测记录写：terminal invocation 进入 P1 best-effort 队列，attempt 中间进度、rollup/live progress、account touch、system task finish 等可延迟项进入 P2 并按 key coalesce。记录入队/flush 失败必须报警和计数，但不得让已经完成的业务响应失败。
- 高频 runtime snapshot 不应默认等同于主事实写。`running` / first-byte / response-ready 这类 UI 新鲜度事件可以先走进程内共享 runtime store + SSE/HTTP overlay；如果服务选择业务优先于记录，terminal success/failure 也可以先进入 P1 write controller，并用 SSE terminal payload + runtime tombstone 支撑短暂最终一致窗口。
- 路由公平性字段如果不是路由正确性的硬状态，可以拆成“进程内立即生效 + batch 落库”。例如 `last_selected_at` 可先写内存锚点并叠加候选排序，账号 status/cooldown/failure 则继续同步写。
- raw payload 完整保留属于观测合同，不应作为 SQLite 止血手段被截断或丢弃；只能补齐 raw IO / Zstd / metadata 写入证据，并通过调度、窄写或配置化压缩策略减压。
- 用写侧 watermark 表达 retention 后的不可恢复 detail 边界。读侧周期任务不得为发现这个边界反向扫描 retained invocation 表。

## 推荐模式

### 1. 写入分级

- 前台关键写入：OAuth callback、请求路由状态、用户可见设置保存；优先拿连接，失败需返回明确业务错误。
- 请求收尾写入：invocation 记录、usage、raw metadata；若产品决策是业务优先于记录，应进入 P1 write controller 队列，业务响应不等待 SQLite。入队失败、flush 失败和 dropped 记录必须结构化记录。
- 请求收尾若已经存在对应 `running/pending` 行，优先原地更新而不是先 `INSERT OR IGNORE` 再走 repair/update；这样可以少一次唯一键冲突写尝试与后续锁竞争。
- terminal invocation 如果仍被定义为审计/计费强主事实，安全做法是同步“已存在 running row 时窄 `UPDATE`，缺行时 `INSERT OR IGNORE`，冲突后重读并按同一状态守卫更新”。如果当前服务明确选择业务优先于记录，则 terminal invocation 可以降级为 P1 best-effort 队列写，但必须保留完整 terminal record、raw metadata、失败分类和结构化失败证据。
- `running` snapshot 如果只是为 UI/SSE 提供进度，应避免每次同步写主表。可在请求 admit 后立即广播 `id=0` 的最小内存 shell record，并让 body parse、attempt start、first-byte、response-ready 等后续快照覆盖补全同一 `invokeId + occurredAt` runtime key；HTTP current-window reconcile 在 DB 结果上 overlay 同一份内存 store。terminal record 入队后 tombstone/remove 内存行，DB terminal 行稍后通过 write controller 最终一致补齐。这样 DB 不需要为每个 first-byte/response-ready 刷新写 `status='running'`，UI 也不会把 body read 或上游路由等待误判成“请求尚未开始”。
- 对同一 attempt 的 phase、latency、capability/compact-support 等进度字段，优先并入同一条前台更新，而不是拆成 `phase bump -> latency patch -> finalize` 的多段慢写；减少单请求尾部把 SQLite 单写者预算切碎。
- 对同一 attempt 的中间 phase、latency、capability/compact-support 等进度字段，如果不需要立刻作为业务决策真相源，可进入 250ms 级短窗口缓冲并按 `attempt_id` 只保留最新值；terminal finalize 必须同步一次写全并通过 `status=pending AND finished_at IS NULL` 防止未 flush 进度覆盖终态。
- Invocation 派生写可以按 `invocation_id` coalesce：hourly rollup/live progress 与 upstream account last activity touch 批量执行。terminal 记录 flush 产生的派生写不应强行复用同一个 SQLite 锁窗口；更稳妥的做法是把派生写放回 pending，在后续 P2 flush 中收敛。
- `system_task_runs` 的 begin 仍同步记录 running 审计入口；finish 可以进入 batch writer，pressure 下延迟或合并，但 shutdown 需要 drain 或记录未 flush 证据。
- 后台维护写入：rollup、retention、account maintenance；pressure 下 fail-soft skip。
- 历史回填写入：startup backfill、archive materialization；pressure 下延后，不阻塞 readiness。

### 2. DB pressure gate

- gate 只包低优先级后台任务。
- 任一后台任务遇到 SQLite busy/locked 或 pool acquire timeout，记录 pressure event 并进入 cooldown。
- cooldown 内后台任务返回 success-like skip，由原有 ticker / coalesced follow-up 继续收敛。
- batch writer 的最大等待 flush 不应在 pressure gate 关闭时强抢前台写锁；可记录 stale/max-age 证据并延后派生写，但 shutdown/barrier 这类完整性路径仍可旁路 gate。
- shutdown drain 也要按写入等级处理。P0/P1 terminal 主事实、路由正确性与审计事实可以尽力 drain；P2 running runtime snapshot 不应在停机时绕过 pressure gate 强制逐条写回 SQLite，否则优雅停机会反向制造写锁尖峰。
- scheduler preflight 不应占用稀缺后台槽位：enabled/due/progress 这类轻量判定应先完成，只有确定任务 due 且要执行重后台工作时才进入 gate。
- 对恢复语义敏感的维护任务可以只针对 `BackgroundBusy` 做短预算等待，避免和同 tick 的其他后台任务形成稳定饥饿；`PressureCooldown` 仍应立即 fail-soft skip。

### 3. 查询热点先补索引

- latest sample 类查询使用 `(owner_id, captured_at DESC, id DESC)`。
- session cleanup 类查询使用 `(status, expires_at)`。
- event timeline 类查询同时考虑 account scoped 与 global time scoped 两种索引。
- 维护候选查询要把固定过滤条件前置到复合索引。
- 启动 backfill / orphan recovery 这类后台扫描必须优先使用 progress cursor 或 partial index；不要把“只在后台跑”当成允许全表扫的理由。

## 常见坑

- 只加 SQLite `busy_timeout` 会把问题变成 30 秒连接等待，并不减少锁竞争。
- 后台任务拿到连接后再判断是否要运行，已经太晚；pressure gate 必须在 acquire DB connection 前。
- 后台任务拿到唯一 background slot 后再判断是否 due，会把“未到期的空跑 tick”变成对其他维护任务的饥饿源。
- skip 必须有日志和后续 ticker，否则会变成静默丢任务。
- write controller 必须有有界队列、flush 触发（时间窗口 / row count / 最大等待）、coalesced row count、oldest age、flush elapsed、queue depth、enqueue failed 与 dropped count 证据；否则只是把 SQLite 锁问题藏到内存里。
- 面向累计读模型的 terminal queue 不能长期保留完整业务记录。应在 enqueue 时投影为紧凑 delta，同时设置字节数与条数双硬限；触限时保留 last-good totals 并进入 dirty reconcile，不能静默截断后继续宣称 healthy。
- terminal 持久化 ACK 必须携带单调 row cursor，并在所有 warm selection 与 in-flight baseline cursor 越过后才回收 delta。只按时间 TTL 清理会在 flush lag 或多 selection 并发时造成漏计。
- buffered progress 不能立刻广播“已持久化”的 DB snapshot；要么广播内存态，要么等后续 reconcile/terminal 更新。否则会把 stale DB state 伪装成实时状态。
- 如果选择内存态广播，就必须让所有相关读方共享同一个 runtime store，包括 SSE、records open-resync、current summary、current timeseries、账号活动 in-flight 统计与 prompt-cache working conversations；否则去掉 DB running 写后会产生多套不一致实时视图。
- Dashboard / account activity 这类短 TTL 聚合快照，如果允许 `<=2s` 的服务端合并刷新预算，就不应再把 live runtime 状态或最新持久化行 ID 放进 cache key；否则表面上有 singleflight，实测仍会长期 `wait_on_in_flight=0`，既保不住实时性，也保不住 SQLite。
- 如果 `stats.summary.current` 与 `/api/stats/summary` 共享同一 owner-facing contract，就必须共享同一内部 summary builder 与相同的 aggregate/fallback 语义；不要让 topic 侧通过 route wrapper 间接复用旧慢链，否则线上会出现“HTTP 已修、SSE 仍慢读”的假收敛。
- `INSERT OR IGNORE` 会静默吞掉 `NOT NULL` 约束错误；用于占位写时必须绑定所有 NOT NULL 默认列，或者检查 `rows_affected` 并记录结构化证据，否则会误以为 batch flush 成功。
- 为每个后台入口单独做局部退避，容易遗漏同一压力窗口内的其他维护任务；进程级 gate 更容易统一行为。
- `SELECT MAX(id) ... WHERE <稀疏条件>`、`NOT EXISTS` + 低选择性 phase 过滤这类查询，即使最终只返回 1 行，也可能在 SQLite 上吃掉秒级读锁预算；若它们会与前台 HTTP 共享同一数据库，必须先压成 cursor 读取或用 partial index 固定扫描面。
- 对 proxy 收尾这类 SSE follow-up，`receiver_count()==0` 应该直接意味着“跳过 follow-up”，而不是继续排队 summary/quota 或 rollup refresh；否则会把没有任何订阅者的请求变成纯数据库放大器。
- 对 proxy 收尾这类 SSE follow-up，active subscriber 也不等于可以强制同步 flush SQLite。若 terminal record 已进入 P1 write controller，follow-up 应避免把 UI 实时性需求重新变成写锁 barrier；先广播 terminal overlay，再让后续 reconcile/summary 在有界延迟内补齐。
- proxy snapshot/broadcast 在 `database is locked` 下应 fail-soft skip 并记录结构化证据，依赖已发出的 SSE 事件和后续 HTTP reconcile 补齐 UI；不要在请求尾部立即重试并放大锁争用。
- write-side live read model 只有在维护成本本身也受控时才值得做：前台请求内同步维护最小必要 working-set / in-progress truth，后台 rebuild 和补偿刷新则继续挂到统一 pressure gate/cooldown，避免为了止住读热点又新增一组不受控维护写入。
- 对 Dashboard 这类连续 terminal 流量下的累计 KPI，不能把固定 SSE publish cadence 等同于 DB reconcile cadence。terminal enqueue 后应以稳定 event key 幂等更新内存 baseline，5 秒窗口仅合并 fanout；SQLite 只承担 warm restore、最长间隔的 reconcile 与异常 fallback。reconcile lock/error 时保留 last-good snapshot，并记录 baseline age、delta/duplicate count、reconcile outcome 与 sequence-gap 证据。
- baseline cursor、聚合查询与 pending-key 判定应共享同一 SQLite read transaction；构建完成后重放 cursor 之后的 compact delta 即可接受 baseline。因并发写入而丢弃完整构建并立即重试，会在稳定流量下形成 build-and-discard 风暴。
- 不要让 P1 terminal flush 在同一锁窗口内继续执行 P2 rollup/account-touch 派生写；这会把“记录最终一致”重新变成“请求尾锁放大”。P1 成功后把 P2 放回队列，等待下一轮时间窗口或 pressure 允许。
- 对业务优先的 terminal admission，可在数据库同目录维护带校验的 append-only journal：先 journal append，再异步入 SQLite；按固定短窗口 group commit，SQLite ACK 后删除完整确认的 segment。必须把 journal pending records/bytes、ACK age、replay count 与 overflow durability mode 作为结构化 telemetry。journal overflow 选择内存可用性时，不得宣称 crash-safe。
- Dashboard read model 遇到 writer pressure 时不得为了 60 秒 reconcile 再竞争一次 SQLite barrier。已有 last-good baseline 的 selection 应以 expiry delta 继续服务，并将 reconcile deferred 明确记录；必须设置最长 defer 上限，超过上限再做一次补偿尝试。
- 不可恢复的历史 payload backlog 必须标为 source-unavailable，并退出 actionable backlog。仅在 archive/payload 恢复事件唤醒，外加每日受行数和耗时限制的 probe；否则退避 ticker 会把永久缺失输入伪装成持续工作。
- event-driven backfill 不能只在表中保存 `next_run_after`，supervisor 也必须把 recovered deadline 镜像为内存等待条件。否则即使任务本身已退避，固定 supervisor ticker 仍会周期性查询每个 progress row、运行无关 maintenance 并写审计记录。repair wake 必须携带受影响 task 集合，pressure defer 只更新该 task 的 retry deadline，空闲 pass 不创建 `system_task_runs`。

## 何时升级方案

- 如果前台关键写入本身持续超过单写者能力，应用层背压只能缓解，不能替代数据库迁移。
- 如果需要跨进程 worker 或严格 FIFO，需要引入外部队列或 PostgreSQL，而不是继续扩大 SQLite 连接池。

## Terminal Projection

- P1 journal ACK 后可向多个 read-side consumer 发布紧凑 terminal event 与 durable row cursor；consumer cursor 必须独立，不能让 P2 rollup 成功阻塞 raw terminal durability。
- P1 的高频 admission ticker 不能同时作为 P2 pressure retry ticker。P2 应由首事件固定 deadline、pressure cooldown 截止时间和 background-slot eligibility 事件唤醒，并把 gate defer 与真实 lock failure 分开统计；否则“没有执行 SQL”的 defer 也会形成 CPU 空转和误导 retry 指标。
- SSE topic 的通用 mutation 广播不能自动等价为 full-window cache invalidation。对 Prompt Cache 这类可按 key 增量维护的读模型，应以 active-topic baseline + compact delta + bounded reconcile 代替每条 terminal 后的整窗 hydrate。
- 用 cursor 与持久 interval segment 替代定时范围重算：正常窗口只 hydrate 新增 rowId 并 additive upsert 受影响 rollup key，明确 repair 才精确重建目标桶。压力期保持 last-good 并 defer，不得为了补账重新抢 P1 锁。
- 对已持久化 terminal 的字段修正要在同一事务中写入 target bucket repair marker；只更新全局 materialization 状态会让 cursor 之后没有新 row 的投影永远看不到修正。
- 目标桶 repair 的 archive source 不只包含 terminal archive，也包含补齐账号归属所需的 attempt archive。archive rewrite 要同时排入旧、新 coverage；单个文件不可读时只延后对应桶并保留 last-good，不能让最老的失败 marker 饿死整个 repair 队列。repair 查询必须按目标自然日绑定范围，避免每个桶都读出整份 archive 后再过滤。
- Projection schema、trigger 和兼容迁移属于启动期工作；固定 cadence 的 flush 或验证路径不得重复执行 DDL，否则后台维护本身会重新抢占 SQLite 写锁。
- 增量 projection 替换全窗重算时，必须把旧 builder 的 hourly retention 一并迁入 P2 维护路径，并同步清理对应 hourly interval 状态；否则 service 虽然不再慢扫原表，却会在长期运行中把 projection 表无限累积。
- `wall_time_ms` 由 interval union 派生，不能像 token/cost 一样对每批 delta 直接相加。持久化 interval segment 是重启恢复 union 的必要条件；upsert 应以完整 union 覆盖旧快照，才能保持重叠调用的精确值。
- A terminal cursor ordered by row ID must not silently skip an older request that becomes terminal after a newer row. Mark that affected bucket for exact repair when the cursor has already crossed the row; normal in-order terminal finalization remains incremental.
- Open-window timeseries must not persist full invocation JSON merely to preserve P95. Store minute aggregates plus exact latency sample blobs, use a terminal-delta overlay until the fixed P2 flush succeeds, and allow raw reads only for explicit leading/trailing minute tails or coverage warmup. A projection consumer must acknowledge individual persisted events, not advance by an unrelated maximum row ID, because SQLite row IDs can be sparse with respect to terminal writes.
- A durable raw overflow spool needs a defined capacity boundary. At that boundary, choose and log a durability mode explicitly: retain in a bounded writer-backed memory queue for availability, or reject capture before it is accepted. Never relabel a capacity failure as an ordinary asynchronous backpressure drop.

## Memory Attribution Before Remediation

- SQLite 文件大小、`COUNT(*)` 行数和 `liveInvocationsCount` 都是磁盘/数据库事实，不能直接当作 Rust 进程内存对象数。性能调查应同时采集 RSS、匿名内存、文件映射、Swap、`VmHWM` 与 cgroup memory。
- 已知组件只做无克隆估算：runtime store、terminal/projection pending、Dashboard snapshot、long-term interval index、timeseries staging、raw writer occupancy、prompt/network/routing cache 与 SQLite writer queue。timeseries 若复用 terminal pending bytes，必须避免重复计入 `managed_bytes`。
- 每个高风险后台操作应记录 `retained_bytes`、`retained_delta_bytes`、`peak_delta_bytes`、`load_row_count`、`clone_avoided` 和 elapsed。`peak_delta_bytes` 使用 `VmHWM` 增量，不能用结束时 RSS 增量冒充。
- `unattributed_anon_bytes` 是正式分类，不是默认故障结论。只有匿名内存未归因比例连续达到阈值时，才允许显式启用一次受限 allocator 诊断；诊断默认关闭且不改变业务并发、数据保留或回收行为。

## High-Frequency Runtime Data Plane

- Dashboard live rendering and terminal-derived consumers must share a bounded in-memory projection, while SQLite remains the recovery, reconcile, and closed-range source. A memory-first label is not sufficient: the healthy live renderer must have no `Pool<Sqlite>` dependency.
- Terminal admission, projection updates, and persistence acknowledgements are separate ownership boundaries. P1 raw durability must not wait for P2 rollups, account touches, or maintenance writes; each consumer needs its own cursor and dirty-last-good state.
- A fixed publish deadline must not invalidate a database snapshot on every terminal event. Publish current-state changes from memory, reconcile baselines on a longer cadence, and defer reconciliation during writer pressure with explicit last-good age and defer telemetry.
- memory-first 只能排除数据库成本，不能自动排除 CPU 放大。projection snapshot、topic overlay 与 delivery frame 必须形成单向 typed 边界；完整业务对象广播、cached JSON 深拷贝和多 topic 重复序列化仍会在零 SQL 情况下制造高 CPU 与 subscription lag。
- 账号窗口、长期统计和 open-window timeseries 也必须纳入一个 typed `StoragePlane`，而不是保留为“Dashboard 之外”的 direct-pool 例外。该边界负责同参 singleflight、读写优先级、coverage/last-good 与内存诊断，CI 应拒绝高频模块新增裸 pool/SQL 入口。
- event-driven projection 的空 dirty 集必须是零写入：不得删除 interval、写 task run 或执行全表 warming。真正的归属、价格、archive rewrite/restore 修正只标记受影响 bucket；pressure 下保留 cursor/last-good 并等待统一 gate。

## Proxy hot-write coordination

- SQLite 的单 writer 约束要求代理 terminal、attempt、route/account 状态和派生统计共享同一个 admission 面；只给后台任务加 pressure gate 不能阻止前台 helper 相互争锁。
- P1 batch 必须同时限定条数、估算字节和 admission 时间。锁冲突要保留完整未提交批次并指数退避；固定短 ticker 会把一次外部锁放大为稳定 CPU/日志风暴。
- 同步 attempt/route 可以保留原返回语义，但应在事务前按优先级等待。P2 只有在 P1 与同步 waiter 为空时运行，并且 cursor replay 必须按 chunk 让出 writer。
- 健康诊断必须展示 active write class、waiter、retry generation、下一次 retry、batch rows/bytes 与 direct bypass；否则“排队成功”仍可能掩盖未迁移的直写入口。
