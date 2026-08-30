# ADR 0005: Staged Summary projection recovery

## Status

Accepted

## Context

Summary HTTP and SSE correctly serve only an immutable in-memory Projection, but
the former startup path required one full-history build to finish before it
published any Projection. A large archive manifest or all-time aggregate could
therefore exceed the startup deadline and leave every otherwise independent
current and rolling request unavailable. Extending that deadline would leave
startup recovery unbounded and would not make interruption or restart progress
recoverable.

## Decision

- Build and atomically publish a Bootstrap Projection from a stable generation
  fence containing the exact legal `current` prefix and every exact rolling or
  calendar selection. Bootstrap does not enter all-time archive reconciliation.
- Keep `all` unavailable until its independent exact proof and aggregate are
  complete. No selection returns partial, approximate, or request-time rebuilt
  data.
- Persist a global and account all-time coverage checkpoint with the manifest
  high-watermark, seek cursor, and completion state. A worker advances one
  bounded manifest page only after its materialization and replay proof is
  complete. A changed high-watermark invalidates the previous generation.
- Finalize and atomically swap all-time responses only after their checkpointed
  coverage is complete. A deadline, pressure defer, or finalization failure
  retains the current Bootstrap or last-good Projection.
- Keep the resident preview-byte budget, source-admission limits, HTTP/SSE wire
  shape, and request-time zero SQLite/archive/file I/O contract unchanged.

## Considered Options

- Increase the single startup deadline. Rejected because it keeps all legal
  recent requests coupled to a full-history task and loses interruption
  progress.
- Publish a partial or zero all-time response. Rejected because it violates the
  exact Summary contract.
- Query SQLite or archive sources when an unready selection is requested.
  Rejected because it moves unbounded recovery work into the hot path.

## Consequences

- Recent operational Summary views regain exact availability without waiting for
  historical convergence.
- `all` can remain exact-unavailable while its durable recovery advances; this
  is intentional and selection-local.
- The system gains a small durable checkpoint table and bounded background
  telemetry, while final all-time publication remains atomic.
