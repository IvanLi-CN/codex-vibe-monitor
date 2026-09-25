# ADR 0018: Bounded Raw Orphan Sweep Worker

## Status

Accepted

Supersedes [ADR 0017: Historical Raw-File Reconciliation Protocol](./0017-historical-raw-file-reconciliation.md).

## Context

The raw reconciliation batch retained at most 32 candidates, but finding them still traversed the entire flat raw directory on every pass. Candidate reference checks also queried live owner tables in addition to the indexed raw-link ledger. The sweep ran as a stage of hourly retention maintenance, so failures and large backlogs could make little progress while holding repeated maintenance work.

## Decision

- Run raw orphan reconciliation as one independently scheduled worker per service process. The worker obtains maintenance admission before opening or advancing the raw directory iterator. A denied admission performs no raw-directory I/O and persists a five-minute retry. For a bounded directory-read slice, retain the background pressure slot but release the SQLite write-coordinator permit, preventing another background task from entering without delaying foreground writes. While admission is held for final candidate release, acquire the raw-directory lock without blocking; contention defers that candidate and releases admission rather than delaying foreground writes behind another process.
- Keep one `std::fs::ReadDir` iterator in process memory across slices. A slice advances at most 128 directory entries and processes at most 32 supported direct regular files. After a slice with progress and no failure, resume after one second. At EOF, close the iterator and begin a new pass after five minutes. After process restart, start from the beginning and rely on the durable per-file ledger for idempotency; do not treat a filename cursor as an OS directory seek position.
- Confirm candidate ownership only through `proxy_raw_payload_blob_links(raw_path)` and its path index. If the legacy-link seed completion marker is absent, fail closed. Evaluate existing absolute/relative and compressed-path fallback precedence in memory, and repeat the indexed ownership check under maintenance admission immediately before unlink.
- Preserve the 24-hour same-identity quarantine, final metadata/reference checks, file lock, and inventory-reset intent before physical deletion. Continue later candidates after an item failure and persist a sanitized fingerprint. A busy file lock is an item failure, not a blocking wait under maintenance admission. Back off a failed slice for 5, 10, 20, 40, then 60 minutes; keep other retention stages independent.
- Reuse `retention_recovery_cursors.raw_payload_files` retry/progress columns. Its existing `cursor` stores a keyset position only while rotating bounded missing-ledger cleanup; it does not checkpoint directory traversal. Earlier binaries may interpret that string as a filename cursor, but their bounded traversal wraps and remains safe.
- Expose low-cardinality sweep state and per-slice counters in the optional System Status `rawOrphanSweep` object.

## Consequences

Each pass performs bounded iterator work instead of rescanning the entire directory for a 32-file selection. Restart may repeat entries from the beginning, and newly created files may wait until the next pass; the identity ledger makes both cases safe. The filesystem iterator is process-local, so forward recovery does not depend on a new schema migration. The indexed link ledger is authoritative only after its existing seed marker proves the legacy rows were represented.

## Alternatives Rejected

- Keeping the bounded heap while walking the entire directory was rejected because retained memory is bounded but per-pass I/O still grows with the directory.
- Persisting a guessed lexical directory position was rejected because filesystem enumeration order is not a portable seek contract.
- Rechecking owner tables for every candidate was rejected because it repeats broad work under SQLite maintenance pressure and bypasses the indexed ownership ledger.
- Deleting based on inventory state or file age alone was rejected because neither proves that a live raw owner cannot resolve to the file.
