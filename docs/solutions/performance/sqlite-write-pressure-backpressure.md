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
- 对请求收尾里的同一实体写入，要优先消除“重复唯一键探测 + 紧跟二次更新”“先重算 rollup 再补 timing 再重算一次”这类单请求内自我放大；SQLite 压力常常不是来自单条大 SQL，而是来自几条语义重复的写语句连发。
- 当并发不能降低且主事实必须同步落盘时，用进程内短窗口 batch writer 承接派生写：terminal invocation / terminal attempt 仍同步写，attempt 中间进度、rollup/live progress、account touch、system task finish 等可延迟项按 key coalesce 后批量写入。
- raw payload 完整保留属于观测合同，不应作为 SQLite 止血手段被截断或丢弃；只能补齐 raw IO / gzip / metadata 写入证据，并通过调度、窄写或配置化压缩策略减压。

## 推荐模式

### 1. 写入分级

- 前台关键写入：OAuth callback、请求路由状态、用户可见设置保存；优先拿连接，失败需返回明确业务错误。
- 请求收尾写入：invocation 记录、usage、raw metadata；允许有界降级和异步旁路。
- 请求收尾若已经存在对应 `running/pending` 行，优先原地更新而不是先 `INSERT OR IGNORE` 再走 repair/update；这样可以少一次唯一键冲突写尝试与后续锁竞争。
- terminal invocation 主事实必须继续同步落盘；安全做法是“已存在 running row 时窄 `UPDATE`，缺行时 `INSERT OR IGNORE`，冲突后重读并按同一状态守卫更新”，避免宽 `ON CONFLICT DO UPDATE` 在竞态下覆盖已终态记录。
- 对同一 attempt 的 phase、latency、capability/compact-support 等进度字段，优先并入同一条前台更新，而不是拆成 `phase bump -> latency patch -> finalize` 的多段慢写；减少单请求尾部把 SQLite 单写者预算切碎。
- 对同一 attempt 的中间 phase、latency、capability/compact-support 等进度字段，如果不需要立刻作为业务决策真相源，可进入 250ms 级短窗口缓冲并按 `attempt_id` 只保留最新值；terminal finalize 必须同步一次写全并通过 `status=pending AND finished_at IS NULL` 防止未 flush 进度覆盖终态。
- Invocation 派生写可以按 `invocation_id` coalesce：hourly rollup/live progress 与 upstream account last activity touch 同事务批量执行。队列满时应优先同步补偿 invocation 派生写，而不是静默丢弃会影响后续统计的 truth maintenance。
- `system_task_runs` 的 begin 仍同步记录 running 审计入口；finish 可以进入 batch writer，pressure 下延迟或合并，但 shutdown 需要 drain 或记录未 flush 证据。
- 后台维护写入：rollup、retention、account maintenance；pressure 下 fail-soft skip。
- 历史回填写入：startup backfill、archive materialization；pressure 下延后，不阻塞 readiness。

### 2. DB pressure gate

- gate 只包低优先级后台任务。
- 任一后台任务遇到 SQLite busy/locked 或 pool acquire timeout，记录 pressure event 并进入 cooldown。
- cooldown 内后台任务返回 success-like skip，由原有 ticker / coalesced follow-up 继续收敛。
- batch writer 的最大等待 flush 不应在 pressure gate 关闭时强抢前台写锁；可记录 stale/max-age 证据并延后派生写，但 shutdown/barrier 这类完整性路径仍可旁路 gate。
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
- batch writer 必须有有界队列、flush 触发（时间窗口 / row count / 最大等待）、coalesced row count、oldest age、flush elapsed、queue depth 与 dropped count 证据；否则只是把 SQLite 锁问题藏到内存里。
- buffered progress 不能立刻广播“已持久化”的 DB snapshot；要么广播内存态，要么等后续 reconcile/terminal 更新。否则会把 stale DB state 伪装成实时状态。
- 为每个后台入口单独做局部退避，容易遗漏同一压力窗口内的其他维护任务；进程级 gate 更容易统一行为。
- `SELECT MAX(id) ... WHERE <稀疏条件>`、`NOT EXISTS` + 低选择性 phase 过滤这类查询，即使最终只返回 1 行，也可能在 SQLite 上吃掉秒级读锁预算；若它们会与前台 HTTP 共享同一数据库，必须先压成 cursor 读取或用 partial index 固定扫描面。
- 对 proxy 收尾这类 SSE follow-up，`receiver_count()==0` 应该直接意味着“跳过 follow-up”，而不是继续排队 summary/quota 或 rollup refresh；否则会把没有任何订阅者的请求变成纯数据库放大器。
- proxy snapshot/broadcast 在 `database is locked` 下应 fail-soft skip 并记录结构化证据，依赖已发出的 SSE 事件和后续 HTTP reconcile 补齐 UI；不要在请求尾部立即重试并放大锁争用。
- write-side live read model 只有在维护成本本身也受控时才值得做：前台请求内同步维护最小必要 working-set / in-progress truth，后台 rebuild 和补偿刷新则继续挂到统一 pressure gate/cooldown，避免为了止住读热点又新增一组不受控维护写入。

## 何时升级方案

- 如果前台关键写入本身持续超过单写者能力，应用层背压只能缓解，不能替代数据库迁移。
- 如果需要跨进程 worker 或严格 FIFO，需要引入外部队列或 PostgreSQL，而不是继续扩大 SQLite 连接池。
