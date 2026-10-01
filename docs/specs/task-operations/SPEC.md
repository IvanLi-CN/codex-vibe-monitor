# 任务运维运行观测与生效计划

> This file is the durable topic requirements contract. Current implementation facts belong in `IMPLEMENTATION.md`; lifecycle and change references belong in `HISTORY.md`.

## Context and Scope

- Context: 任务运维页面需要同时表达真实当前执行、worker 的实际调度策略和可安全修改的运行配置。
- In scope: 维护任务目录、运行快照 API、集中任务能力目录、计划覆盖控制、详情页和响应式筛选交互。
- Out of scope: 跨任务优先级队列、多实例聚合、任务业务数据归属和生产周期调整。

## Terms and Interfaces

- `运行快照`: 进程内有界登记器提供的当前执行实例；维护库只保存历史记录。
- `生效计划`: worker 默认规则与维护库自定义覆盖合并后的可展示策略，包含来源、触发机制和编辑能力。
- Interface: `GET /api/system/managed-tasks/runtime`、现有任务列表/详情接口和任务控制 `PATCH`。

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

## Related ADRs

- `docs/adr/0024-task-runtime-observation-and-effective-schedules.md`

## Visual Evidence

- `docs/solutions/maintenance/task-schedule-and-running-observation.md`

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
