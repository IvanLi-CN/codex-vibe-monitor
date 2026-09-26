# Rust Source Quality Preparation Contract

## Current Implementation

The contract is implemented by the following checked-in surfaces:

- `.github/scripts/run-rust-source-quality.sh` is the canonical ordered runner.
- `.github/scripts/check_rust_source_quality.py` uses only the Python standard
  library and checks selected budgets, `include!`, and suppression inventory.
- `.github/rust-source-quality-policy.json` records the base commit
  `edb6d8624b1713619c32caa050e397f0aded79b4`, 48 current explicit file
  budgets, and 117 standalone suppression declarations. The immutable
  preparation production/test-helper counts remain 32 and 23.
- `.github/scripts/test-rust-source-quality.sh` runs the repository-local
  fixture harness without compiling fixture Rust.
- `package.json`, `.github/workflows/ci-pr.yml`, and
  `.github/workflows/ci-main.yml` delegate Rust source quality to the same
  runner while retaining the existing lint job name and check topology.

The first source-quality extraction keeps
`src/api/slices/invocations_and_summary.rs` as the parent module and moves the
contiguous invocation query foundation (`build_invocation_select_query`
through `append_invocation_order_clause`) into
`src/api/slices/invocations_and_summary/invocation_query.rs`. The parent
explicitly re-exports the symbols used by its remaining runtime-overlay,
summary, dashboard, and workflow-detail code. The parent budget is now 44,631
physical lines, down from 45,660; the 1,049-line query module remains below
the 2,500-line production target and is not a selected inventory entry.

The second extraction moves the single contiguous summary-projection lifecycle
region beginning with `summary_snapshot_bootstrap_keys` and ending with the
complete `await_summary_projection_all_time_build` definition from the parent
into `src/api/slices/invocations_and_summary/summary_projection_lifecycle.rs`.
On the verified PR2 merge base, that region is physical lines 10,259 through
12,016 inclusive (1,758 lines). It contains snapshot bootstrap and hydration,
refresh routing, coverage-recovery maintenance, source-tail restoration,
durable publication coordination, and all-time build deadline handling. The
parent explicitly re-exports `hydrate_summary_snapshots`,
`hydrate_summary_snapshots_with_deadline`, `refresh_summary_snapshots`,
`SummaryCoverageRecoverySupervisor`,
`spawn_summary_coverage_recovery_maintenance`, and
`refresh_summary_snapshots_with_mode`, while preserving the existing test
seams through crate-visible test-only re-exports. The parent budget is now
42,897 physical lines, down from 44,631; the 1,761-line lifecycle module is
below the 2,500-line production target and is not a selected inventory entry.
The remaining parent workstream is limited to the summary-projection
models/builders and the named dashboard activity/network, summary/history/
suggestions/stats responsibilities recorded in the policy.

The PR4 source-quality extraction moves the contiguous workflow-detail
region beginning with the complete `fetch_invocation_pool_attempts`
definition and ending with the complete `fetch_invocation_request_body`
definition from `src/api/slices/invocations_and_summary.rs` into
`src/api/slices/invocations_and_summary/invocation_workflow_detail.rs`. On the
verified PR3 merge base `29e441c6e01ddda38769b4d3a82c58a36db1154d`, the exact
boundary is physical lines 2,992 through 5,421 inclusive (2,430 moved lines).
It contains pool-attempt retrieval, workflow identity/attempt/detail response
models, hero/timeline construction, upstream-account attempt hydration,
workflow-detail reads, request/response body row queries, raw-body fallback and
summary construction, record detail, and the related handlers. The child is
2,436 physical lines after the crate-visible compatibility adjustments and
remains below the 2,500-line production target, so it is not an inventory
entry. The parent budget decreases from 42,897 to exactly 40,532 physical
lines. The parent keeps explicit crate-visible re-exports for routes,
subscriptions, sibling slices, and the existing inline tests; no tests or
resource buckets move.

The fourth source-quality extraction moves the contiguous Dashboard Activity
snapshot-cache state and invalidation region beginning with
`DashboardActivitySnapshotSelection` and ending with
`invalidate_dashboard_activity_snapshots_with_accounts` from
`src/api/slices/settings_models_and_cache.rs` into
`src/api/slices/settings_models_and_cache/dashboard_activity_cache.rs`. The
child preserves the cache models, read model, singleflight guard, memory
estimate, selection fingerprint, and selection/account invalidation behavior;
the parent explicitly re-exports the crate-visible symbols used by the
invocation-summary, subscriptions, prompt-cache/timeseries, runtime, memory,
SQLite writer, account-routing, and stateful SQLite test callers. The parent
is now 2,293 physical lines and the child is 279 physical lines, so the child
is below the 2,500-line production target and is not a selected inventory
entry. The parent candidate is removed from the policy inventory; no further
source-quality extraction is planned from this parent.

The OAuth bridge extraction moves the complete contiguous `#[cfg(test)] mod tests` block from `src/oauth_bridge.rs` into
`src/oauth_bridge/tests.rs`. The parent keeps the same
`#[cfg(test)] mod tests;` declaration, so the test module path, private-item
access, test names, and assertions remain unchanged. The parent is 2,040
physical lines and the test helper is 720 physical lines; both are below their
respective targets, so `src/oauth_bridge.rs` is removed from the policy
inventory.

The service-tier backfill extraction moves the complete contiguous test group
from the verified baseline parent lines 34 through 193 inclusive into
`src/tests/stateful_sqlite/proxy_backfill_and_cost_repairs/service_tier_backfill.rs`.
The group contains the three service-tier backfill tests and keeps their names,
bodies, assertions, fixtures, thresholds, and `stateful_sqlite` resource bucket
unchanged. The parent is now 2,948 physical lines and the child is 162 physical
lines, so both are below the 3,000-line test/helper target. The parent is
removed from the policy inventory; no production runtime behavior changes.

The upstream routing status persistence extraction moves the complete
contiguous production region beginning with `set_account_status` and ending
with the complete `record_classified_account_sync_failure_with_proxy_snapshot`
definition from `src/upstream_accounts/sync_routing_status.rs` into
`src/upstream_accounts/sync_routing_status/status_persistence.rs`. On the
verified main merge base `3d06762198c3abff18806fce8ff65a05030a2d32`, the exact
boundary is physical lines 2,038 through 2,576 inclusive (539 moved lines).
It contains status writes, sync success, suppressed/recovery-blocked and
hard-unavailable status events, failure persistence, and classified failure
recording. The parent explicitly re-exports the crate-visible persistence
adapters used by `sync.rs`, sibling sync slices, and existing tests. The parent
is now 2,154 physical lines and the 541-line child remains below the 2,500-line
production target, so the parent is removed from the policy inventory. The
existing `account_action_event_tests` module remains in the parent; no test
names or resource buckets moved.

Validation for this extraction is the focused sync/status and stateful SQLite
cooldown coverage, rustfmt, the Rust source-quality checker and fixture
harness, all-target Cargo checking, all-target Clippy, and `git diff --check`.

The request-prefix extraction moves the complete contiguous region beginning
with `best_effort_extract_json_string_for_patterns` and ending with
`prepare_target_request_body_with_hosted_intent` from
`src/proxy/stream_gate.rs` into
`src/proxy/stream_gate/request_prefix.rs`. On the verified main merge base
`52b82d70a76f63682ee27d502b070d7bdd0903c6`, the exact boundary is physical
lines 34 through 441 inclusive (408 moved lines). It contains the bounded JSON
prefix extractors, encrypted-content detection, request-prefix tests, and
request-body preparation. The parent is now 2,338 physical lines and the
410-line child remains below the 2,500-line production target, so the parent is
removed from the policy inventory. The parent explicitly re-exports the child
symbols, while existing test names, resource buckets, suppression paths, and
runtime behavior remain unchanged.

The forward-proxy probe and validation extraction moves the complete
contiguous region beginning with `parse_forward_proxy_nodes_latency_test_keys`
and ending with `complete spawn_forward_proxy_bootstrap_probe_round` from
`src/forward_proxy/slices/storage_and_hourly_stats.rs` into
`src/forward_proxy/slices/storage_and_hourly_stats/probe_and_validation.rs`.
On the verified main merge base `cdb7fbfa85e460e5fa666aa66834c9fc52745b8e`,
the exact boundary is physical lines 2,401 through 3,842 inclusive (1,442
moved lines). It contains manual latency probes, candidate and subscription
validation, endpoint probing, and bootstrap probe scheduling. The parent is
now 2,404 physical lines and the 1,444-line child remains below the 2,500-line
production target, so the parent is removed from the policy inventory. The
parent uses an explicit path declaration and crate-visible re-export, while
routes, signatures, serde behavior, tests, resource buckets, and runtime
behavior remain unchanged.

The runtime overlay capture-phase extraction moves the complete contiguous test
and fixture group from physical lines 315 through 1,793 of
`src/tests/stateful_sqlite/runtime_overlay_and_group_rule_behaviors.rs` into
`src/tests/stateful_sqlite/runtime_overlay_and_group_rule_behaviors/runtime_overlay_capture_phases.rs`.
On the verified main merge base `4490fe2a95e0225ae82705ead4f6ee40699db2a7`,
the exact boundary contains 1,479 moved lines, including runtime overlay
capture, cleanup, terminalization, account-switch, and capture persistence
coverage plus its fixture upstreams. The parent is now 2,817 physical lines
and the 1,480-line child is below the 3,000-line test/helper target, so the
parent is removed from the policy inventory. The existing top-level helpers
remain in the parent and are available to the child through `use super::*`;
test names, assertions, resource bucket, helper access, and runtime behavior
remain unchanged.

Validation for this extraction is the focused runtime overlay/capture-phase
tests, rustfmt, the Rust source-quality checker and fixture harness, all-target
Cargo checking, all-target Clippy, and `git diff --check`.

The error-distribution and SSE extraction moves two complete cohesive regions
from `src/api/slices/error_distribution_and_sse.rs`: the inline `#[cfg(test)] mod tests` block into `src/api/slices/error_distribution_and_sse/tests.rs`, and
the dashboard realtime projection state/model/build/scheduler region from
`BroadcastStateCache` through `complete_dashboard_projection_publish_window`
into `src/api/slices/error_distribution_and_sse/dashboard_live_projection.rs`.
On the verified main merge base `b2db1923d5022c0a4c742650b91f5961392f211c`,
the approved source regions were 1,314 and 1,063 physical lines. After
rustfmt and the narrow crate-visible helper re-exports, the parent is 2,414
physical lines, the production child is 1,065 lines, and the test child is
1,286 lines. The out-of-line test module keeps its path, names, assertions,
and test behavior; the parent retains the existing crate-visible API through
the dashboard child re-export. Routes, serde contracts, visibility outside
the required helper adjustment, and runtime behavior remain unchanged. The
parent is removed from the policy inventory and no future source-quality PR
may split this parent again.

Validation for this extraction is the focused
`cargo test error_distribution_and_sse::tests -- --nocapture` selector,
rustfmt, the Rust source-quality checker and fixture harness, all-target
Cargo checking, all-target Clippy, and `git diff --check`.

## Inventory Contract

The policy has 27 `production` entries above the 2,500-line destination target
and 21 `test_helper` entries above the 3,000-line destination target. Each
`line_budget` is the exact current physical line count from the verified base.
The checker only reads those 48 paths; a long path absent from the inventory is
not rejected by a global threshold.

Every current entry has a concrete next module workstream. There are no
retained cohesive-module exceptions in this baseline. If a later change needs
one, it must add the explicit exception object and its narrow reason, and the
fixture contract will continue to reject an empty reason.

Suppression matching is exact after whitespace normalization and uses a
multiset, so duplicate declarations remain distinguishable. Moving a
declaration without changing its path or content does not create churn; adding,
removing, or materially changing one does.

## CI Contract

The existing lint jobs still provision rustfmt and Clippy, restore the same
Cargo caches, and retain their names. Their Rust check step calls the canonical
runner, which executes:

1. `cargo fmt --all -- --check`
2. `cargo clippy --locked --all-targets --all-features -- -D warnings`
3. `python3 .github/scripts/check_rust_source_quality.py ...`

No global Clippy pedantic configuration or new dependency is introduced.

## Verification

- `bash .github/scripts/test-rust-source-quality.sh`
- `bash .github/scripts/run-rust-source-quality.sh`
- `bun run verify:rust`
- `bash .github/scripts/test-quality-gates-contract.sh`
- Focused service-tier backfill, summary-projection, and stateful SQLite coverage,
  including `cargo test backfill_invocation_service_tiers -- --nocapture`, and
  focused workflow-detail coverage, including
  `cargo test workflow_usage_audit_only_attaches_to_last_success_like_attempt -- --nocapture`,
  followed by `bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite`
- Focused `manual_latency_*` and forward-proxy bootstrap probe tests
- `cargo check --locked --all-targets --all-features`
- `git diff --check`

## Out of Scope

No additional production runtime changes, public protocol changes, persistence
changes, release behavior, Web changes, Docker execution, or runtime acceptance
evidence belong to this implementation state.
