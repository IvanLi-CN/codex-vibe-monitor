# SQLite 写入压力与后台背压模式

## 适用场景

- 单进程服务使用 SQLite 作为主库，同时存在前台 HTTP 写入、请求收尾写入、后台维护写入与历史回填写入。
- 线上故障表现为 `database is locked`、连接池 acquire timeout、后台任务堆积，并最终把前台请求放大成 5xx。
- 短期不迁移数据库，也不引入独立队列。

## 核心结论

- SQLite 单写者约束下，后台任务必须是可跳过、可退避、可重试的低优先级工作。
- 前台关键路径不应该和 rollup/backfill/retention/maintenance 使用同等重试预算。
- 连接池等待超时本身就是 pressure signal，应触发后台 cooldown，而不是继续并发重试。
- `proxy capture follow-up` 也必须遵守这个分级：没有 SSE 订阅者时，不得再消耗 summary/quota 或 hourly rollup refresh 预算。
- 即使存在 active SSE 订阅者，terminal follow-up 也不应强制 `flush_now` 式 SQLite barrier；terminal overlay 是 UI 的立即收敛来源，summary/quota 可以在 write controller 后续 flush 后最终一致。
- 对请求收尾里的同一实体写入，要优先消除“重复唯一键探测 + 紧跟二次更新”“先重算 rollup 再补 timing 再重算一次”这类单请求内自我放大；SQLite 压力常常不是来自单条大 SQL，而是来自几条语义重复的写语句连发。
- 对 owner-facing 聚合读路径，同样要先消除“整窗扫明细再丢弃大部分结果”和“同参请求因 cache key 掺入 runtime 抖动而无法合并”这两类读放大；它们会和写侧单写者争用同一 SQLite 预算，并以 `database is locked` 的形式回灌到前台。
- 对 summary open-range 这类总览接口，`usage_breakdown` 与 `non_success_tokens` 不能再为了 account/model 维度或 archive overlap 去扫整窗 raw preview rows、也不能先枚举整段 live invocation id 再和 archive 去重；这类明细必须优先落到 live aggregate + archive aggregate merge，否则 7d overview 会在看似“只读总览”时重新制造数据库压力。
- 当 owner-facing breakdown 需要 `model + reasoning` 粒度时，账号 totals/hourly 级 rollup 不够用；必须补独立的 breakdown rollup（例如 `upstream_account_usage_breakdown_hourly`）并走 `full-hour rollup + exact boundary tail + archive-hole fallback`，否则“只差一个 breakdown”就会把 7d/previous7d 重新打回整段 raw aggregate。
- 对新增 rollup target 的 rollout，还要显式处理“老 archive batch 已 materialized、但新 target 从未 replay”这一迁移态。不要把新 target 直接塞进旧的 materialized shortcut；正确做法是只保留已验证安全的 legacy target shortcut，并把缺新 rollup rows 的历史 batch reopen 到 backfill backlog，否则既会漏数，又会让 telemetry 误报为 healthy。
- 对已裁剪 payload 的 legacy archive，要逐个 target 判断是否真的需要完整 payload。`usage_breakdown` 这类可由结构化列和保留的最小 payload 回放的 target 应允许 materialize，并把不可恢复的 `reasoning_effort` 归入空/unknown；`prompt_cache_*`、`sticky_key` 这类 keyed target 仍应保持 blocked，避免为了清慢读而制造错误维度。
- 当某个 closed-range owner-facing 窗口本来就只需要 exact 结果时，不要为了“统一实时”把它硬塞进 pure SSE。`previous7d` 这类 comparison summary 若继续长期订阅 `stats.summary.current`，会把 archive fallback 重新钉在高频推送链路上，读压再怎么优化都会被不必要的订阅频率放大。
- 如果 read-side 仍需要 legacy archive 的新 rollup target，自愈调度也要把“可修复 backlog”和“永久 blocked target”拆开。像 `upstream_account_usage_breakdown_hourly` 这种缺 replay marker 但可结构化回放的 backlog，应在 startup/backfill 中单独优先 drain；不要继续和 `prompt_cache_*` / `sticky_key` 共用一个 `legacy_archive_pending` 信号，否则闭区间读会长期误判成“还有 backlog”，并反复打开 fallback。
- 当并发不能降低且业务成功率高于观测记录完整性时，用进程内短窗口 write controller 承接所有观测记录写：terminal invocation 进入 P1 best-effort 队列，attempt 中间进度、rollup/live progress、account touch、system task finish 等可延迟项进入 P2 并按 key coalesce。记录入队/flush 失败必须报警和计数，但不得让已经完成的业务响应失败。
- 高频 runtime snapshot 不应默认等同于主事实写。`running` / first-byte / response-ready 这类 UI 新鲜度事件可以先走进程内共享 runtime store + SSE/HTTP overlay；如果服务选择业务优先于记录，terminal success/failure 也可以先进入 P1 write controller，并用 SSE terminal payload + runtime tombstone 支撑短暂最终一致窗口。
- 路由公平性字段如果不是路由正确性的硬状态，可以拆成“进程内立即生效 + batch 落库”。例如 `last_selected_at` 可先写内存锚点并叠加候选排序，账号 status/cooldown/failure 则继续同步写。
- raw payload 完整保留属于观测合同，不应作为 SQLite 止血手段被截断或丢弃；只能补齐 raw IO / gzip / metadata 写入证据，并通过调度、窄写或配置化压缩策略减压。

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
- 不要让 P1 terminal flush 在同一锁窗口内继续执行 P2 rollup/account-touch 派生写；这会把“记录最终一致”重新变成“请求尾锁放大”。P1 成功后把 P2 放回队列，等待下一轮时间窗口或 pressure 允许。

## 何时升级方案

- 如果前台关键写入本身持续超过单写者能力，应用层背压只能缓解，不能替代数据库迁移。
- 如果需要跨进程 worker 或严格 FIFO，需要引入外部队列或 PostgreSQL，而不是继续扩大 SQLite 连接池。
