# Rust Source Quality Preparation Contract

## Lifecycle

- The topic begins with an executable preparation contract rather than a
  production-module refactor.
- The current policy is anchored to the verified mainline baseline and keeps
  its 50 large-file entries explicit.
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
