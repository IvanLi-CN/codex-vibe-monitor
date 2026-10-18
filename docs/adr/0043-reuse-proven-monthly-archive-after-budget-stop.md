# Reuse Proven Monthly Archives After Budget Stop

Status: Accepted

## Context

The existing 120-second retention budget can expire after the canonical monthly gzip has been published and its V2 Summary pages have passed semantic validation, but before the first source-conversion transaction commits. Rebuilding the same month on the next run repeats the dominant file preparation cost while leaving every source row live. A zero-commit run can therefore repeat indefinitely even though the published artifact is complete enough to prove the selected source identity.

ADR 0042 permits durable V2 page progress to be reused, but page progress alone does not make the published gzip reusable. The proof marker was previously written as part of the first source-conversion transaction, so a run that stopped before that transaction had no durable certificate for the already published artifact.

## Decision

- After complete V2 Summary validation, persist the final proof in a short retention-admitted transaction while the matching manifest remains `materializing`. This proof is recovery evidence only; `materializing` continues to be excluded from Summary and cleanup authority.
- A later task run may reuse the canonical final gzip only when the matching `materializing` manifest, non-empty archive SHA, durable final proof marker, manifest coverage matching that proof, the decompressed artifact's exact row count and selected source ID set, deterministic keyset coverage, and current live-source identity all validate together. The source-conversion path rechecks the file digest while holding the archive fence before it commits any deletion.
- Reuse never resumes source candidate IDs, temporary SQLite files, cursors, accumulated values, or a source-deletion transaction. Missing or invalid proof, manifest, file, coverage, or source identity falls back to safe rebuild or deferral.
- Budget exhaustion while inflating the identity-proof copy is a normal partial stop, not a proof failure; the incomplete copy is cleaned up and the next run may safely rebuild or retry.
- Only the existing source-conversion commit may advance the manifest to `completed` and grant the established deletion and cleanup authority. A stale writer may not demote a completed manifest.
- No schema, archive layout, page format, retention budget, or Summary reader contract changes.

## Consequences

A budget stop after publication and proof now leaves a durable, identity-bound recovery point, allowing the next run to spend its budget on source conversion instead of recompressing the same month. Source rows remain live until the normal proof-and-delete transaction commits, and any identity mismatch remains fail-safe.

The database can contain a durable proof for a `materializing` manifest. Readers and cleanup continue to require `completed`, so this additional recovery state does not expose an incomplete archive as authoritative. The implementation and regression test must cover proof persistence after a zero-commit stop, rejection after live-source or archive-content identity changes, exact archive ID-set validation, and budget-bounded proof-copy cleanup.

## References

- [Monthly Summary page checkpoints](0042-monthly-summary-page-checkpoints.md)
- [Bounded retention runs](../specs/bounded-retention-runs/SPEC.md)
- `src/maintenance/retention/batch_plan.rs`
- `src/maintenance/retention/task_batches.rs`
