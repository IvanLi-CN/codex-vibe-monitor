# Runtime Read-Model Pressure Recovery

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## 背景 / 问题陈述

Summary、后台回填和长期投影共享 SQLite 的有限写入能力。Summary hydration 不能因大量 archive manifest 而放弃一个本可精确恢复的快照；压力门拒绝低优先级工作也不能演变为毫秒级重试、日志风暴或无动作审计；遗留长期 interval migration 不应反复执行全窗反关联扫描并阻塞 P1。

这些是三个独立的运行态恢复问题，但它们必须共享同一条边界：SQLite 是耐久与恢复事实源，健康 Summary HTTP 是精确内存读模型，低优先级 maintenance 不得反向挤占代理 P1。

## 目标 / 非目标

### Goals

- 让 Summary Projection 对所有已支持的合法选择提供精确、内存优先的 `StatsResponse`，并以 durable rollup 加精确边界记录承接大历史。
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
- 全局 SQLite pressure gate 与 startup backfill 的 defer/due/wake 合同。
- long-term legacy interval migration 的 cursor/seek、短事务、pressure/cancel 合同。
- 三个交付 Ticket 的 checkpointed promotion 与只读生产观察证据。

### Out of scope

- Dashboard hot-topic 或 timeseries writer 的实现细节。
- HTTP wire shape、owner-facing UI 和运行时部署机制的重设计。
- 通过人工维护任务或请求期回源绕开缺失的 read-model coverage。

## 需求（Requirements）

### MUST

#### Exact Summary Projection

- 对已验证的合法 Summary query，HTTP handler 只能读取内存 Projection；SQL 与文件访问计数必须为零。
- Projection 必须返回完整且精确的既有 `StatsResponse`，包括 totals、usage、model、reasoning、cost 与 account scope 语义；不得把部分 aggregate、空 totals 或临时近似当作正常响应。
- 历史全小时由 durable rollup 服务；任一 window 的未完整覆盖边界、live tail、account-lag 与 archive overlap 必须由精确记录补齐，并且 source partition 合并不得遗漏或重复。
- archive manifest 或历史容量超过固定内存 admission 预算时，系统必须使用受控的 rollup/boundary 恢复或明确可恢复状态；不得把合法的大历史永久降级为初始 hydration 失败。
- 后台 refresh 失败时保留可诊断的 last-good；它不能伪装为 fresh，也不能由 fabricated empty response 替代。首次尚无精确快照时保持现有 unavailable 语义。
- hydration、archive 读取和 reconcile 必须有 deadline、取消点、coalescing 与受控重试，不得在请求路径执行。

#### Pressure Defer And Backfill

- `SQLite Pressure Defer` 是数据库访问前的低优先级拒绝；每个 task/cooldown 只登记一次 next eligibility 与 event/deadline wake。
- defer 不得按毫秒级重试，不得反复写 `system_task_runs`，也不得在没有 actionable work 时记录成功、跳过或失败审计。
- 真正的 SQLite `BUSY`/`LOCKED` 仍按既有有界失败退避处理，并保留与 pressure defer 可区分的 telemetry、reason 与恢复路径。
- cooldown 到期、符合条件的 input 变化或 pressure eligibility generation 变化可以唤醒；固定 ticker 不得在 cooldown 内反复派发相同 task。

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

1. Summary 请求先校验 query，再从内存 Projection 选择精确 snapshot；它从不启动 hydrate 或打开 archive。
2. Projection worker 在后台将 rollup 的完整 interiors 与精确 boundary/tail 组合成新的 immutable snapshot；失败保持 last-good 与明确 freshness 状态。
3. 低优先级 backfill 遇到关闭的 pressure gate 时只注册一次 future eligibility；对应事件或 deadline 才重新尝试。
4. long-term migration 读取 cursor 后执行一个最多 512 行的微事务；压力、取消或时间预算耗尽时安全退出，后续从持久 cursor 继续。
5. 每个 checkpoint 的 GitHub release 由累计 integration frontier 生成；主人部署确认后才开始相关 Ticket 的只读 900 秒观察。

### Edge cases / errors

- 无精确 Summary snapshot 时保留既有 unavailable 错误，不伪造 200/zero response。
- 过期或失败的 refresh 不得把不完整数据提升为 fresh；可诊断 last-good 与 freshness 必须一致。
- account rollup 落后 global rollup、archive/live overlap、source partition 与未对齐窗口边界都必须保持 exactness。
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
- Given global、account 与 source-partitioned rollup 覆盖不一致，When 请求边界或 account scope，Then 响应没有重复、漏项或缺失 usage/model/reasoning/cost 详情。
- Given pressure cooldown 关闭，When 同一 startup backfill task 被触发，Then 只记录一次 next eligibility/event wake，不产生毫秒级 defer storm 或无动作 task-run audit。
- Given 实际 SQLite lock，When 后台任务已开始数据库访问，Then 使用独立的 bounded failure backoff，而不是 pressure defer 语义。
- Given legacy long-term backlog，When migration 运行、遇到 pressure 或取消，Then 每个写事务最多 512 行、cursor 可恢复且 P1 不被低优先级写入饥饿。
- Given 一个 checkpoint 已发布且主人确认 exact version 已部署，When 执行 900 秒 `$srv-101-ops` 只读观察，Then 结果绑定同一 release identity，不执行服务器写入。

## 验收清单

- [ ] Summary 的 exact-memory contract 覆盖 archive、rollup、boundary、last-good 与 HTTP zero-I/O。
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
