# Rust Source Quality Preparation Contract

## Lifecycle

The archive hourly-rollup extraction starts from `2b0b1477e753291876475f809e5c6906f644f8b4`, after the archive-cleanup extraction. It completes governance of the original 7,079-line `src/maintenance/archive/hourly_rollups.rs` in one PR, preserving the existing test module boundaries and progress/support files while extracting five named production responsibilities. The parent leaves the canonical candidate inventory; no remaining split of this parent is deferred. The current inventory decreases from 33 to 32 entries, with 15 production and 17 test/helper candidates and unchanged 125 suppression entries.

- The topic begins with an executable preparation contract rather than a
  production-module refactor.
- The current policy is anchored to the verified mainline baseline and keeps
  its 32 large-file entries explicit.
- Later module-oriented refactor PRs consume this contract one bounded source
  or test/helper area at a time.

## Current Truth

The canonical runner, standard-library checker, policy, fixture harness, CI
delegation, and package delegation are current implementation. Older archive
planning milestones are not part of this topic's current scope. The baseline
has no retained cohesive-module exceptions; exception support remains explicit
and reason-required for future use.

The invocation query foundation is a named child module under the existing
invocations-and-summary parent. Query projection, filter parsing and SQL
construction, request and snapshot types, snapshot resolution, and ordering
remain behaviorally unchanged while the selected parent budget decreases from
45,660 to 44,631 physical lines. The new 1,049-line production file remains
below the inventory target.

The summary-projection lifecycle is now a second named child module under the
same parent. The module boundary is the contiguous verified-base region from
`summary_snapshot_bootstrap_keys` through the complete
`await_summary_projection_all_time_build` definition, physical lines 10,259
through 12,016 inclusive (1,758 lines). Snapshot bootstrap/hydration, refresh
routing, coverage-recovery maintenance, source-tail restoration, durable
publication coordination, and all-time deadline handling remain behaviorally
unchanged. The parent budget decreases from 44,631 to 42,897 physical lines;
the resulting 1,761-line production module remains below the inventory target
and is not selected. Summary-projection and workflow-detail responsibilities
remain explicit follow-up work in the parent policy.

Validation evidence for these extractions is the focused existing
summary-projection/stateful SQLite coverage, the stateful SQLite backend
profile, the Rust source-quality runner, all-target Cargo checking, and
`git diff --check`.

The workflow-detail region is now a third named child module under the same
parent. Its verified-base boundary is the complete
`fetch_invocation_pool_attempts` definition through the complete
`fetch_invocation_request_body` definition, physical lines 2,992 through
5,421 inclusive (2,430 moved lines). Pool-attempt retrieval, workflow identity
and response models, hero/timeline construction, upstream-account attempt
hydration, workflow-detail reads, request/response body retrieval and raw-body
fallbacks, summary construction, record detail, and their handlers remain
behaviorally unchanged. The resulting child is 2,436 physical lines; the
parent decreases from 42,897 to 40,532 physical lines. The child remains below
the production target and is not added to the inventory. The remaining parent
workstream is the named summary-projection models/builders plus dashboard
activity/network and summary/history/suggestions/stats responsibilities.

The focused workflow-detail audit test passed after the move. Full delivery
validation remains the stateful SQLite backend profile, the Rust source-quality
runner, all-target Cargo checking, and `git diff --check`.

The Dashboard Activity snapshot-cache state and invalidation region is now a
fourth named child module under `settings_models_and_cache`. The complete
cache selection, entry, terminal-delta, read-model, in-flight, cache-state,
flight-guard, memory-estimate, selection-fingerprint, and invalidation
responsibilities move into
`src/api/slices/settings_models_and_cache/dashboard_activity_cache.rs` while
existing crate-visible consumers retain the parent paths through explicit
re-exports. The parent decreases from 2,555 to 2,293 physical lines and the
279-line child remains below the production target. The parent candidate is
removed from the quality policy inventory. Runtime behavior, persistence,
schema, SSE, and public API contracts remain unchanged.

The OAuth bridge test module is now a fifth named child module. The complete
test block moves from `src/oauth_bridge.rs` to
`src/oauth_bridge/tests.rs`; the parent retains the same test module path
and private-item access. The parent is 2,040 physical lines and the child is
720 physical lines, both below their targets. At that point in the extraction
sequence, the inventory therefore had 31 production and 22 test/helper
candidates (53 entries total), while
the immutable preparation baseline remains 32 and 23. No production OAuth
bridge logic, test names, assertions, or public behavior changed.

Validation for this extraction is the focused Dashboard Activity cache tests,
both lightweight and stateful SQLite backend profiles, rustfmt, all-target
Cargo checking, all-target Clippy, the Rust source-quality runner, and
`git diff --check`.

The service-tier backfill test group is now a named child module under the
existing `proxy_backfill_and_cost_repairs` parent. Its verified baseline
boundary is physical lines 34 through 193 inclusive (160 moved lines), ending
immediately before the next proxy usage-token test. The three tests, fixtures,
assertions, thresholds, names, and `stateful_sqlite` resource bucket remain
unchanged. The parent is 2,948 physical lines and the child is 162 physical
lines, both below the 3,000-line test/helper target, so the parent is removed
from the quality policy inventory. No production runtime behavior changes.

Validation for this extraction is the focused service-tier backfill tests, the
stateful SQLite backend profile, rustfmt, the Rust source-quality checker and
fixture harness, all-target Cargo checking, all-target Clippy, and
`git diff --check`.

The upstream routing status persistence extraction moves the complete
contiguous production region beginning with `set_account_status` and ending
with the complete `record_classified_account_sync_failure_with_proxy_snapshot`
definition from `src/upstream_accounts/sync_routing_status.rs` into
`src/upstream_accounts/sync_routing_status/status_persistence.rs`. On the
verified main merge base `3d06762198c3abff18806fce8ff65a05030a2d32`, the exact
boundary is physical lines 2,038 through 2,576 inclusive (539 moved lines).
The parent is now 2,154 physical lines and the child is 541 physical lines,
both below the 2,500-line production target. The parent keeps the existing
`account_action_event_tests` module, and no test names or resource buckets
moved. The current inventory is 30 production and 22 test/helper candidates
(52 entries total), while the immutable preparation baseline remains 32 and
23.

Validation for this extraction is the focused sync/status and stateful SQLite
cooldown coverage, rustfmt, the Rust source-quality checker and fixture
harness, all-target Cargo checking, all-target Clippy, and `git diff --check`.

The request-prefix extraction moves the complete contiguous production region
beginning with `best_effort_extract_json_string_for_patterns` and ending with
`prepare_target_request_body_with_hosted_intent` from
`src/proxy/stream_gate.rs` into
`src/proxy/stream_gate/request_prefix.rs`. On the verified main merge base
`52b82d70a76f63682ee27d502b070d7bdd0903c6`, the exact boundary is physical
lines 34 through 441 inclusive (408 moved lines). Prefix extraction,
encrypted-content detection, request-prefix tests, and request-body preparation
remain behaviorally unchanged. The parent is now 2,338 physical lines and the
410-line child remains below the 2,500-line production target; the parent is
removed from the quality policy inventory. Existing test names, resource
buckets, suppression paths, and crate-visible call paths remain unchanged. The
current inventory is 29 production and 22 test/helper candidates (51 entries
total), while the immutable preparation baseline remains 32 and 23.

Validation for this extraction is the focused request-prefix and request-body
tests, rustfmt, the Rust source-quality checker and fixture harness, all-target
Cargo checking, all-target Clippy, and `git diff --check`.

The forward-proxy probe and validation extraction moves the complete contiguous
region beginning with `parse_forward_proxy_nodes_latency_test_keys` and ending
with `complete spawn_forward_proxy_bootstrap_probe_round` from
`src/forward_proxy/slices/storage_and_hourly_stats.rs` into
`src/forward_proxy/slices/storage_and_hourly_stats/probe_and_validation.rs`.
On the verified main merge base `cdb7fbfa85e460e5fa666aa66834c9fc52745b8e`,
the exact boundary is physical lines 2,401 through 3,842 inclusive (1,442
moved lines). The parent is now 2,404 physical lines and the child is 1,444
physical lines, both below the 2,500-line production target, so the parent is
removed from the quality policy inventory. Manual latency probes, candidate and
subscription validation, endpoint probing, and bootstrap probe scheduling
remain behaviorally unchanged; the parent retains crate-visible call paths
through an explicit child-module path and re-export. The current inventory is
28 production and 22 test/helper candidates (50 entries total), while the
immutable preparation baseline remains 32 and 23.

Validation for this extraction is the focused `manual_latency_*` and bootstrap
probe tests, rustfmt, the Rust source-quality checker and fixture harness,
all-target Cargo checking, all-target Clippy, and `git diff --check`.

The runtime overlay capture-phase extraction moves the complete contiguous test
and fixture group from physical lines 315 through 1,793 of
`src/tests/stateful_sqlite/runtime_overlay_and_group_rule_behaviors.rs` into
`src/tests/stateful_sqlite/runtime_overlay_and_group_rule_behaviors/runtime_overlay_capture_phases.rs`.
On the verified main merge base `4490fe2a95e0225ae82705ead4f6ee40699db2a7`,
the exact boundary contains 1,479 moved lines. Runtime overlay capture,
cleanup, terminalization, account-switch, and capture persistence behavior,
test leaf names, assertions, helper access, and the `stateful_sqlite` resource
bucket remain unchanged. The parent is now 2,817 physical lines and the child
is 1,480 physical lines, both below the 3,000-line test/helper target, so the
parent is removed from the quality policy inventory. The current inventory is
28 production and 21 test/helper candidates (49 entries total), while the
immutable preparation baseline remains 32 and 23.

Validation for this extraction is the focused runtime overlay/capture-phase
tests, rustfmt, the Rust source-quality checker and fixture harness, all-target
Cargo checking, all-target Clippy, and `git diff --check`.

The error-distribution and SSE extraction moves the complete inline test module
from `src/api/slices/error_distribution_and_sse.rs` into
`src/api/slices/error_distribution_and_sse/tests.rs`, and moves the complete
dashboard realtime projection region from `BroadcastStateCache` through
`complete_dashboard_projection_publish_window` into
`src/api/slices/error_distribution_and_sse/dashboard_live_projection.rs`.
On the verified main merge base `b2db1923d5022c0a4c742650b91f5961392f211c`,
the source regions contain 1,314 and 1,063 physical lines. The resulting
parent is 2,416 physical lines, the production child is 1,065 lines, and the
test child is 1,287 lines, all within their targets. The test module path,
test names, assertions, route and serde contracts, and runtime behavior remain
unchanged; the required dashboard helper visibility is crate-scoped and the
existing parent re-export preserves callers. The parent is removed from the
quality policy inventory. The current inventory is 27 production and 21
test/helper candidates (48 entries total), while the immutable preparation
baseline remains 32 and 23.

Validation for this extraction is the focused
`cargo test error_distribution_and_sse::tests -- --nocapture` selector,
rustfmt, the Rust source-quality checker and fixture harness, all-target Cargo
checking, all-target Clippy, and `git diff --check`.

The group note CRUD extraction moves the complete contiguous handler region from
physical lines 2,043 through 2,501 of
`src/upstream_accounts/crud_group_notes.rs` into
`src/upstream_accounts/crud_group_notes/group_notes.rs`. The region contains
`update_upstream_account_group` and `delete_upstream_account_group`; the parent
re-exports both handlers with their existing crate-visible paths. After rustfmt,
the parent is 2,261 physical lines and the child is 460 physical lines, both
below the 2,500-line production target. The current inventory is 25 production
and 21 test/helper candidates (46 entries total), while the immutable
preparation baseline remains 32 and 23 and the suppression baseline remains
117. SQL, validation, response behavior, tests, and runtime behavior remain
unchanged.

Validation for this extraction is the focused group-update tests, rustfmt, the
Rust source-quality checker and fixture harness, all-target Cargo checking,
all-target Clippy, and `git diff --check`.

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
allow recorded under the child path. The current inventory is 25 production and
21 test/helper candidates (46 entries total), while the immutable preparation
remains 32 and 23 and the suppression baseline remains 117.

Validation for this extraction is the focused existing proxy cost/backfill and
prompt-cache stateful SQLite tests, the stateful SQLite backend profile,
rustfmt, the Rust source-quality checker and fixture harness, all-target Cargo
checking, all-target Clippy, and `git diff --check`.

The upstream route-binding penalty and live-candidate evaluation extraction
moves the complete contiguous region from physical lines 390 through 819
inclusive (430 moved lines) of
`src/upstream_accounts/routing/selection.rs` into
`src/upstream_accounts/routing/selection/live_candidate.rs`, on the verified
main merge base `c94f1f0a0f6ee51e0c442a6007d431c53ee7f151`. The parent retains
`LivePoolCandidateEvaluation`, scoring helpers, and resolver logic. After
rustfmt, the parent is 2,449 physical lines and the child is 432 physical
lines, both below the 2,500-line production target, so the parent is removed
from the quality policy inventory. The parent and routing module re-exports
preserve existing crate-visible call paths; function signatures, visibility,
SQL, routing behavior, and test semantics remain unchanged. The current
inventory is 24 production and 21 test/helper candidates (45 entries total),
while the immutable preparation baseline remains 32 and 23 and the
suppression baseline remains 117.

Validation for this extraction is the focused route-binding penalty and
candidate-resolution tests, rustfmt, the Rust source-quality checker and
fixture harness, all-target Cargo checking, all-target Clippy, and the
stateful SQLite backend profile.

The routing OAuth route cases move as one contiguous five-test block from
physical lines 2,374 through 3,025 of
`src/tests/stateful_sqlite/routing_timeout_and_overload_failover.rs` into
`src/tests/stateful_sqlite/routing_timeout_and_overload_failover/oauth_route_cases.rs`.
The nested module remains under the stateful SQLite test prefix. After rustfmt,
the parent is 2,487 physical lines and the child is 653 lines, both below the
3,000-line test/helper target. The parent is removed from the policy inventory.
The current inventory is 24 production and 20 test/helper candidates (44 total),
while the immutable preparation baseline remains 32 and 23 and the suppression
baseline remains 117. Test names, assertions, fixtures, timing, and behavior
remain unchanged.

The system raw-payload metrics inventory/circuit lifecycle moves as one
contiguous 630-line block from physical lines 1,049 through 1,678 of
`src/api/slices/system_routes_and_tasks.rs` into
`src/api/slices/system_routes_and_tasks/raw_payload_inventory.rs`, on verified
main base `8d0d2d1197f61776807f2e90e022776f6401e39b`. The parent retains
the preceding filesystem scanner and following status snapshot/task lifecycle.
After rustfmt, the parent is 2,364 physical lines and the child is 632 lines,
both below the 2,500-line production target. The parent is removed from the
policy inventory, leaving 23 production and 20 test/helper candidates (43
total); the immutable preparation baseline remains 32 and 23 and the
suppression baseline remains 117. The crate-visible re-export preserves call
paths, names, signatures, SQL, timing, logging, cache and circuit behavior.

The pricing catalog/settings extraction moves the complete 17-test block from
physical lines 24 through 996 of
`src/tests/stateful_sqlite/pricing_catalog_and_models_passthrough.rs` into
`src/tests/stateful_sqlite/pricing_catalog_and_models_passthrough/pricing_catalog.rs`
on verified main base `97ccccc6bd2e0e2f5dac0def8fa59d91203b2f6d`.
The parent retains the large-stack helper and later model/proxy/routing tests.
After rustfmt, the parent is 2,424 physical lines and the child is 974 lines,
both below the 3,000-line test/helper target. The parent is removed from the
policy inventory, leaving 23 production and 19 test/helper candidates (42
total); the immutable preparation baseline remains 32 and 23 and the
suppression baseline remains 117. Test names, assertions, fixtures, timing,
and stateful SQLite resource classification remain unchanged.

The provisioning-scope extraction moves the six complete contiguous tests from
physical lines 768 through 1,400 of
`src/upstream_accounts/tests/stateful_sqlite/resolver_concurrency_and_node_shunt.rs`
into
`src/upstream_accounts/tests/stateful_sqlite/resolver_concurrency_and_node_shunt/provisioning_scope.rs`
on verified main base `81163d7ff2a3b4c00e5a5193d44e08e05ed61b3f`. Both modules
remain under the stateful SQLite test prefix. The parent is removed from the
source-quality inventory, reducing the current inventory to 23 production and
18 test/helper entries (41 total); the immutable preparation baseline remains
32 and 23 and the suppression baseline remains 117. Test names, bodies,
assertions, fixtures, timing, and resource classification remain unchanged.
The parent is 2,919 physical lines and the child is 634 lines, both below the
3,000-line test/helper target.

The application runtime-state extraction moves invocation records, terminal and
projection tombstones, pruning, prompt-cache projections, and memory estimates
into `src/app_state/invocation_store.rs`, and dashboard current/network/terminal
projection state, coalescing, publication windows, baselines, captures, and
topology counters into `src/app_state/dashboard.rs`. The parent retains the
`RuntimeProjectionHub` coordinator, request-pipeline metrics, lifecycle counters,
and explicit crate-visible re-exports. Store-before-dashboard lock ordering is
preserved, no await occurs while either state lock is held, and mutation,
rollback, prune synchronization, deadline, and revision behavior remain
unchanged. The summary API slice remains frozen for this extraction.

After rustfmt, `src/app_state.rs` is 1,661 physical lines,
`src/app_state/dashboard.rs` is 1,266 lines, and
`src/app_state/invocation_store.rs` is 577 lines. All three are below the
2,500-line production target, so `src/app_state.rs` is removed from the policy
inventory. The current inventory is 22 production and 18 test/helper
candidates (40 entries total); the immutable preparation baseline remains 32
and 23 and the suppression baseline remains 117.

The prompt-cache conversation binding extraction moves operation-event snapshot
encoding and filtered operation-event reads into
`src/api/slices/prompt_cache_and_timeseries/prompt_cache_conversations/bindings/operation_log.rs`,
and encrypted-session owner/routing-policy logic into
`src/api/slices/prompt_cache_and_timeseries/prompt_cache_conversations/bindings/routing_policy.rs`.
The parent retains binding request models and validation, binding persistence,
sticky-route transaction and broadcast coordination, HTTP handlers, and
cache/subscription coordination. Explicit crate-visible re-exports preserve
existing callers; database rows and operation-log serialization details remain
private to the operation module.

The operation-event writers now share one private typed payload and one INSERT
implementation while preserving the existing nullable JSON and executor
semantics. After rustfmt, the parent is 2,277 physical lines, the
operation-log module is 1,115 lines, and the routing-policy module is 654
lines. All three are below
the 2,500-line production target, so the parent is removed from the policy
inventory. The current inventory is 21 production and 18 test/helper
candidates (39 entries total); the immutable preparation baseline remains 32
and 23 and the suppression baseline remains 119. Binding behavior, transaction
ordering, sticky-route mutation semantics, owner routing, API payloads, and
test resource classification remain unchanged.

The prompt-cache timeseries extraction separates the parent into explicit
aggregation, materialization, minute-projection, parallel-work, and query
modules, with restart recovery nested under minute projection. The parent
retains the HTTP extractor and compatibility wrappers; child modules use direct
cross-module imports and narrow crate-visible re-exports. After rustfmt, the
parent is 146 physical lines, while the six production files
`aggregation.rs`, `materialization.rs`, `minute_projection.rs`,
`parallel_work.rs`, `queries.rs`, and `recovery.rs` are 842, 487, 1,830, 288,
1,382, and 148 lines. All are below the 2,500-line target, so the parent is
removed from the policy inventory, leaving 20 production and 18 test/helper
candidates (38 total). The immutable preparation baseline remains 32 and 23,
and the suppression baseline remains 119. Query routing, aggregation
semantics, minute projection fences, materializer overlays, parallel-work
responses, and test resource buckets remain unchanged.

The subscriptions source extraction replaces the 24,445-line flat source with a real module directory. Configuration, protocol, query, topic, state, materializer, hub lifecycle, hub projection, hub recovery, and server-push responsibilities now have named Rust modules, with a separate runtime projection module keeping the production files below the 2,500-line target. The inline tests are grouped into summary, parallel-work, hot-topic, conversation, network, replay, and shared-support files below the 3,000-line test/helper target; existing topic-builder, prompt-cache, and managed-task test paths remain. The source-quality policy removes the obsolete parent entry, reducing the explicit inventory to 35 while retaining the immutable 32/23 candidate baseline and the existing suppression inventory.

## Long-Term Statistics Extraction

The flat `src/long_term_stats.rs` source is replaced by the `src/long_term_stats/` module directory. Models, archive compatibility, schema, projection runtime/rebuild, integrity, refresh, intervals, and projection flushing now have explicit module boundaries, and the inline tests are grouped by control, integrity, intervals, refresh, archive, and shared support concerns. Existing names, visibility paths, SQL, timing, assertions, resource classification, and cleanup behavior remain unchanged, with no `include!` composition.

After rustfmt, every production child is below 2,500 lines and every test/helper child is below 3,000 lines. The policy removes the obsolete parent entry and now has 34 explicit files, while immutable baseline candidate counts remain 32 production and 23 test/helper. The two moved refresh `#[expect]` declarations retain their suppression inventory entries under `src/long_term_stats/refresh.rs`.

## Archive Cleanup Extraction

The archive cleanup facade now delegates to explicit compression-backlog, deletion, historical-rollup, legacy-mirror, storage, Summary Snapshot, and test modules under `src/maintenance/archive/cleanup/`. The production coordinator, archive-file identity implementation, and `prune_archive_batches` orchestration remain in `cleanup.rs`; the 21 existing tests remain under the same `archive_cleanup::tests` module path. After rustfmt, every extracted production file is below 2,500 physical lines and the test helper is below 3,000 lines, so the obsolete parent is removed from the policy inventory. Names, SQL, timing, file-identity checks, deletion transaction boundaries, proof state, test names, and resource classification remain unchanged. The current policy is 33 entries with 16 production and 17 test/helper candidates and 125 suppression entries; immutable candidate counts remain 32 and 23.

The integration reuses mainline's existing Summary fixture repair in `src/tests/stateful_sqlite/parallel_work_stats_and_timeseries/pruned_summary_archives.rs`. The 500-day pruned dates, 440-day existing dates, and explicit different-month assertions remain unchanged; no Summary fixture relocation, date, SQL, assertion, or production behavior changes are part of this extraction's net delta.

The concurrent cold-conversation fixture also uses the existing bounded helper after its 100 ms allocation timeout reproduced in both the verified base archive and the extraction candidate. Only that fixture's allocation call changes: the helper permits at most five attempts for the exact timeout error, while retaining 32 concurrent distinct owners, unique-ID/count assertions, immediate failure for other errors, and the production 100 ms deadline. Focused diagnostic passes and prior full-profile failures remain distinct from complete candidate acceptance.

The retention fixture shares the existing exact-timeout allocation helper for one active-identity setup call, with visibility restricted to sibling modules. The integration preserves mainline's existing exact all-slots nextest group for the startup pending-repair and concurrent cold-conversation fixtures, including its other test entries. The original startup five-second watchdog, cold fixture's 32 concurrent owners and five-attempt exact-timeout bound, every business assertion, production budgets, global six-thread stateful profile, and all other source-quality entries and budgets remain unchanged.

The owner admitted two concrete follow-up candidates into the same archive-cleanup PR: seven parallel-work connection/regression paths and two missing-key fixture paths, expanding its net scope from 14 to 23 paths. The connection repair follows evidence of concurrent baseline builders retaining observer connections while awaiting another permit from the same four-connection pool. The fixture preparation follows separately decoded invocation-range initialization timeouts before missing-key terminalization; the downstream initialization-delay cause remains unproven. These are explicit behavior and fixture changes beyond the mechanical cleanup extraction, while the cleanup membership, policy inventory, business SQL, production timing, and frozen startup fixture remain preserved.

Diagnosis retained every original failure and candidate revision, including the cancelled non-Auto draft, an 8 GiB compiler OOM, the single authorized successful 12 GiB recovery, two old-source constrained RED results, 24 separate candidate passes, and a four-pass/one-failure no-template cold-fixture batch. Startup-only controls distinguish no-template cold setup from the canonical real-schema prerequisite; old production also reproduced the original watchdog failure. A separate debugger run located its own perturbed deadline in archive-usage repair and exited early after a backtrace error, so its GDB exit zero is not a test result. The admitted 23-path candidate requires fresh formal validation and current-head four-lane review; these diagnostic outcomes do not clear prior failures or establish merge readiness.

## Live Hourly Rollup Extraction

The live hourly-rollup extraction starts from `c27815edd2db9a6047b6c07d10c0c2de43bade3c`, the verified merge of PR #1138. It completes governance of the original 5,858-line `src/maintenance/hourly_rollups.rs` in one PR by extracting live repair, archive replay, pool-node-health archive backfill, and bootstrap modules while preserving the parent facade, inline test modules, and existing archive-support path. After rustfmt, the parent is 1,142 lines and the children are 1,949, 2,232, 299, and 287 lines; the canonical inventory decreases from 32 to 31 entries, with 14 production and 17 test/helper entries and unchanged 125 suppression entries. No remaining split of this parent is deferred.

## Proxy Request Entry Extraction

The proxy request-entry extraction starts from `df03c13abbc9700802e2b0ab8f37dabd40b9eed8`, the verified mainline merge of the failover source-quality refactor. It completes governance of the original 6,435-line `src/proxy/request_entry.rs` by extracting response capture, attempt lifecycle, replay-body, image-tool rewrite, and semantic-projection responsibilities into named sibling modules while retaining the parent facade and existing invocation-identity and response-overload modules. After rustfmt, the parent is 565 lines and the new children are 543, 1,542, 1,276, 1,494, and 1,050 lines; the canonical inventory decreases from 27 to 26 entries, with 9 production and 17 test/helper entries, while the 123-entry suppression inventory is preserved with the moved recovered-attempt allow recorded under its child path. Existing crate-visible call paths, request persistence, retry/replay behavior, image-tool rewriting, semantic projection, tests, and resource classification remain unchanged.

Validation for this extraction is the focused request-entry test selectors, rustfmt, the Rust source-quality checker and fixture harness, all-target Cargo checking, all-target Clippy, and `git diff --check`.

## Route Selection Extraction

The route-selection extraction starts from `e69cf51b5521fb8e6c44b8e7d4425fef688a5473`, the verified mainline merge containing the retention update. It completes governance of the original 3,787-line `src/proxy/route_selection.rs` in one PR by extracting complete semantic analysis, pool resolution, pool-forwarding orchestration, and forward-retry responsibilities into named sibling modules. After rustfmt, the parent is 25 lines and the new production children are 1,199, 582, 1,724, and 274 lines; the existing 44-line `auth.rs` boundary is retained, the canonical inventory decreases from 26 to 25 entries with 8 production and 17 test/helper candidates, and the 123-entry suppression inventory is unchanged. Crate-visible call paths, bounded replay parsing, sticky/prompt-cache/encrypted-session routing, candidate and reservation lifecycles, attempt persistence, response streaming, 429 retry, health and observability behavior, tests, and the frozen invocations module remain unchanged.

Validation for this extraction is the focused route-selection/request-entry and proxy forwarding coverage, the lightweight and stateful SQLite backend profiles, rustfmt, the source-quality checker and fixture harness, locked all-target checking, all-features Clippy, quality-gate and documentation contract checks, and `git diff --check`.

## Usage Persistence Extraction

The usage-persistence extraction replaces the 7,213-line flat source with a 25-line facade and six named production children: attempt persistence, attempt recovery, invocation records, raw-payload storage, streaming raw payload, and terminal rollups. The existing attempt-public-ID backfill, invocation-identity, runtime-recovery, and terminal-metadata module paths remain preserved, as does the frozen invocation-summary source. After rustfmt, the new children are 1,564, 1,031, 1,779, 1,283, 1,035, and 541 lines; the policy inventory decreases from 25 to 24 entries, from 8 to 7 production candidates, while the 17 test/helper entries and 123 suppression entries remain. Existing crate-visible call paths, SQL, attempt/terminal lifecycle, raw-payload retention and streaming capture behavior, tests, and resource buckets remain unchanged.
