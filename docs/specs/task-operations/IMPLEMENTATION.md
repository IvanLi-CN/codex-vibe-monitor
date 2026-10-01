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

## Rollout Facts

- No persistent schema migration is introduced. Existing maintenance database rows and custom overrides are read as-is.
- Computed policy fields are response metadata; reads do not rewrite `interval_secs`, `cron_expr`, or `next_trigger_at`.
- Unsupported existing overrides remain visible and can be cleared explicitly. Startup-only actions are not replayed by reset; their default behavior applies on the next process start.

## Validation Notes

- The shared testbox backend lightweight, stateful-SQLite, and archive-file-io profiles passed for the candidate. Its web lane could not run because Bun is not installed there.
- An isolated local production router verified health, a 37-row catalog, the runtime endpoint, supported schedule PATCH/reset behavior, and rejection of a new unsupported override. The controlled task completed before the two-second polling interval, so its active snapshot was intentionally observed through history rather than fabricated in the UI.
- The mock-only Web Demo supplied the desktop and mobile evidence recorded in `SPEC.md`; both screenshots were confirmed by the owner and are committed under `./assets/`.
- The runtime route and PATCH tri-state have focused unit and local HTTP coverage; no persistent schema migration or default-value backfill is required.

## Related Changes

- `docs/adr/0024-task-runtime-observation-and-effective-schedules.md`
- `docs/solutions/maintenance/task-schedule-and-running-observation.md`

## References

- `./SPEC.md`
- `./HISTORY.md`
