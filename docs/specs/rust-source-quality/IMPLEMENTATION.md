# Rust Source Quality Preparation Contract

## Current Implementation

The contract is implemented by the following checked-in surfaces:

- `.github/scripts/run-rust-source-quality.sh` is the canonical ordered runner.
- `.github/scripts/check_rust_source_quality.py` uses only the Python standard
  library and checks selected budgets, `include!`, and suppression inventory.
- `.github/rust-source-quality-policy.json` records the base commit
  `edb6d8624b1713619c32caa050e397f0aded79b4`, 56 explicit file budgets, and
  117 standalone suppression declarations.
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

## Inventory Contract

The policy has 33 `production` entries above the 2,500-line destination target
and 23 `test_helper` entries above the 3,000-line destination target. Each
`line_budget` is the exact current physical line count from the verified base.
The checker only reads those 56 paths; a long path absent from the inventory is
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
- `git diff --check`

## Out of Scope

No Rust production/test module moves, public protocol changes, persistence
changes, release behavior, Web changes, Docker execution, or runtime acceptance
evidence belong to this implementation state.
