# Rust Source Quality Preparation Contract

## Context and Scope

This topic establishes an executable Rust source-quality baseline for
module-oriented refactor pull requests. The initial preparation implementation
adds repository tooling and CI coverage; subsequent bounded module extractions
may move production or test/helper code without changing runtime behavior.

The contract covers `src/**/*.rs` physical line budgets for explicitly selected
files, the `src/` `include!` boundary, and the incremental inventory of
standalone `#[allow]` and `#[expect]` declarations.

It does not cover Web code, Docker or shared-testbox execution, release
publication, or a global repository file-length rule.

## Terms and Interfaces

- A physical line is a line returned by the checker, including blank and
  comment lines and a final line without a newline.
- A selected path is a path explicitly listed in
  `.github/rust-source-quality-policy.json`.
- A destination target is the policy metadata value of 2,500 production lines
  or 3,000 test/helper lines. It is planning guidance, not a global failure
  threshold and does not discover future files.
- A cohesive-module exception is an explicit policy object with a non-empty,
  reasoned `reason`; it is never a wildcard or a threshold-based match.
- A suppression inventory key is `(path, kind, normalized declaration)`. The
  checker compares the complete current multiset to policy, so a new or
  materially altered declaration fails until its narrow reason is recorded.
- `bash .github/scripts/run-rust-source-quality.sh` is the canonical runner.
  `bun run verify:rust` and both existing `Lint & Format Check` jobs delegate
  to this runner.

## Requirements

### REQ-RUST-SOURCE-QUALITY-001

The canonical runner MUST execute, in order, the repository-wide rustfmt check,
locked all-target Clippy with all features and `-D warnings`, and the
dependency-free source-quality checker.

covers: VER-RUST-SOURCE-QUALITY-001

### REQ-RUST-SOURCE-QUALITY-002

The source-quality checker MUST evaluate only the explicit file inventory. It
MUST enforce each selected path's recorded `line_budget`, and MUST NOT scan for
unlisted files using the 2,500/3,000 destination targets.

covers: VER-RUST-SOURCE-QUALITY-002

### REQ-RUST-SOURCE-QUALITY-003

The checker MUST reject `include!` control-flow composition anywhere under
`src/` and MUST compare all standalone `#[allow]` and `#[expect]` declarations
against the reasoned suppression inventory.

covers: VER-RUST-SOURCE-QUALITY-003

### REQ-RUST-SOURCE-QUALITY-004

The policy MUST keep the current inventory as 24 explicit file entries: 7 production candidates above 2,500 lines and 17 test/helper candidates above 3,000 lines. The immutable preparation baseline retains its original candidate counts for checker compatibility. Each current entry MUST record its exact current budget, role, and either a specific next module workstream or a reasoned cohesive-module exception.

covers: VER-RUST-SOURCE-QUALITY-004

### REQ-RUST-SOURCE-QUALITY-005

Both CI workflows MUST continue to expose the existing `Lint & Format Check`
job with the same required-check topology. The runner MUST be invoked inside
that job without enabling global Clippy pedantic lints or adding dependencies.

covers: VER-RUST-SOURCE-QUALITY-005

### REQ-RUST-SOURCE-QUALITY-006

The fixture harness MUST cover selected-path growth, lower-budget enforcement,
an unselected long file, missing cohesive-exception reasoning, unrecorded and
altered suppressions, and forbidden `include!` composition without compiling
the fixtures.

covers: VER-RUST-SOURCE-QUALITY-006

### REQ-RUST-SOURCE-QUALITY-007

The admitted parallel-work baseline repair MUST reuse one borrowed SQLite connection across the complete cold load while retaining the existing row fence, data-version checks, SQL, pool capacity, acquisition deadline, retries, and public response shape. Auto mode MUST retain its typed materializer and buffered runtime replay; Legacy mode MUST retain its database JSON payload and runtime replay boundary. A builder holding its observer MUST NOT acquire another connection from the same pool.

covers: VER-RUST-SOURCE-QUALITY-007

### REQ-RUST-SOURCE-QUALITY-008

The missing-pool-key terminalization fixture MAY prepare its real committed unkeyed invocation range before subscribing and issuing the original request. Preparation MUST remain test-only, allocate no ID, create no active lease, fail on initialization error or an hourly-owner transition, and retain a five-second Arrange bound. The production 100 ms allocation policy and every original HTTP, Running, terminal, identity, and in-flight assertion MUST remain unchanged. The startup pending-repair fixture MAY call the existing legacy account archive marker-repair routine after its original seeded-cursor and candidate-count assertions and before spawning bootstrap. This Arrange prerequisite covers only the three legacy account usage/hourly-stats/minute-stats markers bound to the current manifest SHA; it MUST NOT seed a usage-breakdown replay marker or advance/reset its repair cursor. Every original SQL literal and assertion, the five-second watchdog, 513 candidates, cursor transition from 64 to 320, deferred status, cancellation, one-second join, and execution slots MUST remain unchanged. This fixture accepts pending-cursor deferral with the legacy marker prerequisite prepared; it does not establish cold missing-marker backfill latency.

covers: VER-RUST-SOURCE-QUALITY-008

### REQ-RUST-SOURCE-QUALITY-009

The cold Bootstrap fallback fixture MAY accept initial hydration completing before the `BeforeHistoricalLiveCoverage` hook notification only when the actual outer Tokio build timeout returned `Elapsed`, its one JoinHandle result succeeds, and the existing in-memory Bootstrap fallback fence is present. A narrow `cfg(test)` task-local observer MUST record expiry in the outer timeout error branch before inner build errors are flattened; error-string classification alone is insufficient. This explicitly replaces the hook-first panic, while preserving the original one-second deadline, all Arrange SQL, hook installation and pause/Notify mechanism, Projection assertion, and exact `current` count 1, `1d` count 0, and `7d` unavailable checks after SQLite closes. No retry, execution-slot change, fake clock, forced earlier hook, or production deadline change is permitted. The paged-boundary fixture retains its four-second production Rolling deadline and every original assertion.

covers: VER-RUST-SOURCE-QUALITY-009

## Policy Shape

The checked-in policy contains `schema_version`, baseline counts and commit,
destination `loc_targets`, explicit `files`, and explicit `suppressions`.
Each file has `path`, `role`, and `line_budget`, followed by exactly one of
`next_module_workstream` or `cohesive_exception.reason`. Suppression entries
have `path`, `kind`, normalized `declaration`, and a narrow `reason`.

The current inventory retains no cohesive-module exceptions: all 24 entries have specific next module workstreams. The schema and fixture harness retain the exception form for a future entry only when its reason is explicit and cohesive, never as an escape hatch for an unselected or growing file.

The prompt-cache conversation query governance extraction removes the original parent from the selected inventory and assigns all 60 existing tests exactly once: 47 stateful SQLite cases across metadata/history, persisted statistics, activity/chart windows, pagination, the reused snapshots suite, and runtime/cache coordination; four in-memory response codec cases to lightweight; and nine file-backed metadata backfills to archive/file I/O. After rustfmt, the parent is 10 lines, the stateful children are 923, 1,536, 681, 903, 1,402, 617, and 16 lines, and the resource children are 85 and 591 lines; all affected files are below the 3,000-line test/helper target. The extraction preserves test identities, attributes, assertions, SQL, fixtures, timing, lock/task lifetimes, and cleanup, while consolidating only the three identical count-mode row inserters and moving the two private wrappers to `test_support.rs`; the final candidate also removes an unused archive-test import without changing behavior; immutable preparation counts remain 32/23 and suppression inventory remains 117.

The group note CRUD extraction moves the complete `update_upstream_account_group`
and `delete_upstream_account_group` handler region from physical lines 2,043
through 2,501 of `src/upstream_accounts/crud_group_notes.rs` into
`src/upstream_accounts/crud_group_notes/group_notes.rs`. After rustfmt, the
parent is 2,261 physical lines and the child is 460 physical lines. The parent
is removed from the current inventory while route names, signatures, visibility,
SQL, validation, responses, tests, and runtime behavior remain unchanged.

The proxy metadata backfill extraction moves the complete contiguous region from
the leading `#[cfg(test)]` attribute for `backfill_proxy_missing_costs` through
`backfill_proxy_reasoning_efforts` in `src/proxy/payload_utils.rs` into
`src/proxy/payload_utils/backfill_metadata.rs`. On the verified main baseline
`61c8b2e0b0ec8ddd1f295d03d89aba1e05f14f6a`, the moved region is physical lines
3 through 513 inclusive (511 lines). It contains cost backfill and retry
helpers plus prompt-cache-key, requested-service-tier, and reasoning-effort
backfills and their test-only items. After rustfmt, the parent is 2,427 lines
and the child is 513 lines, both below the 2,500-line production target, so the
parent is removed from the policy inventory. The parent retains `use super::*`,
the child module declaration, and a crate-visible re-export; names, signatures,
conditional compilation, SQL, timing, tests, and runtime behavior remain
unchanged. The suppression inventory remains at 117 entries, with the moved
allow recorded under the child path.

The upstream route-binding penalty and live-candidate evaluation extraction
moves the complete contiguous region from physical lines 390 through 819
inclusive (430 moved lines) of
`src/upstream_accounts/routing/selection.rs` into
`src/upstream_accounts/routing/selection/live_candidate.rs`, on the verified
main merge base `c94f1f0a0f6ee51e0c442a6007d431c53ee7f151`. The parent retains
`LivePoolCandidateEvaluation`, scoring helpers, and resolver logic. After
rustfmt, the parent is 2,449 physical lines and the child is 432 physical
lines, both below the 2,500-line production target, so the parent is removed
from the policy inventory. The parent re-export and routing module export keep
the existing crate-visible call paths; function signatures, visibility, SQL,
routing behavior, and test semantics remain unchanged.

The system raw-payload metrics inventory and capture-circuit lifecycle moves
as one contiguous region from physical lines 1,049 through 1,678 inclusive
(630 moved lines) of `src/api/slices/system_routes_and_tasks.rs` into
`src/api/slices/system_routes_and_tasks/raw_payload_inventory.rs`, on the
verified main base `8d0d2d1197f61776807f2e90e022776f6401e39b`. The parent
retains the preceding filesystem scanner and following status snapshot/task
lifecycle. After rustfmt, the parent is 2,364 physical lines and the child is
632 lines, both below the 2,500-line production target. The parent is removed
from the policy inventory. Its crate-visible re-export preserves existing call
paths; names, signatures, SQL, timing, logging, cache and circuit behavior
remain unchanged.

The pricing catalog/settings extraction moves the complete 17-test block from
physical lines 24 through 996 of
`src/tests/stateful_sqlite/pricing_catalog_and_models_passthrough.rs` into
`src/tests/stateful_sqlite/pricing_catalog_and_models_passthrough/pricing_catalog.rs`.
The parent retains its large-stack helper and later model/proxy/routing tests.
After rustfmt, the parent is 2,424 physical lines and the child is 974 lines,
both below the 3,000-line test/helper target. The parent is removed from the
policy inventory; test names, assertions, fixtures, timing, and resource bucket
remain unchanged.

The application runtime-state extraction moves invocation records, terminal and
projection tombstones, pruning, prompt-cache projections, and memory estimates
into `src/app_state/invocation_store.rs`, and moves dashboard current/network/
terminal projection state, coalescing, publication windows, baselines, captures,
and topology counters into `src/app_state/dashboard.rs`. The parent retains the
`RuntimeProjectionHub` coordinator, request-pipeline metrics, lifecycle counters,
and explicit crate-visible re-exports. It preserves store-before-dashboard lock
ordering, does not await while either state lock is held, and keeps mutation,
rollback, prune synchronization, deadline, and revision behavior unchanged.
After rustfmt, the parent is 1,661 physical lines, `dashboard.rs` is 1,266
lines, and `invocation_store.rs` is 577 lines; all are below the 2,500-line
production target. The parent is removed from the policy inventory, reducing
the current inventory to 22 production and 18 test/helper entries (40 total).
The immutable preparation baseline remains 32 and 23, and the suppression
baseline remains 117. The summary API slice remains unchanged and frozen for
this extraction.

The prompt-cache conversation binding extraction moves operation-event snapshot
encoding and filtered operation-event reads into
`src/api/slices/prompt_cache_and_timeseries/prompt_cache_conversations/bindings/operation_log.rs`,
and encrypted-session owner/routing-policy logic into
`src/api/slices/prompt_cache_and_timeseries/prompt_cache_conversations/bindings/routing_policy.rs`.
The parent retains binding request models and validation, binding persistence,
sticky-route transaction and broadcast coordination, HTTP handlers, and
cache/subscription coordination. Explicit crate-visible re-exports preserve
the existing callers, while database rows and operation-log serialization
details stay inside the operation module.

The operation-event writers now share one private typed payload and one INSERT
implementation while preserving the existing nullable JSON and executor
semantics. After rustfmt, the parent is 2,277 physical lines, the
operation-log module is 1,115 lines, and the routing-policy module is 654
lines. All three are below
the 2,500-line production target, so the parent is removed from the policy
inventory. The current inventory is 21 production and 18 test/helper entries
(39 total); the immutable preparation baseline remains 32 and 23 and the
suppression baseline remains 119. Binding behavior, transaction ordering,
sticky-route mutation semantics, owner routing, API payloads, and test resource
classification remain unchanged.

The prompt-cache timeseries extraction separates the parent into explicit
aggregation, materialization, minute-projection, parallel-work, and query
modules, with restart recovery nested under minute projection. The parent keeps
the HTTP extractor and compatibility wrappers; the child modules keep existing
crate-visible call paths through direct module imports and the narrow required
re-exports. After rustfmt, the parent is 146 physical lines, while
`aggregation.rs`, `materialization.rs`, `minute_projection.rs`,
`parallel_work.rs`, `queries.rs`, and `recovery.rs` are 842, 487, 1,830, 288,
1,382, and 148 lines respectively. All selected production paths are below
the 2,500-line target, so the parent is removed from the policy inventory. The
current inventory is 20 production and 18 test/helper entries (38 total); the
immutable preparation baseline remains 32 and 23 and the suppression baseline
remains 119. Query routing, aggregation semantics, minute projection fences,
materializer overlays, parallel-work responses, and test resource buckets
remain behaviorally unchanged.

## Subscriptions Module Rollout

The subscriptions slice is now a real `src/api/slices/subscriptions/` module directory. Its production responsibilities are split across `config.rs`, `protocol.rs`, `query.rs`, `topic.rs`, `state.rs`, `materializers.rs`, `hub_lifecycle.rs`, `hub_projection.rs`, `hub_projection_runtime.rs`, `hub_recovery.rs`, and `server_push.rs`; existing `topic_builders.rs` remains in place. The inline test suite is grouped into summary, parallel-work, hot-topic, conversation, network, replay, and shared-support modules while retaining the existing prompt-cache and managed-task test modules. No `include!` composition is used, the largest production file is below 2,500 lines, and the largest generated test/helper file is below 3,000 lines.

The subscriptions parent is removed from the explicit source-quality inventory after the extraction. The policy now contains 35 selected entries; immutable baseline candidate counts remain 32 production and 23 test/helper entries, and the suppression inventory is unchanged.

## Long-Term Statistics Module Rollout

The long-term statistics source is now a real `src/long_term_stats/` module directory. Production responsibilities are split across model, archive, schema, projection runtime, projection rebuild, integrity, refresh, intervals, and the existing projection flush module. The inline tests are grouped into control, integrity, intervals, refresh, archive, and shared support modules; no `include!` composition is introduced.

The obsolete `src/long_term_stats.rs` entry is removed from the explicit policy inventory. The policy now contains 34 selected entries, with 17 production and 17 test/helper files; the immutable baseline remains 32 production and 23 test/helper candidates, and the 118-entry suppression inventory follows the moved refresh declarations.

## Archive Cleanup Extraction

The archive cleanup facade now delegates to explicit compression-backlog, deletion, historical-rollup, legacy-mirror, storage, Summary Snapshot, and test modules under `src/maintenance/archive/cleanup/`. The production coordinator, archive-file identity implementation, and `prune_archive_batches` orchestration remain in `cleanup.rs`; the 21 existing cleanup tests remain under the same `archive_cleanup::tests` module path. After rustfmt, every extracted production file is below the 2,500-line target and the test helper is below the 3,000-line target, so the obsolete `src/maintenance/archive/cleanup.rs` inventory entry is removed. Names, SQL, timing, file-identity checks, deletion transaction boundaries, proof state, test names, and resource classification remain unchanged; the current policy is 33 entries with 16 production and 17 test/helper candidates and 125 suppression entries.

## Later Module Rollout

The archive hourly-rollup parent MUST remain a facade with its existing inline test modules and the existing progress/support modules. Entry and archive materialization, invocation replay, account activity repair, network minute replay, and retention reconciliation MUST live in named children under `src/maintenance/archive/hourly_rollups/`. The parent and all five production children MUST remain below 2,500 physical lines, and the parent MUST be absent from the selected inventory. Compatibility exports, SQL, transaction boundaries, timing, caches, fixture semantics, test assertions, and resource buckets MUST remain unchanged.

Refactor PRs may split one production or test/helper candidate at a time. Such
a PR updates the affected explicit budget and workstream, removes an entry only
after the path is no longer a selected candidate, and updates suppression
coverage whenever declarations are moved or changed. Completed extractions are
recorded in the current implementation and history sections of this topic.

## Route Selection Module Rollout

The route-selection source is now a facade over `route_semantics.rs`, `pool_resolution.rs`, `pool_forwarding.rs`, and `forward_retry.rs`, while the existing `auth.rs` boundary is retained. After rustfmt, the parent is 25 physical lines and the new production children are 1,199, 582, 1,724, and 274 lines; every production path is below the 2,500-line target. The parent is removed from the explicit policy inventory, leaving 25 entries with 8 production and 17 test/helper candidates; immutable preparation counts remain 32 and 23 and suppression entries remain 123. Existing crate-visible exports, replay semantic parsing, sticky and prompt-cache owner routing, reservation and attempt persistence, response streaming, 429 retry behavior, observability, tests, and the frozen invocations module remain unchanged.

## Usage Persistence Module Rollout

The usage-persistence source is now a facade over explicit attempt-persistence, attempt-recovery, invocation-records, raw-payload-storage, streaming-raw-payload, and terminal-rollups modules, while the existing attempt-public-ID backfill, invocation-identity, runtime-recovery, and terminal-metadata modules remain in place. The parent is 25 physical lines and every new production child is below the 2,500-line target; the parent is removed from the explicit inventory, leaving 24 entries with 7 production and 17 test/helper candidates. The invocation summary source remains frozen. Existing crate-visible call paths, SQL, terminal/attempt lifecycle, raw-payload retention, streaming capture, tests, resource buckets, and the 123-entry suppression inventory remain unchanged; the two moved targeted suppressions are recorded under their new child paths.

## Verification

### VER-RUST-SOURCE-QUALITY-001

Method: inspect and execute the canonical runner and its package delegation.
Pass condition: rustfmt, locked all-target Clippy, and the source-quality
checker run in that order.

covers: REQ-RUST-SOURCE-QUALITY-001

### VER-RUST-SOURCE-QUALITY-002

Method: run the dependency-free fixture harness against selected and unselected
fixture paths.
Pass condition: selected growth and lower budgets fail while an unselected long
file passes.

covers: REQ-RUST-SOURCE-QUALITY-002

### VER-RUST-SOURCE-QUALITY-003

Method: run the source-quality checker on suppression and `include!` fixtures.
Pass condition: unrecorded or altered suppressions and `include!` fail.

covers: REQ-RUST-SOURCE-QUALITY-003

### VER-RUST-SOURCE-QUALITY-004

Method: inspect and validate the checked-in policy baseline.
Pass condition: the policy has 8 production and 17 test/helper entries (25 total), with exact budgets and explicit workstreams or reasoned exceptions.

covers: REQ-RUST-SOURCE-QUALITY-004

### VER-RUST-SOURCE-QUALITY-005

Method: run the repository quality-gates contract test against both workflow
definitions.
Pass condition: the existing lint job name and required-check topology remain
valid while both jobs invoke the canonical runner.

covers: REQ-RUST-SOURCE-QUALITY-005

### VER-RUST-SOURCE-QUALITY-006

Method: run the complete repository-local source-quality fixture and formatting
checks without compiling fixtures.
Pass condition: all required positive and negative fixtures pass their expected
outcomes.

covers: REQ-RUST-SOURCE-QUALITY-006

### VER-RUST-SOURCE-QUALITY-007

Method: run the Auto and Legacy one-connection baseline regressions through the canonical lightweight backend runner, retaining the persisted-conversation and caller-level buffered-runtime assertions, together with the original unknown-account reconcile test and complete backend profiles. Pass condition: six real concurrent builders complete with one connection and preserve their mode-specific payload and replay boundaries; original assertions and all three backend profiles pass. Old-source RED and temporary-candidate observations remain diagnosis evidence and do not replace formal validation.

covers: REQ-RUST-SOURCE-QUALITY-007

### VER-RUST-SOURCE-QUALITY-008

Method: run the original missing-pool-key terminalization test, the five preserved stateful fixture regressions with the canonical schema-template prerequisite, and all three complete backend profiles. Pass condition: the real range is committed before the original missing-key request, all original terminalization assertions pass, and the startup watchdog, cold-conversation, retention, and pruned-summary regressions pass without timeout, SQL, assertion, or slot relaxation. Preserve every failed run separately from later candidates and diagnostics. The startup fixture uses the admitted legacy-marker Arrange prerequisite while retaining all original SQL and assertions; its acceptance claim excludes cold legacy-marker completion.

covers: REQ-RUST-SOURCE-QUALITY-008

### VER-RUST-SOURCE-QUALITY-009

Method: validate the cold Bootstrap fixture through the canonical lightweight runner with its mandatory schema-template prerequisite, original one-second deadline, and all-slots override; retain the complete candidate's required backend and source-quality gates. Pass condition: actual outer deadline expiry observed in the same spawned task, one successful hydration join, a published Bootstrap fallback fence, and every original Projection/current/1d/7d assertion pass. Static review MUST confirm that each select outcome consumes its JoinHandle result once and that the observer changes no production error, clock, admission, or retry behavior. A single pass does not establish execution of both select arms, the exact stage or underlying cause of the elapsed deadline, or resolution of the separate four-second Rolling failure. Preserve earlier failed formal runs and temporary observations independently.

covers: REQ-RUST-SOURCE-QUALITY-009

No owner-visible UI changes are part of this contract. Backend runtime regressions require the canonical tests above; temporary stress samples are supporting diagnosis rather than a separate performance acceptance claim.

## Related ADRs

None

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
- `../../../.github/rust-source-quality-policy.json`
