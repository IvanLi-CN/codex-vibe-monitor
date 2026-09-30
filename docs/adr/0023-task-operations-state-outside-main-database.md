# Task Operations State Outside the Main Database

Status: Accepted

Task management state lives in a new local maintenance SQLite database, separate from both the business main database and the existing performance telemetry database. The maintenance database owns task definitions and controls, enablement and trigger configuration, progress snapshots, task-run history, and bounded error summaries. Runtime tasks publish these records asynchronously on a best-effort basis; the task execution path never waits for the maintenance database and never falls back to synchronous writes in the business main database.

The performance telemetry database keeps its existing role as a bounded, low-cardinality aggregate store. It receives task overhead and system-pressure measurements such as duration, throughput, database waits, latency distributions, failures, and deferrals. It does not store task configuration, progress cursors, or run-history detail.

The existing `system_task_runs` and `startup_backfill_progress` records are migrated to the maintenance database, with a bounded compatibility read window during rollout, after which new writes to those main-database tables stop. Other task-specific state remains in the main database only where it is part of business or projection correctness; each such state is classified separately before being moved or mirrored.

If the maintenance database is unavailable, tasks continue according to their existing execution and admission rules. The UI reports stale or missing operational coverage, and control writes report failure. There is no main-database fallback and no fabricated completion or run record.

## Initial Migration Boundary

The first migration moves these records into the maintenance database and stops adding their new operational writes to the main database:

- `system_task_runs`: task start, finish, duration, result, summary, trigger, and bounded error detail.
- `startup_backfill_progress`: the sixteen startup-backfill task cursors, due times, suspension state, enablement, and last-pass counters. This is an explicit exception to the usual rule because it is already a task scheduler checkpoint and has been accepted as operational state.
- New task registry and control rows: task identity, page metadata, enabled state, trigger mode, interval or cron expression, pause state, and control-generation metadata.
- New page-only progress snapshots for upstream account maintenance, forward-proxy subscription refresh, retention/archive, raw orphan sweep, raw-payload inventory, pool orphan recovery, startup backfill (parent and children), hourly-rollup bootstrap, dashboard projection reconcile, summary snapshot, summary coverage recovery, long-term statistics, timeseries projection, invocation-timeline cleanup, and system-status snapshot refresh.

The following remain authoritative in the main database during the first phase because changing them changes product data, archive publication, deletion safety, or projection correctness:

- Retention and archive correctness: `archive_batches`, `retention_prepared_archives`, `retention_recovery_cursors`, and `retention_raw_reconciliation`.
- Raw-storage safety and circuit-breaker state: `system_raw_payload_metrics`, `system_raw_payload_inventory_paths`, and raw payload ownership/link tables.
- Rollup and projection facts/checkpoints: `hourly_rollup_materialized_buckets`, invocation and prompt-cache rollups, `parallel_work_*` rollups, `timeseries_minute_projection_*`, `summary_*` coverage/checkpoint tables, and `long_term_*` projection/state tables.
- Existing product control-plane and account-routing state, including forward-proxy settings and upstream account/session/cooldown records.
- The source invocation, account, quota, archive, and proxy-attempt facts consumed by those workers.

Management pages read asynchronous summaries of the retained main-database state from the maintenance database. The summary is diagnostic and may be stale; it never replaces the main-database source of truth. The performance sampler is already isolated to the performance telemetry database and remains there rather than being copied into either task state or business tables.

## Operational Policies

- Progress keeps one latest snapshot per task and does not record a high-frequency history. Task-run summaries are retained for 90 days; bounded, redacted error detail is retained for 30 days. Performance metrics follow the existing performance-database retention policy.
- Manual maintenance operations have their own pages, but no interval or cron field. Their enable switch means that manual invocation is allowed, and the primary action is an explicit one-off run.
- A task has at most one active run. A manual invocation while the task is active returns a conflict instead of queueing another run. A disabled or already-active schedule occurrence is skipped rather than accumulated for a later burst.
- Disabling uses the safe-boundary pause contract. The current batch or checkpoint may finish; new work does not start. Re-enabling waits for the next normal eligibility unless the operator explicitly invokes a one-off run.

## Database Path

The path is configurable through `MAINTENANCE_DATABASE_PATH`. By default, the service derives a sibling file from `DATABASE_PATH` using the same basename stem and the `.maintenance.sqlite` suffix. For example:

```text
DATABASE_PATH=/srv/app/data/codex.sqlite
MAINTENANCE_DATABASE_PATH=/srv/app/data/codex.maintenance.sqlite
PERFORMANCE_DATABASE_PATH=/srv/app/data/codex.performance.sqlite
```

The leading dot in `.maintenance.sqlite` is a suffix separator after the main filename stem; the resulting file does not begin with a dot and is not hidden. An explicit maintenance path must be different from both the main database path and the performance database path.

## Management Page Contract

Every Managed Task page uses the same structure: task identity and observation freshness, enablement and safe pause controls, one-off execution, trigger configuration, metric cards for total/completed/progress/ETA/phase, a checkpoint summary, recent runs, and performance metrics read from the performance database. Manual maintenance pages omit interval and cron controls while retaining the enable and one-off execution controls.

The page reuses the existing system administration permission model. Read-only users may inspect task state; only system administrators may change controls, schedules, or invoke a task. No task-specific role is introduced in the first phase.

Interval and cron values are validated before persistence. Cron uses UTC and shows the next calculated occurrence. Event-driven tasks keep their event wake path; interval or cron is only a bounded fallback probe. The scheduler rejects configurations that would create an unsafe high-frequency load.

## Consequences

The management pages add no synchronous operational-write pressure to the business database, and performance queries cannot accidentally become a source of task-control truth. Operators must tolerate an observability gap when the maintenance database is unavailable, and migration must preserve enough compatibility to avoid losing existing run or backfill visibility. The new database needs its own lifecycle, retention, backup, and corruption handling policy.
