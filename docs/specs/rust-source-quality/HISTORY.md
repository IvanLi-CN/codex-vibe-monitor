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

The invocation query foundation is now a named child module under the existing
invocations-and-summary parent. Query projection, filter parsing and SQL
construction, request and snapshot types, snapshot resolution, and ordering
remain behaviorally unchanged while the selected parent budget decreases from
45,660 to 44,631 physical lines. The new 1,049-line production file remains
below the inventory target. Validation evidence for this extraction is the
focused stateful SQLite invocation-query coverage, the stateful SQLite backend
profile, the Rust source-quality runner, all-target Cargo checking, and
`git diff --check`.
