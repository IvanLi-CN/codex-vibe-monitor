# 任务运维运行观测与生效计划 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: complete for the accepted scope
- Lifecycle: active
- Catalog note: Runtime snapshots are process-local; task history remains in the maintenance SQLite database.

## Implementation Coverage

- `REQ-TASK-OPS-001`: `src/task_runtime_observation.rs`, `src/api/slices/system_routes_and_tasks.rs`, `src/runtime.rs`, and the startup-backfill work boundary.
- `REQ-TASK-OPS-002`: `src/maintenance_store.rs` decorates the 21 root tasks and 16 startup-backfill children without persisting computed defaults; `web/src/lib/api/core-foundation.ts` carries the additive response fields.
- `REQ-TASK-OPS-003`: `OptionalField` deserialization in `src/api/slices/system_routes_and_tasks.rs` preserves PATCH omission/null/value; `MaintenanceStore::update_control` applies the six-task allowlist and reset behavior.
- `REQ-TASK-OPS-004`: `web/src/pages/system/SystemTasksPage.tsx`, `SystemTaskDetailPage.tsx`, demo handlers, unit coverage, and the system workspace Storybook state provide the running section, filters, schedule policy display, and responsive catalog.
- Startup-backfill control updates compensate a two-store failure by restoring the maintenance control row when the progress store rejects an enablement change. Opening an older row that contains both interval and cron values keeps cron authoritative and recomputes its persisted next trigger.

## Verification Commands

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo test task_runtime_observation::tests -- --nocapture`
- `cargo test maintenance_store::tests::decorates_root_and_backfill_tasks_with_effective_policy_and_capability -- --nocapture`
- `cargo test maintenance_store::tests::preserves_enabled_when_clearing_unsupported_override_and_rejects_new_one -- --nocapture`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cd web && bun run test -- src/pages/system/SystemTasksPage.test.tsx`
- `cd web && bun run test`
- `cd web && bun run test-storybook -- SystemWorkspace`
- `cd web && bunx tsc -b`
- `cd web && bun run build`
- `bunx biome check web`
- `cargo test maintenance_store::tests::repairs_legacy_dual_schedule_trigger_on_schema_open -- --nocapture`
- `cargo test startup_backfill_tests -- --nocapture`

## Rollout Facts

- No persistent schema migration is introduced. Existing maintenance database rows and custom overrides are read as-is.
- Computed policy fields are response metadata; reads do not rewrite `interval_secs`, `cron_expr`, or `next_trigger_at`.
- Unsupported existing overrides remain visible and can be cleared explicitly. Startup-only actions are not replayed by reset; their default behavior applies on the next process start.

## Validation Notes

- The shared testbox backend lightweight, stateful-SQLite, and archive-file-io profiles passed for the candidate. Its web lane could not run because Bun is not installed there.
- Candidate `1955d173e3ea8f86b640ea71a6fd464e99c11d19` was checked with an isolated local production router on a leased port and fresh SQLite files: health returned `ok`, the catalog returned 37 rows, the runtime endpoint returned a real `raw_payload_metrics_inventory` active instance with execution id, phase, start time, and elapsed milliseconds, a managed dashboard run completed successfully, the approved dashboard interval override and dual-null reset returned HTTP 200, and an unsupported new override returned HTTP 400. The reset response preserved `enabled` and reported the system-default policy.
- The mock-only Web Demo supplied the desktop and mobile evidence recorded in `SPEC.md`; both screenshots were confirmed by the owner and are committed under `./assets/`.
- The runtime route and PATCH tri-state have focused unit and local HTTP coverage; no persistent schema migration or default-value backfill is required.

## Related Changes

- `docs/adr/0024-task-runtime-observation-and-effective-schedules.md`
- `docs/solutions/maintenance/task-schedule-and-running-observation.md`

## References

- `./SPEC.md`
- `./HISTORY.md`
