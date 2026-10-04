# 任务运维运行观测与生效计划 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: requirements `REQ-TASK-OPS-001..014` are implemented, including durable run workload samples, bounded detail refresh, recent-run charts, Retention backlog Tabs, and conditional estimates. The current correction restores the shared segmented Tabs, labels the existing views “次数 / 时间”, and removes the unrequested run-hour filtering mode. The owner confirmed all six rectification screenshots on 2026-10-04; the accepted Storybook component evidence is persisted in `SPEC.md` and `assets/`.
- Lifecycle: active
- Catalog note: Runtime snapshots remain process-local, while execution identity and historical intervals are persisted in the maintenance SQLite database and merged by execution UID when both sources overlap.
- Requirements coverage: `REQ-TASK-OPS-010..014` are implemented. Backend validation remains applicable to unchanged Rust sources; the current correction passed shared-testbox Web and Storybook validation. Earlier timeline evidence does not validate the task-detail charts.

## Implementation Coverage

- `REQ-TASK-OPS-001`: `src/task_runtime_observation.rs`, `src/api/slices/system_routes_and_tasks.rs`, `src/runtime.rs`, and the startup-backfill work boundary.
- `REQ-TASK-OPS-002`: `src/maintenance_store.rs` decorates the 21 root tasks and 16 startup-backfill children without persisting computed defaults; `web/src/lib/api/core-foundation.ts` carries the additive response fields.
- `REQ-TASK-OPS-003`: `OptionalField` deserialization in `src/api/slices/system_routes_and_tasks.rs` preserves PATCH omission/null/value; `MaintenanceStore::update_control` applies the six-task allowlist and reset behavior.
- `REQ-TASK-OPS-004`: `web/src/pages/system/SystemTasksPage.tsx`, `SystemTaskDetailPage.tsx`, demo handlers, unit coverage, and the system workspace Storybook state provide the running section, filters, schedule policy display, and responsive catalog.
- `REQ-TASK-OPS-005`: `src/maintenance_store.rs` reads actual FIFO `requested` rows and active worker admission deferrals; `src/api/slices/system_routes_and_tasks.rs` reports each source's availability separately. Normal future schedules are excluded.
- `REQ-TASK-OPS-006`: task light/dark colors are persisted in the managed-task metadata, assigned only when absent, and shared by catalog dots, current work, and timeline segments.
- `REQ-TASK-OPS-007`: `src/task_timeline.rs` records actual start/end/duration independently of legacy request-time fields; runtime snapshots and revision-based timeline deltas are delivered through `system.managed-tasks.runtime` and `system.managed-tasks.timeline` SSE topics. The page does not poll either HTTP endpoint. A shared one-second clock advances the 12-hour view and open durations between events; connection loss is visible and stops runtime extrapolation after the bounded grace period. Overlap packing and dense short-run aggregation are rendered by `web/src/features/system/TaskTimelineChart.tsx`. The API and maintenance history retain their longer read/retention range.
- `REQ-TASK-OPS-008`: scheduler boundaries record resource-busy and pressure-cooldown deferrals, which share the execution time axis in one pressure row; overlapping causes remain individually inspectable, retention pressure source aliases map to the managed retention task, and explicit coverage and maintenance-write gaps remain unknown rather than healthy.
- `REQ-TASK-OPS-009`: a bounded nonblocking event channel persists executions, deferrals, coverage sessions, and revisions to maintenance storage; channel overflow is localized as an explicit interval, shutdown flushes retry a bounded number of times and leave coverage open after persistent failure, startup closes unconfirmed coverage at the last confirmed boundary, retention preserves a 48-hour buffer, and the window endpoint uses fixed-watermark pages plus revision deltas.
- Startup-backfill control updates compensate a two-store failure by restoring the maintenance control row when the progress store rejects an enablement change. Opening an older row that contains both interval and cron values keeps cron authoritative and recomputes its persisted next trigger.

## Task Detail Metrics

- `src/maintenance_store.rs` defines per-task measurement capabilities and the additive `workloadTrend` detail response. It merges persisted samples with confirmed legacy managed attempts, deduplicates by execution UID and task key, fetches the persisted running attempt separately from the latest completed history, reserves response capacity for it and the in-memory running snapshot, returns at most 100 attempts in run order, and leaves pre-upgrade metric values unknown.
- `managed_task_work_runs` stores one monotonic sample per `(execution_uid, task_key)` in the maintenance database. Recorder notifications reuse the bounded asynchronous timeline queue; terminal samples force a final snapshot. Startup marks unconfirmed running samples unknown and rebuilds malformed sample JSON as an identity-preserving unknown record, sequence checks reject stale updates, recorder loss is represented as a coverage gap, and cleanup preserves the latest 100 samples per task plus every running sample.
- Retention records an exact eligible invocation-row snapshot at run start, distinct eligible candidates, and rows only after their archive transaction commits. Other roots and the 16 startup-backfill children expose only task-specific units with proven semantics; truncated scan counts and mixed `processedCount` values remain unsupported.
- Committed-work notifications are scoped to the managed task identity. Compression records completed files, manifest refresh records completed archive batches, rollup materialization records batches after transaction commit, and archive pruning records each finalized deletion. Later failures therefore retain successful work without mixing units between nested maintenance operations.
- `system.managed-tasks.detail?taskKey=...` publishes the same detail shape using persisted workload revision changes. Detail pages consume this topic and the existing runtime topic; the two-second runtime HTTP poll is removed while local elapsed-time animation remains.
- `web/src/features/system/TaskWorkloadTrend.tsx` renders raw P/D/C as overlapping zero-baseline areas without explanatory implementation prose, keeps all three legend entries, and preserves empty/loading/error/legacy chart frames. A running attempt without plotted counters gets an inspectable baseline marker. Point details show proven partition values. The shared segmented control remains on one row: “次数” selects recent attempts and Retention's “时间” selects independent seven-day count and overdue-hour charts. The panel reserves the largest responsive layout while mounting chart canvases only for the selected Tab. Mobile evidence stories bind their native 393×852 viewport through Storybook globals.
- Summary fields expose only exact pending snapshots, the latest measured processing sample, complete-attempt processing rates, and fresh same-range backlog estimates with the documented coverage thresholds. A zero snapshot is called cleared only when fresh and exact; stale, disabled, and gapped zero snapshots retain their reason. Overall percentage remains hidden unless a fixed cohort and unique cumulative completion are provable.

## Current Candidate Verification

- Against comparison base `a8532fc673b65b79fe486666f26d2d5efe088a1d`, the current candidate passed all shared-testbox backend profiles: `lightweight` 1,310, `stateful-sqlite` 1,400, and `archive-file-io` 307 tests. The same run passed `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and `cargo clippy --locked --all-targets --all-features -- -D warnings`.
- Targeted regressions passed for parent/child notification-throttle cleanup, unknown metric coverage, atomic legacy schedule repair, ETA overflow and date-boundary rejection, immediate recovery of every running row at startup, full rollback of a failed legacy migration, the `run-window` range on committed child counts, managed-run identity on system-task observations, identity-preserving recovery of malformed terminal samples, and retention of recent workload sample identities. Existing workload tests cover persistence without detail subscribers, restart recovery, monotonic sample ordering, legacy unknowns, and queue-overflow gaps.
- Tier 3 review findings were fixed in batches: terminal samples release notification throttles; supported metrics with missing values report unknown coverage; legacy schedule repair and history import are transactional; ETA arithmetic rejects unrepresentable results; selected run windows include only overlapping coverage gaps; child processing counts carry a matching range; startup immediately retires every prior-process running row; stale, disabled, and gapped zero values are not labeled cleared; forward-proxy attempts retain their managed-run identity; malformed persisted samples remain visible with unknown metrics; selected-window legend availability follows the displayed window; skipped samples have touch and keyboard inspection; and narrow chart height matches the approved 280px layout.
- Current shared-testbox Web unit tests passed (1,788 passed, 6 skipped); the focused `TaskWorkloadTrend` and `SystemWorkspace` Storybook suites passed 79 tests, including single-row “次数 / 时间” Tabs, keyboard and touch inspection, hidden-series scaling, counterless and skipped runs, stale-zero and disabled-zero summaries, Retention tab height, selected-window legend availability, and gaps outside the selected run window. Web typecheck, lint, and production build passed on the same testbox. Lint reported 96 repository warnings; build reported stale Browserslist data and large-chunk warnings. Logs are retained under the task's `web-rectification/logs/` directory.
- Earlier `SPEC.md` contract validation and Spec drift checks against the comparison base passed. The current six desktop/mobile Storybook component images were captured at `d981c0b1` and confirmed in chat; the canonical paths now contain these accepted images. The later direct Chromium captures and local full-suite runs are excluded from current compliant evidence. Current captures use Ego Browser and source-bound Storybook viewports, and heavy verification runs on the shared testbox.
- PR CI exposed missing source-quality coverage and an outdated Demo E2E contract. The follow-up extracts Retention workload reporting, coverage repair, and archive rollup estimation into focused modules without raising file budgets or registering new lint suppressions. Workload fixtures group execution boundaries; the synchronous scoped-observation test drives its task-local future through a current-thread runtime. The shared testbox passed all eight production Demo E2E cases with the approved charts and Tabs, and the unchanged Rust source-quality policy passed. Logs are retained in the task's `ci-repair/logs/` directory. The testbox host uses Rust 1.94.1; current-head CI supplies the repository's Rust 1.96.0 gate. Frontend render inputs remain identical to the accepted six-image evidence.
- A controlled live run on pre-sync feature candidate `c11fc819` streamed a task-scoped detail event and retained the same two workload identities and values after restart. With the maintenance database held under a 40-second write lock, 23 samples (8 success, 15 confirmed skip) were generated and persisted with their original attempt times after the lock released; `/health` stayed `200` in 21 ms, and the business database had no workload table. A new manual run request correctly returned `409` because its scheduler control record could not be persisted, so no unaccepted run was dispatched. This is historical empirical evidence; post-sync automated profile and regression results above are current-candidate evidence. The scenario does not represent a production workload or promise ETA accuracy.
- Remaining delivery gates: four fresh Tier 3 read-only review lanes bound to the final candidate, live PR checks, and the Fast Flow merge-ready handoff. No merge or release has been performed.

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
- `cargo test maintenance_store::tests::recovers_all_incomplete_runs_on_restart_idempotently -- --nocapture`
- `cargo test maintenance_store::tests::legacy_state_migration_rolls_back_every_write_on_failure -- --nocapture`
- `cargo test task_runtime_observation::tests::direct_managed_backfill_does_not_duplicate_its_workload_sample_as_a_child -- --nocapture`
- `cargo test startup_backfill_tests -- --nocapture`
- `cargo test scoped_committed_work_is_limited_to_its_managed_task -- --nocapture`
- `cargo test estimates_only_from_fresh_complete_same_range_backlog_samples -- --nocapture`
- `cargo test task_runtime_observation::tests::subset_relation_requires_compatible_units_and_nested_counts -- --nocapture`
- `cargo test confirmed_skips_count_as_zero_without_fabricating_a_sample_metric -- --nocapture`
- `cargo test workload_migration_is_repeatable_and_protects_recent_and_running_samples -- --nocapture`
- `cargo test workload_store_ignores_out_of_order_sample_sequences -- --nocapture`
- `cargo test legacy_run_metrics_remain_unknown_and_associated_samples_deduplicate -- --nocapture`
- `cd web && bun run test-storybook -- TaskWorkloadTrend.stories.tsx SystemWorkspace.stories.tsx`

## Rollout Facts

- The timeline adds idempotent columns and tables to the maintenance SQLite database. Existing `started_at` and `duration_ms` fields keep their request-time meanings; old rows do not receive reconstructed execution intervals. Existing enablement and schedule overrides are preserved.
- Computed policy fields are response metadata; reads do not rewrite `interval_secs`, `cron_expr`, or `next_trigger_at`.
- Unsupported existing overrides remain visible and can be cleared explicitly. Startup-only actions are not replayed by reset; their default behavior applies on the next process start.
- Observation writes use a bounded asynchronous channel and never write to the business database. Saturation or maintenance-store failures surface as coverage gaps.
- Workload samples are stored in a separate additive maintenance table; old task history is read only as attempt identity and never receives inferred P/D/C values. The task detail endpoint does not issue a business-table count or file walk to build a chart.

## Historical Verification

The evidence below records earlier implementation candidates and is not current-candidate proof. The active delivery flow owns current validation and empirical acceptance evidence.

- The shared testbox `lightweight`, `stateful-sqlite`, and `archive-file-io` profiles passed with 1,265, 1,377, and 299 tests respectively. `cargo fmt --all -- --check`, locked all-target/all-feature `cargo check`, and locked all-target/all-feature Clippy with warnings denied passed.
- Web unit tests passed (1,691 passed, 6 skipped); the focused timeline suite passed all 6 tests and the `SystemWorkspace` Storybook suite passed all 36 interactions. `bun run typecheck:web`, `bun run lint:web`, and `bun run build` passed. Lint reported 92 existing warnings; build reported stale Browserslist data and large chunks.
- An isolated production binary ran against three fresh SQLite files. HTTP snapshots showed six simultaneous FIFO requests with positions 1–6, distinct from active admission deferrals; the waits later released. Eight manual execution intervals (including two separate runs of the same task) persisted as eight unique successful IDs with measured actual durations. The service ran and recorded intervals while no page was open.
- After graceful shutdown and restart against the same maintenance database, both coverage sessions retained explicit start/end boundaries, the downtime remained outside observed coverage, and both sessions reported zero dropped events. The eight execution IDs remained unique, history was readable over HTTP, and the runtime did not resurrect an old execution as active. A separate cloned maintenance database with 510 pagination fixtures returned 500 rows plus 169 rows over HTTP; both pages shared watermark 78 with no duplicate IDs, and `afterRevision=78` later returned 16 new segments at watermark 86. The persisted catalog exposed 37 distinct light/dark task color pairs.
- Owner-confirmed mock-only evidence now covers the desktop dark disconnected state, desktop light connecting state, and mobile light 12-hour chart without row labels. The desktop and mobile comparisons use the exact assets in the rebased `origin/main` commit `7037e63e` and were reviewed with their heatmaps before confirmation.

## Earlier Candidate Verification

- Candidate `1ae7f784164f59878d4afae6c3d4bfc923634498` passed `cargo fmt --all -- --check`, the full Web unit suite (1,711 passed, 6 skipped), the focused `SystemWorkspace` Storybook suite (37 passed), Web typecheck, lint, and build. Lint reported 92 existing warnings; build reported stale Browserslist data and a large-chunk warning.
- Targeted Rust regressions passed: managed-task execution identity (3), timeline/SSE/persistence tests (12), managed-task contract tests (7), and fresh recorder coverage for admission waits (1).
- Shared testbox profiles, all-target/all-feature Rust check and Clippy, and isolated production-service acceptance have not been rerun after syncing `origin/main`; the testbox currently responds to ping but its SSH service times out during banner exchange. Earlier candidate evidence is not used as current-candidate proof.
- Tier 3 formal review lanes and PR CI had not started on that earlier candidate.

## Related Changes

- `docs/adr/0024-task-runtime-observation-and-effective-schedules.md`
- `docs/adr/0026-durable-task-execution-and-deferral-timelines.md`
- `docs/solutions/maintenance/task-schedule-and-running-observation.md`

## References

- `./SPEC.md`
- `./HISTORY.md`
