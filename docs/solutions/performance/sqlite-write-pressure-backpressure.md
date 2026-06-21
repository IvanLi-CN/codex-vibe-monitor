# SQLite 写入压力与后台背压模式

## 适用场景

- 单进程服务使用 SQLite 作为主库，同时存在前台 HTTP 写入、请求收尾写入、后台维护写入与历史回填写入。
- 线上故障表现为 `database is locked`、连接池 acquire timeout、后台任务堆积，并最终把前台请求放大成 5xx。
- 短期不迁移数据库，也不引入独立队列。

## 核心结论

- SQLite 单写者约束下，后台任务必须是可跳过、可退避、可重试的低优先级工作。
- 前台关键路径不应该和 rollup/backfill/retention/maintenance 使用同等重试预算。
- 连接池等待超时本身就是 pressure signal，应触发后台 cooldown，而不是继续并发重试。

## 推荐模式

### 1. 写入分级

- 前台关键写入：OAuth callback、请求路由状态、用户可见设置保存；优先拿连接，失败需返回明确业务错误。
- 请求收尾写入：invocation 记录、usage、raw metadata；允许有界降级和异步旁路。
- 后台维护写入：rollup、retention、account maintenance；pressure 下 fail-soft skip。
- 历史回填写入：startup backfill、archive materialization；pressure 下延后，不阻塞 readiness。

### 2. DB pressure gate

- gate 只包低优先级后台任务。
- 任一后台任务遇到 SQLite busy/locked 或 pool acquire timeout，记录 pressure event 并进入 cooldown。
- cooldown 内后台任务返回 success-like skip，由原有 ticker / coalesced follow-up 继续收敛。
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
- 为每个后台入口单独做局部退避，容易遗漏同一压力窗口内的其他维护任务；进程级 gate 更容易统一行为。
- `SELECT MAX(id) ... WHERE <稀疏条件>`、`NOT EXISTS` + 低选择性 phase 过滤这类查询，即使最终只返回 1 行，也可能在 SQLite 上吃掉秒级读锁预算；若它们会与前台 HTTP 共享同一数据库，必须先压成 cursor 读取或用 partial index 固定扫描面。

## 何时升级方案

- 如果前台关键写入本身持续超过单写者能力，应用层背压只能缓解，不能替代数据库迁移。
- 如果需要跨进程 worker 或严格 FIFO，需要引入外部队列或 PostgreSQL，而不是继续扩大 SQLite 连接池。
