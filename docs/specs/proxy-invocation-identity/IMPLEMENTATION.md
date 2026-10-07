# Proxy Invocation Identity Implementation

> The current effective contract remains in `./SPEC.md`; this file records implementation coverage and verification facts.

## Current Status

- Implementation: Materialization, long-wait isolation, allocation-range correction, and PR2 public API/frontend integration implemented
- Lifecycle: active
- Catalog note: Durable backend identity, allocation, persistence, and retention contract.

## Implementation Coverage

主线 range reservation 的服务验收已转移到 `shared-testbox-proxy-runtime-acceptance`
及 `proxy-long-wait-loadgen.py`：保留预热、100 ms cold refusal 可见性、range probes、
crash recovery 和 older-reader reservation 比较；不读取退役性能库。终态入队同时
保留观测开始时间与 pending identity registry，业务身份释放顺序保持主线合同。

The hosted CPU diagnosis reuses the same synthetic proxy request setup and immutable
candidate image. Invocation identity, retries, terminal deduplication and persisted
business fields remain unchanged; diagnostic counters omit labels and payloads.
Current candidate quality and performance evidence must be refreshed after mainline sync.

- The service-process priority fixture refreshes cached System Status pressure through the existing managed-task API inside its original 60-second probe budget. It records snapshot timestamps, never admits a stale idle snapshot, and retains the same measured latency, progress, and convergence gates.
- `REQ-PII-001`: `src/prompt_cache_conversations.rs` owns the six-character conversation prefix, four-character base-31 sequence, hourly unbound prefix, overflow handling, and cache recovery; HTTP capture uses the allocator, while retained WebSocket invocation IDs remain readable as historical data.
- `REQ-PII-002`: `prompt_cache_conversations` stores identity and delayed aggregate statistics; `src/schema.rs` installs only additive structure, while the ordered `prompt_cache_conversations_materialization_v1` startup task performs historical materialization in the background. Each 400-key logical page uses an in-memory adaptive 64..400 micro-batch controller with a 32-key floor. The additive `idx_pool_attempts_account_model_success` partial covering index supports the model-health latest-success check without a temporary sort and is safe to create repeatedly during startup. Statistics pages use the existing prompt-cache key/occurred-at expression index and the SQLite rowid tie-breaker without a temporary sort.
- `REQ-PII-003`: `AppState` carries a shared `InvocationRangeManager`. HTTP entrypoints use committed memory ranges for hot issuance. Normalized prompt-cache keys retain cancellation-safe active references, and concurrent entrypoints share one owner initialization/refill without stacking serial wait budgets. Only cold initialization, shared refill, and bounded tail return acquire SQLite write admission; no global cache lock spans those operations. Atomic slot admission protects waiting allocations and remains bounded at 4096 conversations.
- `REQ-PII-004`: `src/sqlite_batch_writer.rs` and prompt-cache key backfill enqueue touched keys without synchronously scanning retained invocations; the startup materialization task drains the durable refresh queue. Queue events preempt ordinary idle or continuation deadlines and coalesce through the scheduler's per-task wake set, while active pressure defers retain their retry deadline. Final prompt-cache checkpoint writes compare the run-start wake generation so an event that arrives during processing cannot be overwritten by a late ordinary deadline. Identity discovery commits adaptive identity batches and their cursor together. During `stats_rebuild`, each conversation's final source page atomically publishes its aggregate, deletes only the matching queue generation/staging row, and advances the continuous key cursor in one business transaction. A partial source page commits staging alone, so completed prefixes survive a later key's pause or budget boundary. `queue_drain` uses queue deletion rather than the rebuild cursor. Unchanged-source pages continue within the existing three-second run and two-second query budgets, with one visited-key allowance for all pages of that key and a 10 ms scheduler window between page commits. Actual scanned keys and published rows drive run counts, and ETA is nullable until statistics are complete. `src/maintenance/retention.rs` deletes archived invocations before refreshing affected aggregates, releases orphan masters, and clears released identities from memory.
- `REQ-PII-005`: Allocation, migration, recovery, statistics, retention, collision, and exhaustion paths emit structured diagnostic events using prompt-key fingerprints.
- `REQ-PII-008`: Aggregate publication no longer updates `last_invoke_sequence`. The unified range manager initializes 64-sequence ranges, combines eligible conversation/hour owners into one commit, and publishes at most one standby per owner after commit. Idle retirement conditionally returns only the unissued tail within 100 ms, keeping the generation fenced until an owned SQLite connection has drained an uncertain operation.
- `REQ-PII-009`: Capacity starts at 128, is seeded once asynchronously by a covering-index read capped at 4096 rows, and is recalculated hourly only from an independent bounded memory activity index. Live and seed timestamps merge by maximum; eviction keeps activity history. The captured UTC cutoff is serialized in the master's existing Shanghai-local timestamp representation for the indexed read. A single retirement worker bounds tail-return pressure; an unrelated cold owner reserves an available slot below the 4096 maximum while retirement catches up. Saturation and retiring-owner admission have one 100 ms deadline.
- `REQ-PII-010`: Schema setup atomically installs `hourly_invoke_prefixes` and the immutable `invocation_range_ownership_v1` marker. Hourly initialization recovers the same prefix and skips committed reservations across restart. HTTP cancellation guards and the terminal journal's recovered/pending identity registry protect active and pending owners. Retention fences a cache generation before deleting eligible conversation/hourly rows; uncertain SQL drains on an owned connection before the fence is released.
- Terminal reconciliation removes the allocator's active identity before releasing journal pending protection. HTTP guard callbacks carry a scoped namespace reference until the callback drains, so legitimate prefix reuse cannot race a late callback from the old lifetime. Ended-hour cleanup scans bounded primary-key pages with a memory cursor, allowing later eligible rows to progress past protected older hours.
- Upstream URL validation failures issue an unleased ID because they return without a terminal persistence record. They preserve the existing error response and sequence advancement while allowing the ended hourly owner to be released; calls that enter persistence retain their active identity protection.
- Request cleanup guards, orphan persistence reconciliation and terminal journal identity tracking live in focused `invocation_identity` child modules. The existing parent modules retain their call contracts and remain within the repository's explicit source-line budgets. The retired WebSocket entrypoint and its allocation helper stay removed; retained WebSocket identities remain readable through historical records.
- Retirement completion releases the cache mutex before rescheduling deferred shrink. Multiple owners that become idle during one return operation therefore retire sequentially toward the calculated target, retaining the existing single-worker and bounded-tail-return policy.
- `REQ-PII-007`: The live working-set update trigger in `src/schema/prompt_cache_working_set_triggers.rs` declares the source columns used by key, scope, status, timestamp and aggregate projection. The terminal writer's `t_persist_ms` follow-up no longer runs two redundant same-key scans. Relevant updates retain the original old/new-key refresh bodies and live-window guard. Startup atomically replaces the existing three working-set triggers and records `prompt_cache_working_set_relevant_updates_v1` in the existing `schema_refresh_migrations` table; an already initialized database needs no projection-row rebuild for this upgrade.
- Long-wait acceptance coverage: `scripts/shared-testbox-proxy-runtime-acceptance --scenario long-wait --duration 600 --rounds 3 --rate 4` drives eight 179-second/180-second-boundary proxy calls, two fast workers, same-key/different-key/unbound allocation, and cancellation while recording allocation, request, terminal confirmation, status, and incomplete-work samples. The business fixture retains its timing and assertions after extraction from the retired telemetry runner; it does not query the removed performance API or database.
- Operational follow-up: `GET/PATCH /api/system/prompt-cache/materialization` and managed-task control use the same committed `managed_tasks.enabled` value in the maintenance database. The matching `startup_backfill_progress` checkpoint is updated in that maintenance transaction. Both PATCH handlers use the existing 503 Unavailable response when the control transaction fails. A one-time origin marker preserves an existing maintenance control; only a newly created managed row may be seeded from the old business-database enable bit. The business bit is not read by the run path or written by this task. An in-memory generation gate lets an already registered short SQL step finish while preventing later steps from starting under a stale generation. The existing status response still exposes durable progress, estimated remaining time, recent bounded run history, and the pause/resume control.

## Verification

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo test prompt_cache_conversation -- --nocapture`
- `cargo test prompt_cache_conversation_allocator_releases_lease_after_cancellation -- --nocapture`
- `cargo test ensure_schema_adds_idempotent_success_attempt_route_lookup_index -- --nocapture`
- `cargo test prompt_cache_materialization_yields_only_after_a_committed_micro_batch -- --nocapture`
- `cargo test prompt_cache_materialization_large_key_is_paged_once_across_phases -- --nocapture`
- `cargo test prompt_cache_materialization_pages_resume_across_generation_change_and_control_restart -- --nocapture`
- `cargo test prompt_cache_materialization_uses_maintenance_control_when_legacy_business_flag_disagrees -- --nocapture`
- `cargo test prompt_cache_materialization_wake_preserves_operator_disable -- --nocapture`
- `cargo test prompt_cache_materialization_failure_history_reports_per_run_work -- --nocapture`
- `cargo test existing_maintenance_prompt_cache_control_and_checkpoint_win_over_legacy_state -- --nocapture`
- `cargo test legacy_only_prompt_cache_control_is_imported_once_into_maintenance -- --nocapture`
- `cargo test prompt_cache_control -- --nocapture`
- `cargo test prompt_working_set_ -- --nocapture`
- `cargo test prompt_cache_stale_checkpoint_cannot_overwrite_pause_or_resume -- --nocapture`
- `cargo test prompt_cache_control_commit_waits_for_admitted_checkpoint_finalization -- --nocapture`
- `cargo test stale_prompt_cache_disable_schedule_cannot_erase_a_resume_wake -- --nocapture`
- `cargo test prompt_cache_materialization_fixed_400_vs_adaptive_representative_scale -- --ignored --nocapture --test-threads=1`
- `bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite`
- Focused stateful SQLite tests in `src/tests/stateful_sqlite/proxy_long_wait_allocation_and_index.rs`, `src/tests/stateful_sqlite/prompt_cache_materialization_control.rs`, and `src/tests/stateful_sqlite/prompt_cache_working_set_trigger.rs`. The working-set tests invoke the actual terminal persistence core with a redundant-update sentinel, compare business mutations against a source rebuild, and interrupt completion-marker insertion to verify transactional DDL rollback and startup reentry without touching live rows.
- The long-wait workload is a required Linux shared-testbox acceptance run; its candidate SHA, run directory, and per-round gate results are recorded with the delivery evidence.
- `scripts/shared-testbox-proxy-runtime-acceptance --scenario prompt-cache-control --duration 300 --rounds 3 --rate 4` owns the service-process task-control replay. Its pressure-log mount and all progress, permission, timing, and completion assertions are preserved.
- The `prompt-cache-control` service-process fixture records every online call, persistent identity and staging checkpoints, pause/resume generations, maintenance file-lock responses, and restart completion. Before steady input, a bounded synthetic business-file lock keeps an admitted SQL step open while an interactive proxy writer queues; the fixture requires a fresh service-process `coordinator_priority` result, including a zero-work yield that correctly adds no run-history record instead of depending on incidental contention. These probe calls join the steady calls in allocation and terminal p99 checks. Its 30-second progress clock runs only while the task is enabled and eligible: a future continuation deadline or a fresh service-process pressure-denial deadline is excluded. Historical `coordinator_priority` run records never exempt later eligible work. The Linux runner streams the current service log into a read-only fixture mount and restarts that log reader with the service; missing pressure evidence fails validation. The independent 180-second post-input completion bound still applies to the entire observation window.
- Baseline `prompt-cache-control` replay at `b68295595d875aead33ef31514eaf602279ece94`: the isolated service accepted a managed run with maintenance control enabled and the legacy business bit disabled, but after 60 seconds retained 400 queued keys with zero materialized identities, staging rows, or materialization-run records. Evidence: `/srv/codex/agents/01a0f565-fc82-7563-a079-af80ead822b6/runs/20261002_143605_performance_b68295595d87_75209/logs/loadgen.jsonl` and `run-metadata.json`.

## Coverage / rollout summary

- `prompt-cache-checkpoint` acceptance seeds 400 conversations and 71,750 synthetic invocations with six large keys, instruments committed staging/publication events in the test database, and checks prefix cursor progression, exact aggregates, 2.85.1 partial staging, queue-drain pause/restart, stable completion, and online p99. One 600-second baseline round reproduces a completed-prefix restart; three 600-second candidate rounds cover cold start, partial staging, and queue-drain recovery. Runtime logs and scenario source are bound to the candidate SHA and scenario digest.
- Delivery evidence records the exact service/harness SHA, scenario digest, per-round completion time, online p99, exact aggregate checks, generation-bound queue deletion, and older-reader compatibility. The queue-drain round intentionally has no stats-rebuild cursor events. Inspect the current run metadata and raw samples at `/srv/codex/agents/01a0f565-fc82-7563-a079-af80ead822b6/validation-control/logs/checkpoint-candidate-run-metadata.json` and `checkpoint-candidate-loadgen.jsonl`; these locators are populated from the final candidate run before review.
- The final candidate's Rust checks and complete backend resource profiles are recorded in `checkpoint-candidate-quality.json` in the same evidence directory, with the validated source SHA and an inspectable Git-input binding to the delivery head.

- `ensure_schema()` creates the conversation master, indexes, mutation triggers, durable refresh queue, and `prompt_cache_conversation_migration_progress`; it does not scan historical invocations, so HTTP readiness is not held by historical data volume.
- The `prompt_cache_conversations_materialization_v1` startup task runs in ordered `identity_backfill`, `identity_reconciliation`, `stats_rebuild`, and `queue_drain` phases. Identity backfill snapshots the maximum invocation row ID, paginates prompt-cache keys, and finishes with an uncursored missing-identity reconciliation. Identity phases commit identities, `cursor_key`, and the identity-key counter together, without scanning aggregate statistics. Statistics phases own the bounded source-page traversal; reruns are idempotent. Completed logical pages and empty phase transitions continue only within the original scan and elapsed-time budgets.
- The `prompt_cache_conversations_v1` identity marker, `prompt_cache_conversations_stats_v2` freshness marker, complete migration phase, and empty durable refresh queue are required together. Aggregate prompt-cache reads and subscription baselines return the existing `ApiError::Unavailable` contract until all conditions hold; partial or zero-valued complete results are not published.
- The trigger-dependency upgrade adds only a completion-marker row and replaces existing trigger definitions in one `BEGIN IMMEDIATE` transaction. It does not add tables or columns, transform source rows, or change HTTP fields. Older readers may ignore the additive marker; rollback does not delete it or restore broad timing-only projection work.
- Invocation triggers continue to enqueue affected keys and remove the freshness marker. New requests remain writable during migration, and failures, SQLite pressure, or process restarts leave durable checkpoints for later retry. Each identity batch or statistics page has a committed checkpoint. A partial statistics page preserves staging progress; only the final generation-consistent page publishes the aggregate and clears its queue generation. The outer statistics cursor advances after the required keys are complete; a priority waiter defers the next boundary after the current step commits.
- The maintenance `managed_tasks.enabled` row is the prompt-cache task's only enablement authority. Its scheduler checkpoint is committed in the same maintenance-database transaction, then the in-memory generation is published before the control PATCH returns. A one-time maintenance metadata marker records whether this managed row existed before task seeding, so a pre-existing maintenance choice wins and a legacy business enable bit seeds only a genuinely new control. The business bit is not queried by the prompt-cache run path or synchronized. A short in-memory permit is acquired before each identity batch, statistics page, cursor update, and phase transition; it is released before/after the bounded business SQL step and never waits on the maintenance database.
- Maintenance checkpoint start/finalization and business-triggered wakes share an asynchronous control-update fence. The generation is rechecked after acquiring the fence, so an older result cannot overwrite a newer pause/resume checkpoint. An already admitted checkpoint may commit before a queued control update, which then takes authority. This fence never spans business materialization SQL; online write admission is released before maintenance-database reads, control-fence waits, and final checkpoint writes. In-memory scheduler mutations also recheck the published generation, preserving a newer resume wake.
- Statistics-page results distinguish complete, pending, source-generation change, budget exhaustion, actual disablement, priority yield, and unavailable maintenance control. Committed staging cursors remain resumable; an incomplete statistics page does not advance the outer key cursor or publish a freshness marker. Page/generation/budget continuations use the existing 15-second follow-up, coordinator-priority retries remain ordinary event-preemptible deadlines, and database pressure retains its event/deadline boundary. Actual disable clears queued, due, and pressure-deferred scheduler state; a new enable generation supplies one wake. Repeated same-value control updates and ordinary business events preserve an unexpired database-pressure retry deadline.
- Proxy allocation briefly accesses the cache to register active references and clone the manager, then releases the cache mutex. The manager reserves an owner slot atomically and shares initialization/refill notifications; only background reservation work enters `InteractiveProxy`. Memory publication follows a confirmed commit. No network or SQL wait is inside the global cache mutex; P1 terminal writers retain priority. The route lookup index is additive and older readers continue to use existing rows if they do not recognize it.
- The adaptive controller starts at 64 keys, doubles after two successful <=50 ms batches, halves at >=200 ms or pressure/failure, and stays within 32..400. Its state is process-local and is reset to 64 after restart. Statistics staging is bounded to 256 invocation rows per source page and a final publish transaction; identity discovery does not repeat those scans. After each committed adaptive batch with more work remaining, a 10 ms scheduler window gives foreground readers a bounded chance to run. Startup logs include `batch_count`, `last_batch_size`, `max_batch_size`, `batch_elapsed_ms`, and `defer_reason`.
- The migration progress row stores the fixed source total and committed identity-key counter after the identity snapshot; stats cursor commits do not double-count those keys. The status surface caps incomplete phases below 100%, reports zero ETA only for the complete phase, and reads only durable counters, bounded queue sampling, and the latest 10 records. Schema setup repairs counters for older complete rows, while failure history preserves the durable phase checkpoint.
- Statistics rebuild now persists the outer cursor with each final conversation-page transaction instead of waiting for the whole logical key batch. This resumes 2.85.1 empty-cursor/partial-staging databases directly, bounds repeat work to the unfinished key, and leaves source mutations behind the cursor for queue drain. Queue-drain publication continues to rely on deletion of its matching generation. Incomplete stats/queue ETA is `null`; complete ETA is zero. Page traces identify conversations only by fingerprint and include generation, source snapshot, page cursor, rows read, staged count, and completion state.
- Earlier adaptive-batch controlled representative SQLite acceptance passed with 3 trials over 4,000 keys and 40,000 invocations: fixed median 129812 ms versus adaptive median 103232 ms; foreground p95 251 us versus 266 us and p99 622 us versus 613 us. The ignored fixture is an explicit empirical acceptance command rather than the unrelated summary-scale CI selector.
- Existing invocation IDs remain readable as historical rows; new HTTP proxy capture uses the compact contract, while no new WebSocket invocation IDs are created after ADR 0020.
- PR2 exposes the durable prompt-cache conversation identity and materialized aggregate breakdowns
  through additive optional fields on `GET /api/stats/prompt-cache-conversations`. Hydration reads
  the master row for selected keys on the same SQLite connection and keeps snapshot statistics
  fail-closed when the materialized last invocation is newer than the snapshot boundary.
- The web API normalizer, Live prompt-cache table, dashboard working-conversation mapper, and live
  merge consumers preserve absent delayed fields, display the identity/token/cost breakdown, and
  use `firstInvocationAt` for count-mode history ordering while retaining live activity ordering
  for working conversations.

## Delivery and Rollout Gates

- `REQ-PII-008`, `REQ-PII-009`, and `REQ-PII-010` require final candidate regression and service evidence. Focused stateful SQLite and file-lock regressions cover memory-only issuance with the pool closed and write admission held, mixed-owner thresholds, commit failure, restart skipping, bounded activity and seed merge, covering-index selection, conditional/busy tail return, protected-slot saturation, legacy/pending recovery floors, namespace release, hourly rollover, late guard callbacks, and cancellation. These package checks do not replace final candidate validation.
- The extended Linux harness installs ceiling-update observation triggers only in isolated acceptance databases. A controlled 73-call probe prepares two conversations and one hourly owner, then verifies six ceiling updates and a shared three-owner refill. Long-wait replay additionally kills the service after recording committed ceilings and requires restart to preserve prefixes and issue above those ceilings. Checkpoint older-reader smoke snapshots durable ownership and requires it to remain unchanged. The scenario digest includes `scripts/invocation-range-acceptance.py`; these fixtures add no production allocation files.
- Long-wait evidence reports the contract's explicit 100 ms cold admission/empty-range HTTP 503 outcomes separately and includes their full response latency in the allocation p99. Cached-owner failures and other cold errors still fail the scenario. Exit-time service logs are retained for failed startup or workload diagnostics.
- Long upstream wait fixtures first admit each identity through a real fast proxy request, with at most five setup attempts and visible cold-refusal counts. This ensures each 179/180-second assertion observes an upstream call; the sustained workload still creates a fresh cold conversation every third request.
- Control and checkpoint fixtures likewise admit their pressure-probe and steady-input owners before acquiring the synthetic SQL lock or starting the measured input window. Setup attempts and cold refusals remain visible, and the extra control-target invocation joins its expected aggregate count. This makes the priority probe exercise hot allocation followed by terminal-write admission; measured allocation, terminal, progress, restart, and exact checkpoint-aggregate gates remain unchanged.
- The file-backed retirement-worker regression likewise prepares its first and replacement identities with at most five attempts, retrying only the allocator's permitted cold 100 ms timeout outside the measured lock race. The allocation attempted while the database lock is held, generation-drain fence, recovered sequence assertions and production deadlines remain unchanged.
- The control fixture records staging-page commits through isolated test-database triggers. Its progress gate observes committed events even when aggregate publication removes a staging row between samples; rolled-back pages produce no event. Final staging and refresh queues must still be empty, and the existing progress and completion deadlines continue to apply.
- Before taking the synthetic priority-probe lock, fixture setup checks the existing runtime pressure snapshot so startup rollup/P2 work can drain without that test lock. The same bounded probe deadline applies; unavailable pressure evidence or failure to observe a fresh priority yield still fails the fixture.
- Three-round Linux, non-test service-process `prompt-cache-control` acceptance is a required delivery gate. Delivery evidence carries the exact candidate SHA, scenario digest, persistent checkpoint samples, and completion results; the older adaptive benchmark does not replace this gate.
- Three-round Linux, non-test service-process `prompt-cache-checkpoint` acceptance is a required delivery gate. Current SHA-bound raw samples and metadata must pass before review or PR readiness.
- The representative runtime fixture is intentionally ignored by default and must be rerun in a controlled environment when SQLite size or lock-contention characteristics change.

## Development Work Order

This is the locked scope for the PR1 allocation correction. Its packages form one backend delivery
boundary; they are not separate independently deployable allocator implementations. Public API
and frontend implementation belongs to PR2 after backend acceptance.

| Package                                  | Dependencies  | Required result                                                                                                                                                                                                     | Acceptance                                                                                                                                               |
| ---------------------------------------- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A1: Durable ownership and upgrade        | None          | Add the hourly table and immutable schema marker; preserve existing conversation masters and historical IDs; make the allocator the only ceiling writer; recover known floors on demand.                            | Idempotent and interrupted upgrade, old partial materialization state, no statistics overwrite, forward repair of committed reservation state.           |
| A2: Unified range manager                | A1            | One coordinator for conversation and hourly owners; 64-sequence reservations; strict 31/32 and 47/48 thresholds; one standby/refill; commit before memory publication; hot issuance without SQL.                    | Concurrent mixed-owner batching, failed/ambiguous commit, same-hour restart, cancellation, final partial range, and database contention.                 |
| A3: Activity sizing and cache retirement | A2            | Immediate capacity 128; one indexed asynchronous seed of at most 4096 rows; independent 48-hour activity memory; hourly memory-only resizing; temporary growth and bounded slot admission; conditional tail return. | Delayed seed merge, expiry, query plan, protected shrink, 4096 saturation, 100 ms admission/return/refill waits, and stale-generation fences.            |
| A4: Lifecycle and diagnostics            | A2, A3        | Protect active and pending persistence identities; carry existing IDs across hour changes; release eligible masters/hourly rows and namespace state; add structured owner/range/cache logs.                         | Terminal journal recovery, retention versus refill/return races, busy/timeout warnings, release then reuse, no raw keys or fragmented reservation files. |
| A5: Candidate and online acceptance      | A1 through A4 | Bind focused regressions and Linux service-process evidence to the delivery SHA; verify statistics convergence and allocation under pressure before PR2.                                                            | `VER-PII-003`, `VER-PII-008`, `VER-PII-009`, `VER-PII-010`, plus the existing materialization and online latency gates.                                  |

For timed-out return SQL, discard the cache's issuance rights within the budget while retaining
the owner fence until cancellation/rollback or the committed outcome is known. A replacement must
not race that operation; it follows bounded admission rather than extending an old SQL operation
into an unlimited request wait. No global cache lock spans a database wait.

Migration and release-impact records are
`assets/allocation-range-migration-record.json` and
`assets/allocation-range-version-impact-record.json`. They distinguish focused compatibility
verification from full candidate service and delivery gates. The verified compatibility impact is
Minor because durable ownership changes the supported writer contract; public response fields
remain compatible. PR2 has a separate `assets/pr2-version-impact-record.json`: its additive read
fields are `public_api=patch`, while persistent-state impact is not applicable because it reads
existing master rows without schema or write-path changes. Final release identity follows live
repository policy and candidate evidence.

Place focused regressions in the matching backend resource bucket. Run named regressions and
formatting checks appropriate to each package; use the documented resource-profile runner and
shared testbox for heavy backend validation and non-test Linux replay. Reuse the existing
materialization scenarios and their latency/completion limits; extend evidence with actual database
operation counts, range commit/publication, cache sizing, crash recovery, rollover, and cleanup.
Historical acceptance locators above do not certify the new allocator candidate.

PR2 updates public API and frontend consumers to use persisted conversation identity and delayed
aggregate statistics. Production deployment and acceptance remain outside this local delivery.

## Related Changes

- `docs/adr/0021-prompt-cache-background-materialization.md`
- `docs/adr/0022-prompt-cache-adaptive-materialization.md`
- `docs/adr/0027-prompt-cache-materialization-step-boundaries.md`
- `docs/adr/0028-prompt-cache-continuous-statistics-checkpoints.md`
- `docs/adr/0029-conversation-invocation-range-reservations.md`
- `docs/adr/0030-durable-hourly-invocation-prefixes.md`
- `docs/adr/0031-prompt-cache-event-wake-priority.md`
- `docs/specs/proxy-invocation-identity/assets/allocation-range-migration-record.json`
- `docs/specs/proxy-invocation-identity/assets/allocation-range-version-impact-record.json`
- `docs/specs/proxy-invocation-identity/assets/pr2-version-impact-record.json`
- `docs/specs/proxy-invocation-identity/assets/version-impact-record.json`

## References

- `./SPEC.md`
- `./HISTORY.md`
