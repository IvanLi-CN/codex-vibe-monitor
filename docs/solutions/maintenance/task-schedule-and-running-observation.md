---
title: Task schedule and running observation
module: task-operations
problem_type: configured-and-effective-schedule-mismatch
component: Managed task catalog and runtime observation
tags:
  - maintenance
  - scheduling
  - observability
status: active
related_specs:
  - docs/specs/s7m3q-system-workspace/SPEC.md
---

# Task schedule and running observation

## Symptoms

任务目录只显示启用状态和触发类型，没有正在执行的任务摘要。详情页的计划方式默认选中固定间隔，但间隔输入框为空；任务实际仍可能定期工作。请求入队后，详情页可能把尚未执行的任务显示为“运行中”。

## Root cause

- `managed_tasks.interval_secs`、`cron_expr` 和 `next_trigger_at` 可为空。`seed_tasks()` 只写入目录元数据和启用控制，不写入默认计划。列表和详情 API 直接读取这些字段，没有合成实际调度策略。
- 这些字段保存的是运维自定义计划。字段为空时，任务仍可能由原后台 worker 按常量、运行配置或自适应策略执行；为空不等于没有计划。
- `legacy_worker_should_skip()` 在自定义 interval 或 cron 存在时跳过原 worker，交由 managed dispatcher 执行。直接把原 worker 的默认周期补写进这两个字段，会改变执行路径。
- 目录 `trigger_mode` 可能与实际 worker 不一致。例如 `dashboard_runtime_projection_reconcile` 标记为 `event`，实际在非 Legacy 运行模式下每 60 秒检查一次，并受数据库压力准入约束。
- 前端只根据是否存在 `cronExpr` 选择计划方式；缺少 cron 时无条件选择 interval。目录标签还会用 cron 覆盖原触发类型，而非解释其与原执行机制的关系。
- 目录页只在挂载时读取一次。详情页用最近一条历史记录的 `requested` 或 `running` 判断“运行中”；请求入队时写入的 `started_at` 不等于实际执行开始时间。claim 只改 status，实际执行用时仅在 dispatcher 的内存计时器中计算。

上述结论来自源码，不等于已检查某个部署实例的运维自定义配置。

## Resolution principles

### Separate configured and effective scheduling

保留持久化自定义计划，另外提供只读生效策略。生效策略必须使用与实际 worker 相同的配置和策略来源，避免在前端复制常量或猜测默认值。

| 信息         | 含义                                                                   |
| ------------ | ---------------------------------------------------------------------- |
| 自定义计划   | 运维设置的固定间隔或 UTC cron；没有覆盖时为空                          |
| 生效策略     | 当前执行路径采用的定时、事件、启动或自适应策略；多种机制并存时分别说明 |
| 策略来源     | 系统默认、运行配置或运维自定义                                         |
| 检查间隔     | 检查是否有可执行工作所用的周期，不承诺每次检查都会执行                 |
| 下次计划检查 | 调度器或 worker 能权威给出的下一次检查点；不能推导时保持未知           |
| 编辑能力     | 当前任务明确支持的安全覆盖方式，不能只用 `!is_manual` 判断             |

固定间隔显示明确数值和单位，UTC cron 显示表达式和时区，自适应策略显示实际规则。例如系统状态快照的检查周期由 60 秒缓存上限减去 5 秒提前量得到，展示的是 55 秒检查周期，而非笼统的“固定间隔”。账号维护的检查周期与单账号同步间隔应分别说明。

`next_trigger_at` 仅覆盖 managed 调度器记录的下一次计划触发；不能为原 worker 用每次请求时的 `now + interval` 伪造下一次执行时间。已停用、等待事件、等待启动、运行模式不适用和观测未知需要分别呈现。

计划编辑必须明确它会切换执行路径还是增加安全兜底探测。事件任务保留事件路径、定时仅作兜底的约定见 ADR 0023；不具备此实现时，不能把覆盖原 worker 的行为描述为兜底。已有自定义值不得在目录修复、初始化或启用时被静默清除。

### Observe execution independently from history

正在执行的任务需要有明确的实际执行边界。利用执行租约建立有界进程运行快照，在开始执行时记录墙钟开始时间和单调时钟起点，退出执行边界时移除。原 worker 和 managed dispatcher 应共同发布；仅观察 managed 队列会漏掉默认执行路径。

任务历史和进度仍由维护库异步保存。历史持久化失败或终态写入延迟不应把已结束的任务永久显示为正在执行；运行快照不可用时应显示未知，不能显示“没有任务运行”。请求时间、等待时长和执行开始时间应有独立含义，不能通过重命名旧历史字段伪造实际开始时间。

回填子任务与父任务共享执行租约，应在一个执行实例下补充当前子任务身份，避免把父子任务重复计算为两个并行工作。已有记录缺少真实执行起点时，执行用时保持未知。

任务执行／让行级别只说明实际资源规则。managed dispatcher 的 FIFO、账号维护分层、数据库写入类别和后台压力准入是不同概念，不应合成虚构的统一任务优先级。没有对应规则时显示“未定义”；多阶段任务不能用其中一个 SQL 类别冒充整个任务的调度等级。

### Keep the operator view coherent

顶部运行区展示全部当前执行实例及名称、执行／让行级别、实际开始时间和持续更新的用时；下方目录独立按启用状态和真实触发方式组合过滤。目录过滤不能隐藏运行区正在工作的任务。等待执行与正在执行分别标明，空结果与观测不可用分别处理。

刷新必须能发现页面打开后才开始的任务，不能只在已有运行时轮询。客户端用时更新不必每秒重新请求整个目录，刷新响应也不能覆盖未保存的计划表单。

## Verification and reuse notes

- 为系统默认、运行配置、自定义 interval、自定义 cron、事件／启动、自适应、停用和不适用模式分别核对 API 与 worker 的一致性。
- 验证读取生效计划不会修改自定义控制；“恢复默认”只撤销明确的覆盖，并恢复该任务受支持的默认执行机制。
- 验证请求等待、开始执行、资源让行、执行结束、终态持久化失败和重启后的状态区分；用时不能包含未标明的排队等待。
- 验证目录两类过滤可组合，未过滤运行区、过滤后空结果、运行观测缺失和多个同时执行实例都可辨认。
- UI 实现阶段补桌面、窄屏及可控场景证据；源码诊断和设计文档本身不构成渲染验证。

## References

- `src/maintenance_store.rs`: schema, task registry, seeding, legacy skip rules, request/claim lifecycle, catalog queries.
- `src/runtime.rs`: managed dispatcher and one-off worker dispatch.
- `src/api/slices/error_distribution_and_sse/dashboard_live_projection.rs`: 60-second reconcile cadence and pressure deferral.
- `src/api/slices/system_routes_and_tasks.rs`: system status cadence and managed task APIs.
- `web/src/pages/system/SystemTasksPage.tsx`: catalog fetch and rendering.
- `web/src/pages/system/SystemTaskDetailPage.tsx`: schedule form and inferred active state.
- `web/src/pages/system/taskLabels.ts`: trigger and next-trigger presentation.
- `docs/adr/0023-task-operations-state-outside-main-database.md`: maintenance-state ownership and event fallback contract.
