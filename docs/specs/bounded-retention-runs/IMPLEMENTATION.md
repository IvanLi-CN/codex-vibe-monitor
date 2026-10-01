# 有预算、可恢复的 Retention：实现映射与具体方案

## Current Status

- Implementation: Candidate 已实现；代码、迁移、API、Demo 页面及主要验证证据已进入交付收敛，审查修复批次已补齐失败、计时和历史保留契约。
- Lifecycle: active。
- 当前分支：`th/retention-bounded-recovery-design`，开发基线为 `a02b08f12c84d3c52ce4bae6f2c3e58a31243aab`。
- 已提交设计基线：`b018ae06`；本轮实现提交为 `b151c895`，修复批次提交后刷新审查证据再创建 PR。
- shared-testbox 三个资源 profile 已通过，百万行候选/开发基线对照也已完成；v2.71.x/v2.80.2 前向修复夹具已通过，视觉比较确认仍是交付前门禁。

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
| REQ-BRR-008/009/010/011/013/015 | `web/src/lib/api/core-foundation.ts`, `web/src/pages/system/SystemTaskDetailPage.tsx`, `web/src/pages/system/taskLabels.ts`, `web/src/demo/handlers.ts` | 已实现可选字段兼容、成果/阶段/单位/计划、未知/过期/统计待刷新显示；Storybook、Web 单测、typecheck、lint、build 和任务页 E2E 已通过，快照比较待确认       |
| REQ-BRR-014                     | `src/maintenance_store.rs`, `src/prompt_cache_conversations.rs`, `src/tests/stateful_sqlite/*`                                                          | 已实现幂等 DDL、主库续作结构、维护库 nullable 字段和代次回归；v2.71.45/v2.80.2 前向修复、重复 DDL、中断 staging/generation 保留及 ready 不回填回归已通过 |

## Compatibility and Migration

计划影响：public API 为 patch（原有字段及 status 保留，新增可选字段），持久化语义为 minor（新的跨页及孤儿续作状态只承诺前向恢复），整体按 minor 准备。Candidate 已采用该分类，边界旧 schema、重启修复和百万行实测均已有证据；最终发布结论仍依赖视觉门禁。分别记录在 [version impact](assets/version-impact-record.json) 和 [migration record](assets/persistent-state-migration-record.json)。

主库只增加业务正确性需要的 cursor、source 范围、刷新代次/暂存和索引。维护库增加 nullable 结果/快照字段。结构安装幂等且不历史扫描；运行时逐页产生 DML；旧进度、历史耗时和完成度保持未知。既有 archive artifact、保留天数和 wire 格式不变。

支持状态以既有 retention 的 v2.71.x 旧 schema 和本次生产 v2.80.2 schema 为必测边界；当前仓库没有额外可复现的中间 retention schema 快照，因此不把未验证的中间结构写成已覆盖事实。候选读取旧状态，而旧 Minor 程序不承诺维护新续作语义。停止发布后的恢复采用新版本前向修复，备份恢复单独处理。`ensure_schema_repairs_v271_and_v2802_retention_fixtures_without_historical_backfill` 固定了两个声明边界的旧 invocation/队列形状，重复运行 DDL，并验证历史行、未完成队列和 staging generation 不丢失；新增结构仍由幂等 DDL 和缺列修复覆盖。

## Verification and Remaining Gaps

当前已完成：Rust `cargo fmt --all -- --check`、`cargo check --locked --all-targets --all-features`、`cargo clippy --locked --all-targets --all-features -- -D warnings`；Prompt 统计代次/分页、预算入口、retention 和性能窗口定向回归；Web 单测 1,682 项通过、typecheck、lint（仅既有 warning）和 build；Storybook 136 项通过；任务页 E2E 7/7 通过。shared-testbox 三 profile 通过：lightweight 1,246/1,246、stateful-sqlite 1,375/1,375、archive-file-io 299/299。profile 耗时不作为候选性能结论，百万行对照单独记录在 benchmark card。

百万行同种子对照也已完成：同一测试机上候选 130 万行/热 key 50 万行，刷新 370.8s、在线读 p95/p99 为 238/290ms；开发基线刷新 101.7s、在线读 p95/p99 为 230/253ms；两者精确计数均为 500,000 且队列归零。候选读延迟在本次负载下略高于基线，分页提交带来的总刷新耗时约为基线 3.65 倍，不能把有界执行表述为吞吐提升；证据卡保留该限制。

尚未形成最终证据的项目：最终截图的基线视觉比较确认。任何缺少该证据的验收项保持未验证，不降低门槛。

实施验收对应 SPEC 的 VER-BRR-001..007。代表性大表至少包含超过百万 invocation、稀疏孤儿和单 key 倾斜；同时施加在线读写，验证有界查询、主库锁释放、公平推进及静默后统计收敛。性能门槛不能用 4,000-key/40,000-invocation 小 fixture 代替，也不能从生产的历史时间倒推保证。

Rust 回归按 `lightweight`、`stateful-sqlite`、`archive-file-io` 合同分桶；真实 archive/file/锁行为留在 archive-file-io。重型及集成验证直接在 shared-testbox 运行。Web 覆盖接口可选字段、结果状态、默认调度、未知/过期和暂不可用。渲染改动已在本地 Demo 生成桌面/移动端状态证据；主人已确认当前桌面、移动端和 partial 状态截图准确。

## Visual Evidence

最终 Demo 桌面状态已显示五项同口径指标、默认 `3600s` 有效计划、阶段检查点、运行完成度、Prompt 统计“暂不可用（积压 3）”和性能覆盖率；移动端状态已显示按钮、核心指标和可滚动任务内容。Storybook 与任务页 E2E 已覆盖 completed、partial、deferred、failed、未知/过期和统计待刷新。当前截图来自本地 Demo，主人已确认截图准确；这些截图是 owner-facing 的本地 Demo 证据，不冒充生产页面。

- [桌面任务详情](assets/retention-task-desktop.png)
- [移动端完成状态](assets/retention-task-mobile.png)
- [移动端部分完成状态](assets/retention-task-partial-mobile.png)

## References

- [长期需求](SPEC.md)
- [主题历史](HISTORY.md)
- [ADR 0025](../../adr/0025-retention-core-and-conversation-derived-maintenance.md)
