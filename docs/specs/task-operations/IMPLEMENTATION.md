# 任务运维运行观测与生效计划 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: current execution observation, effective schedules, separate dispatcher/admission waits, stable task colors, durable execution timelines, and task-deferral intervals are implemented; visual evidence is confirmed and persisted, while delivery remains subject to current-candidate backend profiles, empirical service acceptance, Tier 3 review, and CI.
- Lifecycle: active
- Catalog note: Runtime snapshots remain process-local, while execution identity and historical intervals are persisted in the maintenance SQLite database and merged by execution UID when both sources overlap.

## Implementation Coverage

- `REQ-TASK-OPS-001`: `src/task_runtime_observation.rs`, `src/api/slices/system_routes_and_tasks.rs`, `src/runtime.rs`, and the startup-backfill work boundary.
- `REQ-TASK-OPS-002`: `src/maintenance_store.rs` decorates the 21 root tasks and 16 startup-backfill children without persisting computed defaults; `web/src/lib/api/core-foundation.ts` carries the additive response fields.
- `REQ-TASK-OPS-003`: `OptionalField` deserialization in `src/api/slices/system_routes_and_tasks.rs` preserves PATCH omission/null/value; `MaintenanceStore::update_control` applies the six-task allowlist and reset behavior.
- `REQ-TASK-OPS-004`: `web/src/pages/system/SystemTasksPage.tsx`, `SystemTaskDetailPage.tsx`, demo handlers, unit coverage, and the system workspace Storybook state provide the running section, filters, schedule policy display, and responsive catalog.
- `REQ-TASK-OPS-005`: `src/maintenance_store.rs` reads actual FIFO `requested` rows and active worker admission deferrals; `src/api/slices/system_routes_and_tasks.rs` reports each source's availability separately. Normal future schedules are excluded.
- `REQ-TASK-OPS-006`: task light/dark colors are persisted in the managed-task metadata, assigned only when absent, and shared by catalog dots, current work, and timeline segments.
- `REQ-TASK-OPS-007`: `src/task_timeline.rs` records actual start/end/duration independently of legacy request-time fields; runtime snapshots and revision-based timeline deltas are delivered through `system.managed-tasks.runtime` and `system.managed-tasks.timeline` SSE topics. The page does not poll either HTTP endpoint. A shared one-second clock advances the 12-hour view and open durations between events; connection loss is visible and stops runtime extrapolation after the bounded grace period. Overlap packing and dense short-run aggregation are rendered by `web/src/features/system/TaskTimelineChart.tsx`. The API and maintenance history retain their longer read/retention range.
- `REQ-TASK-OPS-008`: scheduler boundaries record resource-busy and pressure-cooldown deferrals, which share the execution time axis; explicit coverage and maintenance-write gaps remain unknown rather than healthy.
- `REQ-TASK-OPS-009`: a bounded nonblocking event channel persists executions, deferrals, coverage sessions, and revisions to maintenance storage; startup closes unconfirmed open segments as unknown, retention preserves a 48-hour buffer, and the window endpoint uses fixed-watermark pages plus revision deltas.
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

- The timeline adds idempotent columns and tables to the maintenance SQLite database. Existing `started_at` and `duration_ms` fields keep their request-time meanings; old rows do not receive reconstructed execution intervals. Existing enablement and schedule overrides are preserved.
- Computed policy fields are response metadata; reads do not rewrite `interval_secs`, `cron_expr`, or `next_trigger_at`.
- Unsupported existing overrides remain visible and can be cleared explicitly. Startup-only actions are not replayed by reset; their default behavior applies on the next process start.
- Observation writes use a bounded asynchronous channel and never write to the business database. Saturation or maintenance-store failures surface as coverage gaps.

## Validation Notes

- The shared testbox `lightweight`, `stateful-sqlite`, and `archive-file-io` profiles passed with 1,265, 1,377, and 299 tests respectively. `cargo fmt --all -- --check`, locked all-target/all-feature `cargo check`, and locked all-target/all-feature Clippy with warnings denied passed.
- Web unit tests passed (1,691 passed, 6 skipped); the focused timeline suite passed all 6 tests and the `SystemWorkspace` Storybook suite passed all 36 interactions. `bun run typecheck:web`, `bun run lint:web`, and `bun run build` passed. Lint reported 92 existing warnings; build reported stale Browserslist data and large chunks.
- An isolated production binary ran against three fresh SQLite files. HTTP snapshots showed six simultaneous FIFO requests with positions 1–6, distinct from active admission deferrals; the waits later released. Eight manual execution intervals (including two separate runs of the same task) persisted as eight unique successful IDs with measured actual durations. The service ran and recorded intervals while no page was open.
- After graceful shutdown and restart against the same maintenance database, both coverage sessions retained explicit start/end boundaries, the downtime remained outside observed coverage, and both sessions reported zero dropped events. The eight execution IDs remained unique, history was readable over HTTP, and the runtime did not resurrect an old execution as active. A separate cloned maintenance database with 510 pagination fixtures returned 500 rows plus 169 rows over HTTP; both pages shared watermark 78 with no duplicate IDs, and `afterRevision=78` later returned 16 new segments at watermark 86. The persisted catalog exposed 37 distinct light/dark task color pairs.
- Owner-confirmed mock-only evidence now covers the desktop dark disconnected state, desktop light connecting state, and mobile light 12-hour chart without row labels. The desktop and mobile comparisons use the exact assets in the rebased `origin/main` commit `7037e63e` and were reviewed with their heatmaps before confirmation.

## Current Candidate Verification

- Candidate `1ae7f784164f59878d4afae6c3d4bfc923634498` passed `cargo fmt --all -- --check`, the full Web unit suite (1,711 passed, 6 skipped), the focused `SystemWorkspace` Storybook suite (37 passed), Web typecheck, lint, and build. Lint reported 92 existing warnings; build reported stale Browserslist data and a large-chunk warning.
- Targeted Rust regressions passed: managed-task execution identity (3), timeline/SSE/persistence tests (12), managed-task contract tests (7), and fresh recorder coverage for admission waits (1).
- Shared testbox profiles, all-target/all-feature Rust check and Clippy, and isolated production-service acceptance have not been rerun after syncing `origin/main`; the testbox currently responds to ping but its SSH service times out during banner exchange. Earlier candidate evidence is not used as current-candidate proof.
- Tier 3 formal review lanes and PR CI have not started because current-candidate review readiness is not yet satisfied.

## Related Changes

- `docs/adr/0024-task-runtime-observation-and-effective-schedules.md`
- `docs/adr/0026-durable-task-execution-and-deferral-timelines.md`
- `docs/solutions/maintenance/task-schedule-and-running-observation.md`

## References

- `./SPEC.md`
- `./HISTORY.md`
