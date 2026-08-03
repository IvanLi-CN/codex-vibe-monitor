# Sticky Route CAS and Causal Audit

## Context

Conversation Sticky routing can receive multiple in-flight attempts that were selected from the same empty or stale route state. A completion-time overwrite turns completion order into routing policy and leaves an operation timeline that cannot explain why the target moved.

## Resolution

- Use one persisted monotonic generation as the optimistic concurrency token for every Sticky create, target replacement, and removal.
- Capture that generation during route selection and compare it inside the SQLite writer transaction before mutating the Sticky row.
- Advance the generation only for an actual target change; same-target keepalives remain no-op route mutations.
- Preserve request delivery and account-health handling when a completion loses the compare-and-swap. Suppress only the Sticky mutation and append an audit event.
- Make automatic clear conditional on both the captured generation and failed account still owning the Sticky row, then advance the generation in the same transaction when it removes that row.
- Persist structured causal evidence in operation events: reason code, routing source, status, and public attempt IDs. For fresh assignment, persist the immutable candidate-decision snapshot on the attempt before dispatch and copy it to the Sticky event: selected account, eligible count, first decisive comparator, and bounded normalized exclusions. Keep raw upstream messages in the existing protected attempt detail surface.

## Verification

- Exercise two different fresh targets with the same captured generation and complete the intended winner first.
- Exercise a delayed failure after the target changes and prove that it cannot clear the replacement.
- Verify the event API and UI show safe reason text plus Records links for cause and trigger attempts, and that a fresh assignment link opens the same candidate-decision snapshot shown on the event. Historical rows remain explicitly unannotated.
