# Runtime Read-Model Pressure Recovery

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## Related ADRs

- [ADR 0002: Exact Summary projection contract](../../adr/0002-exact-summary-projection-contract.md)
- [ADR 0003: Durable invocation classification](../../adr/0003-durable-invocation-classification.md)

## 背景 / 问题陈述

Summary、后台回填和长期投影共享 SQLite 的有限写入能力。Summary hydration 不能因大量 archive manifest 而放弃一个本可精确恢复的快照；压力门拒绝低优先级工作也不能演变为毫秒级重试、日志风暴或无动作审计；遗留长期 interval migration 不应反复执行全窗反关联扫描并阻塞 P1。

历史 invocation 的分类还不能让 Summary、rollup 与全量统计各自从 payload 或 raw response 推导。一个 terminal record 的 failure class 必须是有 revision 的 durable fact；旧 live/archive 记录由受控 materializer 取得 classification coverage，而不是由 HTTP 或某个读取器临时兼容。

这些是三个独立的运行态恢复问题，但它们必须共享同一条边界：SQLite 是耐久与恢复事实源，健康 Summary HTTP 是精确内存读模型，低优先级 maintenance 不得反向挤占代理 P1。

## 目标 / 非目标

### Goals

- 让 Summary Projection 对所有已支持的合法选择提供精确、内存优先的 `StatsResponse`，并以 durable rollup 加精确边界记录承接大历史。
- 将 terminal invocation classification 物化为统一、versioned durable fact，使 Summary、hourly rollup 与全量 aggregate 对同一记录给出同一成功/失败语义。
- 将 `SQLite Pressure Defer` 与真实 `BUSY`/`LOCKED` 失败分开，使 defer 只有一次 due/event wake，不制造轮询或无动作写入。
- 把长期 legacy migration 改为 cursor/seek 驱动的可取消微批，以保持 P1 优先级与可恢复 backlog。
- 将 checkpoint 发布与手动部署后的只读 900 秒观察作为独立证据链，禁止自动化部署操作。

### Non-goals

- 不扩大 SQLite 连接池，不以部分、近似或零值 Summary 换取可用性。
- 不在 Summary HTTP 请求期执行 SQL、archive 打开或文件扫描。
- 不修改 timeseries minute writer；该 ownership 留给独立的 #837 后续线。
- 不自动操作 Dockrev、srv-101、重启、回滚或部署镜像。
- 不把单次 terminal journal tail repair 或外部 forward-proxy 530 视为本主题的实现项；它们只在观察期记录是否复发。

## 范围（Scope）

### In scope

- Summary archive hydration 的容量、coverage 与 exact-boundary 恢复合同。
- terminal invocation classification、legacy/archive overlay 与 rollup classification coverage 的 materialization 合同。
- 全局 SQLite pressure gate 与 startup backfill 的 defer/due/wake 合同。
- long-term legacy interval migration 的 cursor/seek、短事务、pressure/cancel 合同。
- 四个交付 Ticket 的 checkpointed promotion 与只读生产观察证据。

### Out of scope

- Dashboard hot-topic 或 timeseries writer 的实现细节。
- HTTP wire shape、owner-facing UI 和运行时部署机制的重设计。
- 通过人工维护任务或请求期回源绕开缺失的 read-model coverage。

## 需求（Requirements）

### MUST

#### Exact Summary Projection

- 对已验证的合法 Summary query，HTTP handler 与 `SummaryCurrent` SSE topic materializer 只能读取内存 Projection；SQL 与文件访问计数必须为零。首次 hydration 尚未发布 Projection 时，两者必须返回现有 `unavailable` 语义，不得用 legacy builder 回源 SQLite。
- Projection 必须返回完整且精确的既有 `StatsResponse`，包括 totals、usage、model、reasoning、cost 与 account scope 语义；不得把部分 aggregate、空 totals 或临时近似当作正常响应。
- 历史全小时由 durable rollup 服务；任一 window 的未完整覆盖边界、live tail、account-lag 与 archive overlap 必须由精确记录补齐，并且 source partition 合并不得遗漏或重复。
- 最近索引超过固定预算时，Projection 必须保留首个省略 live 行的时间边界；任何覆盖该边界或更早时间的 rolling/calendar 全局或 account 请求必须 `unavailable`，不得以截断索引或请求期回源返回 totals；边界之后、完整保留的窗口继续从内存精确响应。
- `current` 的 newest-N 视图可以与 rolling/archive 的精确边界视图分开索引，但两者实际持有的 preview 行字符串必须共同受同一常驻字节上限约束；超过上限时不得以第二份副本扩大内存预算。
- `current` 只能在每个请求 scope 的 newest-N 候选来源均已被有界 admission 证明时返回成功；一个可能进入该前缀、但未被常驻 Projection 收录的 archive 记录必须使对应 global 或 account `current` 请求 `unavailable`，不得返回较短的 200。确认早于 selected cutoff 的 archive 不得阻断不受影响的 global `current`。
- archive 的不可读或未回放证明必须为 `current` 保留精确 manifest 时间端点；小时 bucket 扩展只用于 rolling/calendar 的 aggregate gap，不得把同小时但早于 selected cutoff 的 archive 放大为 `current` 不可用。materialized archive 的 partial raw boundary gap 只替换受影响的精确边界，不能否定已完整覆盖的 durable rollup interior；replay coverage 必须匹配当前 manifest identity，除非受控兼容标记明确定义为 materialized coverage。
- 所有具有有限 manifest coverage 的 raw replay 与 compact-rollup proof 都必须把 inclusive final-row timestamp 归一为同一 exclusive range；`coverage_start_at == coverage_end_at` 的单行 manifest 仍是有界 source，不能退化为无 coverage 的 legacy manifest。raw current-candidate admission 的 record-budget 溢出只能使受影响 selection 或 range `unavailable`，不得中止可由完整 durable rollup 服务的其他 Projection snapshot。
- 共享常驻字节预算无法同时容纳某个 rolling/archive 精确边界与独立 newest-N 视图时，必须回退该精确边界为范围局部 `unavailable`；已经完整证明的 `current` 和不相交的合法窗口继续从内存精确响应。
- runtime overlay 追加或替换导致再次裁剪 `current` 时，遗漏时间边界只能保持或向更新的遗漏记录收紧；旧 overlay 不得把已有持久化遗漏边界放宽，从而误放行覆盖该行的 rolling/calendar 请求。
- 仅为 `current` newest-N 候选而读取的 archive record 若在 current-index admission 中被裁剪，且该 record 已由相同 global/account totals 与 usage compact rollup 完整证明，则其裁剪不得写入 rolling/calendar 的遗漏时间边界；没有该完整 scope proof 的裁剪仍必须 fail closed。
- archive manifest 或历史容量超过固定内存 admission 预算时，系统必须使用受控的 rollup/boundary 恢复或明确可恢复状态；不得把合法的大历史永久降级为初始 hydration 失败。
- rolling 与 calendar 请求的 admission 只覆盖其合法 public horizon 和精确边界；仅 `all` 可达的更早 rollup 容量不得阻止合法 rolling snapshot 发布，且 `all` 继续保持 exact-or-unavailable。
- 后台 refresh 失败时保留可诊断的 last-good；它不能伪装为 fresh，也不能由 fabricated empty response 替代。首次尚无精确快照时保持现有 unavailable 语义。
- hydration、archive 读取和 reconcile 必须有 deadline、取消点、coalescing 与受控重试，不得在请求路径执行。
- 超出 raw live tail 的 persisted terminal 必须分别取得 global 与 account rollup coverage proof；global 已覆盖而 account 尚未覆盖时，全局 Summary 和通用 SSE baseline 可以继续精确去重，account rolling 请求必须 `unavailable`，不得因复用全局 proof 双计或漏计 terminal overlay。
- terminal record 的 `failure_class`、actionable state 与 classification revision 是 canonical durable facts。terminal persistence 必须在同一 durable write 中 materialize 当前 revision；读取器不得把 raw payload 或 response bytes 当作另一条分类事实源。
- legacy live rows 及 immutable archive rows 的 canonical classification 通过具有 durable cursor、identity-keyed archive overlay 和 coverage proof 的后台 materializer 取得。archive 文件不得为此被原地改写。
- Summary、hourly rollup、all-time aggregate 与 failure aggregate 必须消费同一 canonical classification。分类 coverage 缺失或 revision 不匹配时，只能得到 diagnosed `unavailable` 或等待后台 repair；不得把该记录当作 success、重新解析 payload，或用局部 aggregate 填补。
- canonical classification materializer 必须使用小批可恢复 transaction，按 pressure/cancel/deadline 让出 SQLite；其完成后需使受影响的 rollup/Projection coverage 可重新发布。

#### Pressure Defer And Backfill

- `SQLite Pressure Defer` 是数据库访问前的低优先级拒绝；它不读取或修改 durable progress，而是为每个 task/cooldown 只登记一次内存 scheduler next-eligibility deadline 与 event/deadline wake。
- Account Activity V2 coverage repair 由 startup task 持有唯一 background permit 后直接执行底层 repair；不得在已有 permit 时调用会再次获取 global gate 的 convenience wrapper。gate 拒绝时不得读取或写入 coverage progress，也不得写 task-run audit。
- coverage repair outcome 及其后每次 coverage progress 读写都在该 permit 内使用同一 SQLite 错误分类。实际 `BUSY`/`LOCKED` 在 permit 释放前恰好关闭一次 pressure gate，并作为 pressure-deferred outcome 返回外层 maintenance loop，不得写 task-run audit 或走普通 failure retry；repair 失败后的 retry progress 读写即使也返回锁错误，也不得产生第二个 pressure event；非锁错误保持普通 scheduler failure 路径、审计与重试，且不记录 pressure。
- defer 不得按毫秒级重试，不得反复写 `system_task_runs`，也不得在没有 actionable work 时记录成功、跳过或失败审计。
- 真正的 SQLite `BUSY`/`LOCKED` 仍按既有有界失败退避处理，并保留与 pressure defer 可区分的 telemetry、reason 与恢复路径。
- cooldown 到期、符合条件的 input 变化或 pressure eligibility generation 变化可以唤醒；eligibility wake 重新进入普通 durable due 检查，不得绕过未来的 `next_run_after`。固定 ticker 不得在 cooldown 内反复派发相同 task。

#### Long-Term Legacy Migration

- legacy interval migration 使用持久 cursor/seek 前进，不以重复的全窗 `NOT EXISTS` 扫描发现同一 backlog。
- 每个迁移、retention 或兼容清理写事务最多处理 512 行；在每个事务边界重新检查 pressure 与 shutdown/cancellation。
- pressure 或 cancellation 停止时，cursor 与剩余 backlog 必须保持可恢复，既有 last-good publication 不得被未完成迁移替换。
- P1 terminal durability 与同步路由写入始终优先于此类 P2 migration。

#### Promotion And Observation

- 每个 checkpoint 只发布 GitHub 制品；发布成功不表示已部署。
- 只有主人确认某个 exact release 已手动部署后，才允许使用 `$srv-101-ops` 进行 900 秒只读观察。
- 任何观察不得执行 srv-101 写入、Dockrev 操作、部署、重启或回滚。

### SHOULD

- 在 System Status 的内存诊断中暴露 snapshot freshness、pressure defer、actual lock、next eligibility 与 backlog，且读取诊断本身不增加 SQLite 查询。
- 用 production-shaped fixture 与 `EXPLAIN` 防止新的非 sargable 历史扫描回归。

### COULD

- 在不改 wire shape 的前提下补充 additive health counters，帮助区分完整性、defer 与 lock 的恢复进度。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

1. Summary HTTP 请求和 `SummaryCurrent` SSE topic 先校验 query，再从内存 Projection 选择精确 snapshot；它们从不启动 hydrate、打开 archive 或回源 SQLite。首次没有精确 snapshot 时返回 `unavailable`。
2. Terminal writer 在持久化 terminal outcome 的同一事务中保存 canonical classification。legacy/immutable archive materializer 在后台补齐 revisioned classification coverage，并重新建立受影响的 rollup coverage。
3. Projection worker 在后台将 rollup 的完整 interiors 与精确 boundary/tail 组合成新的 immutable snapshot；它只消费 canonical classifications，失败保持 last-good 与明确 freshness 状态。
4. 低优先级 backfill 遇到关闭的 pressure gate 时只注册一次内存 scheduler future-eligibility deadline；对应事件或 deadline 只重新选择候选任务，执行前仍检查 durable progress，durable progress 保持不变。Account Activity V2 coverage repair 在同一 permit 内完成这个 due 检查与 repair，避免嵌套获取 global gate。
5. long-term migration 读取 cursor 后执行一个最多 512 行的微事务；压力、取消或时间预算耗尽时安全退出，后续从持久 cursor 继续。
6. 每个 checkpoint 的 GitHub release 由累计 integration frontier 生成；主人部署确认后才开始相关 Ticket 的只读 900 秒观察。

### Edge cases / errors

- 无精确 Summary snapshot 时保留既有 unavailable 错误，不伪造 200/zero response。
- 过期或失败的 refresh 不得把不完整数据提升为 fresh；可诊断 last-good 与 freshness 必须一致。
- account rollup 落后 global rollup、archive/live overlap、source partition 与未对齐窗口边界都必须保持 exactness。
- 任何 legacy/archived row 的 classification revision 不足都不得由不同读取器各自推导；它阻止相应 exact coverage，直到 background materializer 完成。
- pressure defer 与实际 lock 在 counters、日志、重试与审计中是不同状态。
- 观察失败冻结相应 checkpoint 的后续 promotion，按 Initiative v4 recovery contract 处理，而不是自动部署或回滚。

## 接口契约（Interfaces & Contracts）

None。现有 Summary、System Status、long-term HTTP 与 SSE wire shape 保持不变；本主题只允许 additive、内存态 health 诊断。

## 关联合同

- `docs/specs/vmcs3-terminal-projection-materialization/SPEC.md`：P1/P2 ownership、cursor、pressure gate 与 bounded transaction。
- `docs/specs/5k89c-long-term-usage-analytics/SPEC.md`：长期统计、archive proof 与 canonical interval state。
- `docs/specs/9aucy-db-retention-archive/SPEC.md`：archive/retention durable coverage。
- `docs/specs/high-frequency-runtime-data-plane/SPEC.md`：健康 read path 的内存态边界。
- `docs/solutions/performance/sqlite-write-pressure-backpressure.md`：pressure defer 与 lock retry 的既有设计约束。

## 验收标准（Acceptance Criteria）

- Given 多于旧 manifest admission 上限的已验证 archive 历史，When Summary Projection hydrate，Then 合法 current/1d 与滚动窗口保持精确，且 HTTP 读取不执行 SQL 或文件访问。
- Given 一个 legacy terminal invocation 的 payload diagnostics 与现有 `failure_class` 不一致，When canonical materializer 完成，Then terminal row、archive overlay、hourly rollup、all-time aggregate 和 Summary 对该 invocation 使用同一 revisioned classification；在 completion 前，覆盖它的 Summary range 是 diagnosed unavailable 而非 partial 或 payload-derived response。
- Given 当前 terminal write，When durable transaction commits，Then canonical classification revision 与 terminal outcome 原子可见；后续 projection 不读取 raw payload 仍与 full aggregate 完全一致。
- Given 低 retention 且 31 天外的高基数 rollup 超出 admission，When Summary Projection hydrate，Then 合法 30d 仍精确、可用且仅从内存读取。
- Given global、account 与 source-partitioned rollup 覆盖不一致，When 请求边界或 account scope，Then 响应没有重复、漏项或缺失 usage/model/reasoning/cost 详情。
- Given 49,999 个 48 小时内的 live 行和至少两个仍在合法 rolling window 内、但落后 global/account cursor 的更早 live 行，When 最近索引超过 50,000 条，Then 覆盖省略边界的 global 与 account rolling 请求返回 `unavailable`，较新的完整窗口仍从内存精确响应且不执行 SQL 或文件 I/O。
- Given pressure cooldown 关闭，When 同一 startup backfill task 被触发，Then 只记录一次内存 scheduler next-eligibility/event wake，不产生毫秒级 defer storm、SQLite pre-read 或无动作 task-run audit。
- Given Account Activity V2 coverage repair 已被 background gate 接纳，When 它等待 hourly rollup lock 后执行，Then 底层 repair 恰好在该 permit 内运行一次；Given gate 拒绝，Then 不读取或写 coverage progress，也不创建 task-run audit。
- Given Account Activity V2 coverage repair 的 outcome 或后续 progress SQLite 操作返回实际 `BUSY`/`LOCKED`，When 该 permit 仍被持有，Then 只关闭一次 pressure gate，outer maintenance loop 不写 task-run audit 或通用失败重试，permit 释放后的下一个后台 task 以零 SQLite I/O defer；非锁错误不关闭 pressure gate，并保留 task-run failure audit。
- Given 实际 SQLite lock，When 后台任务已开始数据库访问且失败状态写入成功，Then 在释放 background permit 前关闭 pressure gate，使用独立的 bounded failure backoff，并使后续任务以零 SQLite I/O defer。
- Given 任务的 durable `next_run_after` 仍在未来，When 该任务先因 `BackgroundBusy` defer 后收到 pressure eligibility event，Then 它不执行、不写 progress 或 task-run audit，并继续等待原 deadline。
- Given legacy long-term backlog，When migration 运行、遇到 pressure 或取消，Then 每个写事务最多 512 行、cursor 可恢复且 P1 不被低优先级写入饥饿。
- Given 一个 checkpoint 已发布且主人确认 exact version 已部署，When 执行 900 秒 `$srv-101-ops` 只读观察，Then 结果绑定同一 release identity，不执行服务器写入。

## 验收清单

- [ ] Summary 的 exact-memory contract 覆盖 archive、rollup、boundary、last-good 与 HTTP zero-I/O。
- [ ] Canonical invocation classification 覆盖 terminal writer、legacy live、immutable archive overlay、rollup coverage 与所有 aggregate consumers。
- [ ] pressure defer、actual lock、next eligibility 与无动作审计边界明确且可测试。
- [ ] long-term migration 的 cursor、seek、512-row transaction、pressure/cancel 合同明确且可测试。
- [ ] 手动部署和 900 秒只读观察边界明确。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- 每个 child PR 运行定向回归、`cargo fmt --all -- --check`、`cargo check --locked --all-targets --all-features` 与 Clippy。
- 与 archive/SQLite 行为相符的 backend resource profile 通过 CI；integration frontier 使用 `Backend Tests (Stateful SQLite)` 的 exact SHA 结果。
- Summary Ticket 使用 production-shaped archive fixture、HTTP SQL/file counter 与 exactness 对账；long-term Ticket 使用 `EXPLAIN` 与 bounded-migration fixture。

### Quality checks

- child PR 与 aggregate/checkpoint PR 只能使用 current-head review 与 GitHub CI。
- 每次 checkpoint/aggregate 以 Initiative v4 guard 的 exact head/base、release 与 observation receipt 为准。

## Visual Evidence

PR: none
