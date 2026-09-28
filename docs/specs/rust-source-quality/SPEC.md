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

The policy MUST keep the current inventory as 41 explicit file entries: 23
production candidates above 2,500 lines and 18 test/helper candidates above
3,000 lines. The immutable preparation baseline retains its original candidate
counts for checker compatibility. Each current entry MUST record its exact
current budget, role, and either a specific next module workstream or a
reasoned cohesive-module exception.

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

## Policy Shape

The checked-in policy contains `schema_version`, baseline counts and commit,
destination `loc_targets`, explicit `files`, and explicit `suppressions`.
Each file has `path`, `role`, and `line_budget`, followed by exactly one of
`next_module_workstream` or `cohesive_exception.reason`. Suppression entries
have `path`, `kind`, normalized `declaration`, and a narrow `reason`.

The current inventory retains no cohesive-module exceptions: all 41 entries have
specific next module workstreams. The schema and fixture harness retain the
exception form for a future entry only when its reason is explicit and
cohesive, never as an escape hatch for an unselected or growing file.

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

## Later Module Rollout

Refactor PRs may split one production or test/helper candidate at a time. Such
a PR updates the affected explicit budget and workstream, removes an entry only
after the path is no longer a selected candidate, and updates suppression
coverage whenever declarations are moved or changed. Completed extractions are
recorded in the current implementation and history sections of this topic.

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
Pass condition: the policy has 23 production and 18 test/helper entries, with
exact budgets and explicit workstreams or reasoned exceptions.

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

No UI evidence or empirical runtime acceptance is applicable; all checks are
deterministic repository-local checks.

## Related ADRs

None

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
- `../../../.github/rust-source-quality-policy.json`
