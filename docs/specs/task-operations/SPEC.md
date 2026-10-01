# 任务运维运行观测与生效计划

> This file is the durable topic requirements contract. Current implementation facts belong in `IMPLEMENTATION.md`; lifecycle and change references belong in `HISTORY.md`.

## Context and Scope

- Context: 任务运维页面需要同时表达真实当前执行、等待执行、最近 24 小时执行区间、任务让行状态、worker 的实际调度策略和可安全修改的运行配置。
- In scope: 维护任务目录、运行与等待观测、执行时间线、任务标识色、任务让行时间线、集中任务能力目录、计划覆盖控制、详情页和响应式筛选交互。
- Out of scope: 跨任务优先级队列、多实例聚合、任务业务数据归属和生产周期调整。

## Terms and Interfaces

- `运行快照`: 进程内有界登记器提供的当前执行实例；维护库只保存历史记录。
- `生效计划`: worker 默认规则与维护库自定义覆盖合并后的可展示策略，包含来源、触发机制和编辑能力。
- 待执行请求、准入延后任务、任务执行区间、任务标识色和任务让行状态采用 [CONTEXT.md](../../../CONTEXT.md) 的定义。
- Interface: `GET /api/system/managed-tasks/runtime` returns process-local executions plus separately available FIFO requests and admission waits; task catalog responses add persisted light/dark identity colors. `GET /api/system/managed-tasks/timeline` reads RFC 3339 windows up to 24 hours, with fixed-watermark pages of at most 500 segments and `afterRevision` incremental updates; an expired cursor returns `resetRequired` so the client can resynchronize. Existing task list/detail interfaces and control `PATCH` retain their prior fields and behavior.

## Requirements

### REQ-TASK-OPS-001

- The system MUST expose only actual work intervals in the runtime snapshot, with a stable execution identity, task identity, actual start time, monotonic elapsed time, trigger, phase, and evidence-backed execution class.
- Inputs: legacy workers and the managed dispatcher enter the shared observation boundary after admission and before work; parent backfill work may publish the active child identity.
- Outputs: `GET /api/system/managed-tasks/runtime` returns `observedAt` and `activeRuns`; normal, failed, cancelled, and unwound work removes its in-memory observation without depending on history persistence.

### REQ-TASK-OPS-002

- The system MUST provide a typed capability catalog for all root and startup-backfill tasks, including trigger mechanisms, effective policy, policy source, edit capability, and execution class.
- Reading the catalog MUST NOT write default values into schedule override columns or synthesize an authoritative next trigger for worker-only policies.

### REQ-TASK-OPS-003

- The system MUST distinguish omitted, `null`, and value schedule fields in task-control PATCH requests.
- New interval/UTC cron overrides MUST be accepted only for the six catalog-approved tasks, with a minimum interval of 60 seconds and five-field UTC cron validation. Existing unsupported overrides MUST remain readable and explicitly resettable.
- Resetting a schedule MUST clear both override fields and the derived next trigger while preserving `enabled`.

### REQ-TASK-OPS-004

- The web task workspace MUST show all active runtime instances above a compact task catalog, support enabled-state and multi-trigger filters with OR within triggers and AND across filter groups, and keep the runtime section independent from catalog filtering.
- Visible pages MUST poll runtime state every two seconds, update elapsed time locally every second from server elapsed milliseconds, refresh immediately on foregrounding, avoid overlapping requests, and show an unknown state after observation failure.

### REQ-TASK-OPS-005 — 当前等待任务

- 页面 MUST 在当前执行实例之外列出两类等待，分别标为“已入队”和“等待准入／压力延后”。等待任务 MUST 来自实际请求队列或当前调度边界的权威观测；等待下次正常定时、事件或启动的任务不得因此列入等待列表。
- 已入队请求 MUST 显示任务身份、触发来源、请求时间和等待用时；队列顺序仅在对应 dispatcher 有权威顺序时展示，不得将独立 worker 合成为跨任务 FIFO 或可编辑优先级队列。
- 准入延后任务 MUST 显示观测到的等待原因、观测时间，以及有权威依据时的重试或下次资格时间。不得根据历史延期记录或一般系统告警推测任务仍在等待。
- 当前执行与两类等待 MUST 独立于目录筛选刷新，明确区分空结果、观测过期与观测未知；父子任务共享实例时不得重复计数。

### REQ-TASK-OPS-006 — 固定任务配色

- 每个纳管任务 MUST 拥有固化在任务元数据中的任务标识色；已分配的颜色不得因排序、筛选、启停、触发方式、再次运行或服务重启而改变。既有任务须一次性补齐，新任务的分配不得重排旧任务颜色。
- 任务目录名称左侧 MUST 显示该颜色的 Dot；当前任务列表、等待列表、时间图色条与其任务标识 MUST 使用一致配色。不同任务须分配不同颜色，并在深浅主题下保持可见。
- 颜色 MUST 只表达任务身份；运行结果、等待类别与压力状态须有独立的文字、图标或条纹表达。任务名称与悬停详情须能支撑不依赖颜色的识别。

### REQ-TASK-OPS-007 — 最近 24 小时执行总览

- 时间图 MUST 展示滚动最近 24 小时内与窗口相交的实际任务执行区间，包括已经结束的运行和当前执行实例。跨越窗口边界的区间须裁剪显示并保留真实起止信息；不得将计划检查或排队时间画为实际执行。
- 时间图 MUST 使用紧凑多泳道，按执行区间的重叠关系分配行；行不固定归属某个任务。同一任务的多次运行使用同色独立区间，同时发生的运行不得互相遮蔽。
- 前端 MUST 每秒本地推进时间轴、当前时间标记和仍在执行的色条终点；执行状态以后台观测为准，本地计时不得制造结束结果或继续延伸已知过期的运行观测。页面恢复前台时立即校准，图表的每秒更新不得要求每秒重新获取目录或整段历史。
- 色条详情 MUST 提供任务名称、触发来源、真实起止时间、实际执行用时与结果；缺少的历史字段保持未知。短于实时刷新周期的任务也须能由执行边界记录进入历史；极短区间的可见标记不得改变原始耗时。
- 已结束区间 MUST 保留成功、失败、取消或其他已观测结果的区别，不得只展示成功记录。共享一个执行实例的父子任务不得画成两个并行任务；当前子任务身份须在对应实例详情中可辨认。
- 密集区间 MAY 采用像素级聚合标记，但 MUST 展示聚合数量并支持查阅其包含的实际运行；聚合不得改写真实起止、合并运行身份或静默丢弃记录。
- 目录筛选 MUST 保持当前执行和等待区的完整性；时间图默认展示全部任务的窗口内执行，色条须可关联到对应任务详情。窄屏须保留时间尺度与任务识别能力。

### REQ-TASK-OPS-008 — 任务让行状态行

- 时间图 MUST 保留一行与执行区间共用时间轴的任务让行状态，以不同颜色横条表达正常准入、资源占用让行、压力延后与观测未知等实际状态。
- 此行 MUST 聚焦会延后任务的压力、资源占用和准入限制；系统总体健康或与任务等待无关的告警不得直接作为让行状态。悬停须展示对应时间范围、原因、可确认的受影响任务以及可确认的恢复资格时间。
- 相邻且状态与原因一致的区间 SHOULD 合并。同一时段存在多种限制时须保留原因集合，不得在汇总为单条横条后丢失具体让行原因；未观测的时段不得标为正常。
- 当前状态 MUST 随后台观测刷新，前端每秒推进仍有效的开放区间；一般累计压力计数或性能时间桶不得冒充精确的状态起止区间。

### REQ-TASK-OPS-009 — 持久区间与观测覆盖

- 后端 MUST 独立于 HTTP 请求和页面打开持续采集实际执行边界及任务让行状态变化，并保留可恢复的最近 24 小时历史。服务重启后须读取已有区间，记录窗口之前开始、但仍与窗口相交的执行或状态也不得因保留策略丢失。
- 请求时间、实际执行开始、实际执行结束与实际执行用时 MUST 分别表达。当前快照和持久历史 MUST 通过同一执行身份关联并去重；跨重启不得复用一个身份将不同运行合并。
- 历史写入和采集 MUST 有界、异步、可合并且不会阻塞任务工作；新增执行区间、让行区间和覆盖诊断归维护库，性能指标库继续保存聚合指标。观测不可用不得回退为业务主库同步写入，也不得改变任务准入、启停、调度周期或无追赶规则。
- 停机、采集失败、丢弃、维护库不可用及功能启用前的无证据时段 MUST 显示缺失或未知覆盖。重启前未确认结束的运行须标明中断／结束未知，不得延伸至当前或伪造成功；已知记录不因部分缺口而消失。
- 旧历史记录中含入队含义的 `started_at` 和可能包含排队的 `duration_ms` MUST 保留原有兼容含义；不得重命名为实际执行起点，也不得通过旧耗时反推起点。新的字段或区间记录须通过前向、幂等且有界的维护库迁移安装，已有配置与任务颜色不被覆盖。
- 时间线读取 MUST 使用有界、可续取的窗口查询与增量刷新；高频刷新不得每次加载完整历史、扫描业务表或以静默数量截断冒充完整的 24 小时覆盖。压力区间保留须满足窗口覆盖，并合并相同状态及原因的相邻区间，不能为了前端一秒动画每秒写入相同状态。

## Verification

### VER-TASK-OPS-001

- Method: Rust unit tests for the observation registry and a controlled worker boundary.
- covers: `REQ-TASK-OPS-001`
- Pass condition: elapsed time is monotonic, child identity is attached, clones share one execution identity, and dropping the last guard removes the snapshot.

### VER-TASK-OPS-002

- Method: maintenance-store catalog and schedule-control tests plus HTTP contract tests.
- covers: `REQ-TASK-OPS-002`, `REQ-TASK-OPS-003`
- Pass condition: all 37 catalog rows have real policy metadata; unsupported additions fail; explicit null clears a legacy override and leaves `enabled` unchanged.

### VER-TASK-OPS-003

- Method: `SystemTasksPage` unit tests, Storybook interaction states, and a controlled local service/browser run.
- covers: `REQ-TASK-OPS-004`
- Pass condition: running/empty/unknown states render, combined filters produce the expected count, and desktop/mobile layouts preserve task identity and policy values.

### VER-TASK-OPS-004

- Method: controlled dispatcher queue, independent worker admission rejection, release, cancellation, and observation-failure fixtures; frontend state tests.
- covers: `REQ-TASK-OPS-005`
- Pass condition: queued requests and admission-deferred tasks remain distinct; normal future schedules stay out of the waiting list; observed release removes waiting; filtering cannot hide current work; missing evidence never appears as an empty healthy queue.

### VER-TASK-OPS-005

- Method: task-metadata initialization and upgrade fixtures, restart, new-task insertion, sorting/filtering, and light/dark rendered states.
- covers: `REQ-TASK-OPS-006`
- Pass condition: every task has a distinct stable identity color; existing colors survive initialization, restart and catalog growth; dots and execution bars agree, and textual identity remains available.

### VER-TASK-OPS-006

- Method: deterministic frontend clock and execution fixtures covering repeated, concurrent, very short, cross-window, ongoing, failed, stale and unknown runs; controlled desktop/mobile UI evidence.
- covers: `REQ-TASK-OPS-007`
- Pass condition: lanes avoid overlap, repeated runs retain task color, each local tick advances the rolling window and valid ongoing bars, finished runs stop, actual time excludes queuing, and short executions remain discoverable without one-second history requests.

### VER-TASK-OPS-007

- Method: task-admission transition fixtures with pressure cooldown, occupied resources, concurrent reasons, recovery, unrelated health degradation, and missing observation coverage.
- covers: `REQ-TASK-OPS-008`
- Pass condition: the dedicated row aligns with task execution time, exposes actual deferral reasons, retains concurrent causes, merges equivalent adjacent states, ignores unrelated alerts, and represents missing coverage as unknown.

### VER-TASK-OPS-008

- Method: restart, page-closed collection, migration, delayed/dropped history writes, queue-wait, parent/child, retention-boundary and dense-window fixtures; API pagination and frontend clock checks.
- covers: `REQ-TASK-OPS-007`, `REQ-TASK-OPS-008`, `REQ-TASK-OPS-009`
- Pass condition: history survives restart without live-run resurrection or duplicate bars; short runs appear while no page is open; actual execution time excludes request waiting; legacy unknowns and downtime remain visible; intersecting boundary intervals survive retention; dense groups retain inspectable run identities; reads remain bounded and observations never add synchronous main-database writes.

## Related ADRs

- [Task Runtime Observation and Effective Schedules](../../adr/0024-task-runtime-observation-and-effective-schedules.md)
- [Durable Task Execution and Deferral Timelines](../../adr/0026-durable-task-execution-and-deferral-timelines.md)

## Visual Evidence

- source_type: `ui_demo`
- target_program: `vite_web_demo`
- viewport_strategy: `ui-demo-source + devtools-emulate`
- desktop_viewport: `1440x900`
- mobile_viewport: `393x852`
- state: dark operational scene with an active dashboard projection reconciliation and all trigger filters selected
- owner_confirmation: confirmed in chat after candidate `f16c2c02`
- assets:
  - `./assets/task-operations-runtime-desktop.png`
  - `./assets/task-operations-runtime-mobile-393x852.png`
- `docs/solutions/maintenance/task-schedule-and-running-observation.md`

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
