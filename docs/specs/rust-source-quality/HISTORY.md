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
