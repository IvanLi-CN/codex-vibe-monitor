# ADR 0017: Historical Raw-File Reconciliation Protocol

- Status: Accepted
- Date: 2026-09-23

## Context

The raw capture circuit protects current availability, but older releases can leave raw payload files that no current row references. A complete reference snapshot plus an unbounded directory walk is not a safe maintenance protocol: it can consume unbounded memory, repeat work after interruption, and delete a file while its identity or fallback role is no longer understood.

## Decision

Historical raw-file cleanup uses an independent, durable reconciliation protocol:

- The resolved raw root is the only filesystem scope. Each bounded pass selects at most 32 direct regular files through a fixed-size heap and persists a `raw_payload_files` cursor. Subdirectories, spool contents, symlinks, unknown file names, and paths outside the root are retained.
- Supported candidates are raw payload suffixes `.bin`, `.bin.gz`, and `.bin.zst`. The SQLite ledger `retention_raw_reconciliation` stores the normalized path, a stable metadata identity, byte size, and the first durable quarantine timestamp. An identity change replaces the ledger observation and restarts quarantine.
- Quarantine is logical and does not rename or move a file. This preserves existing readers and owner paths while the candidate waits. A process restart repeats the bounded cursor pass and resumes from the ledger.
- Before release, maintenance checks the candidate identity and size again, checks the link table and live owner columns, and evaluates `.bin`/`.bin.gz` fallback paths. A referenced primary path or a missing primary whose fallback is the candidate keeps the candidate. The checks are repeated immediately before deletion under the existing maintenance admission and directory lock.
- Physical deletion happens before the ledger row is cleared. A crash after deletion therefore leaves only a stale recoverable ledger row; a later bounded cleanup removes that row after confirming the path is gone. A database or filesystem failure keeps the candidate and reports the stage failure.
- Dry runs do not create ledger state or mutate files. Successful releases trigger the existing physical-inventory reset; System Status field semantics remain unchanged.

## Consequences

This protocol adds two bounded durable surfaces: a raw reconciliation cursor and a per-candidate quarantine ledger. It can take at least one additional maintenance pass before an old residual is released, but it makes the release decision auditable and restart-safe. Files that cannot prove supported identity, owner absence, or quarantine expiry remain available for the circuit-breaker safety boundary rather than being guessed away.

## Alternatives Rejected

- Enumerating every raw file and materializing every database reference in memory was rejected because the work and retained state are unbounded.
- Renaming candidates into a quarantine directory was rejected because existing raw-path readers use the original path and a rename would create a new availability failure.
- Deleting based only on file age or only on the blob-link table was rejected because either signal can be stale or can miss a fallback path during a partial migration.
