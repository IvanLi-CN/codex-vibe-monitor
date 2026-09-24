# Rust Source Quality Preparation Contract

## Lifecycle

- The topic begins with an executable preparation contract rather than a
  production-module refactor.
- The current policy is anchored to the verified mainline baseline and keeps
  its 56 large-file entries explicit.
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
720 physical lines, both below their targets. The current inventory therefore
has 31 production and 23 test/helper candidates (54 entries total), while
the immutable preparation baseline remains 32 and 23. No production OAuth
bridge logic, test names, assertions, or public behavior changed.

Validation for this extraction is the focused Dashboard Activity cache tests,
both lightweight and stateful SQLite backend profiles, rustfmt, all-target
Cargo checking, all-target Clippy, the Rust source-quality runner, and
`git diff --check`.
