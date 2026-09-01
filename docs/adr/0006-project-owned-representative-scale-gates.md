# ADR 0006: Project-owned representative-scale validation gates

## Status

Superseded by ADR 0007

## Context

The repository has stable backend resource-profile commands, but their execution
depends on caller-installed `cargo-nextest`, an implicit toolchain, and writable
paths that are not part of the project contract. The production-copy validation
script also checks startup status and timing without proving exact response
content, and it is not a required PR or release gate. These gaps allow a change
to pass repository checks while failing at production-shaped Summary scale.

## Decision

- The project owns a `backend-test` execution target with a pinned Rust
  toolchain, pinned and checksum-verified cargo-nextest, system dependencies,
  and an explicit writable test workspace. The existing profile runner remains
  the single command truth; Runner Isolation supplies only resource safety,
  namespacing, and cleanup.
- Affected pull requests run a deterministic, data-blind, production-shaped
  fixture and an independent exactness oracle. Trusted main/release workflows
  publish and consume immutable test-image digests; untrusted PR workflows do
  not receive package-publish permissions.
- Affected releases require a production-copy receipt bound to the commit,
  image digest, fixture contract, oracle version, and deadlines. Missing, stale,
  mismatched, timed-out, or environment-failed receipts block the affected
  release; no skip or fallback path is provided.
- Bootstrap `current` and supported rolling/calendar selections must become
  Exact-Ready within 30 seconds. `all` must become exact within 1800 seconds.
  The gate verifies Range-Local Unavailable behavior for unproven ranges and
  does not reinterpret HTTP success alone as exactness.

## Considered Options

- Install nextest in a generic Rust image at runtime: rejected because network,
  PATH, version, and cold-start behavior remain caller-dependent.
- Use only manual production validation: rejected because it cannot enforce the
  existing Summary contract at merge or release time.
- Make every PR consume a production copy: rejected because it expands data
  exposure and resource cost beyond normal PR validation.
- Add a single global Summary readiness gate: rejected because it would recreate
  the historical coupling between independent selections.

## Consequences

The project gains a reproducible, auditable test contract and a hard release
guard without changing production Summary behavior. CI gains an image-build and
receipt-validation path, and maintainers must update fixture/oracle contracts
when the projection contract changes. Production copies remain confined to the
approved testbox execution boundary.
