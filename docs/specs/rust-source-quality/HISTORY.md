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
