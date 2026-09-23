# Rust Source Quality Preparation Contract

## Context and Scope

This topic establishes an executable Rust source-quality baseline for later
module-oriented refactor pull requests. The preparation implementation adds
repository tooling and CI coverage only; it does not move production Rust code
or test modules and does not change runtime behavior.

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

The policy MUST keep the current baseline as 56 explicit file entries: 33
production candidates above 2,500 lines and 23 test/helper candidates above
3,000 lines. Each entry MUST record its exact current budget, role, and either a
specific next module workstream or a reasoned cohesive-module exception.

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

The current baseline retains no cohesive-module exceptions: all 56 entries have
specific next module workstreams. The schema and fixture harness retain the
exception form for a future entry only when its reason is explicit and
cohesive, never as an escape hatch for an unselected or growing file.

## Later Module Rollout

Later refactor PRs may split one production or test/helper candidate at a time.
Such a PR updates the affected explicit budget and workstream, removes an entry
only after the path is no longer a selected candidate, and updates suppression
coverage whenever declarations are moved or changed. This preparation PR does
not bundle those module moves.

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
Pass condition: the policy has 33 production and 23 test/helper entries, with
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
