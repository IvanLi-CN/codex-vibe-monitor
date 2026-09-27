# ADR 0018: Bounded Raw Orphan Sweep Worker

## Status

Accepted

Supersedes [ADR 0017: Historical Raw-File Reconciliation Protocol](./0017-historical-raw-file-reconciliation.md).

## Context

The raw reconciliation batch retained at most 32 candidates, but finding them still traversed the entire flat raw directory on every pass. Candidate reference checks also queried live owner tables in addition to the indexed raw-link ledger. The sweep ran as a stage of hourly retention maintenance, so failures and large backlogs could make little progress while holding repeated maintenance work.

## Decision

- Run raw orphan reconciliation as one independently scheduled worker per service process. Before any raw-root or candidate filesystem inspection, including reconciliation-ledger metadata and fallback-path checks, reserve the background pressure slot with bounded FIFO priority wait of at most 15 seconds, then obtain maintenance write admission with a separate 15-second bound. Record only the fixed stage/cause pair for an unsuccessful admission; perform no raw filesystem I/O and persist a five-minute retry. For bounded filesystem-only slices, retain the background pressure slot but release the SQLite write-coordinator permit, preventing another background task from entering without delaying foreground writes. While admission is held for final candidate release, acquire the raw-directory lock without blocking; contention defers that candidate and releases admission rather than delaying foreground writes behind another process.
- Keep one `std::fs::ReadDir` iterator and an in-memory pending candidate queue across slices. A slice advances at most 128 directory entries, processes at most 32 supported direct regular files, and spends at most two seconds on candidate processing. After a slice with progress and no failure, resume after one second; incomplete work, including a mid-pass admission defer, resumes before opening or advancing the directory iterator. At EOF, close the iterator and begin a new pass after five minutes. After process restart, start from the beginning and rely on the durable per-file ledger for idempotency; do not treat a filename cursor as an OS directory seek position.
- Confirm candidate ownership only through `proxy_raw_payload_blob_links(raw_path)` and its path index. If the legacy-link seed completion marker is absent, fail closed. Evaluate existing absolute/relative and compressed-path fallback precedence in memory, and repeat the indexed ownership check under maintenance admission immediately before unlink.
- Preserve the 24-hour same-identity quarantine, final metadata/reference checks, file lock, and inventory-reset intent before physical deletion. Continue later candidates after an item failure and persist a sanitized fingerprint. A busy file lock is an item failure, not a blocking wait under maintenance admission. Back off a failed slice for 5, 10, 20, 40, then 60 minutes; keep other retention stages independent.
- Reuse `retention_recovery_cursors.raw_payload_files` for retry/progress and add nullable admission stage/cause, settled-pass, and nonzero-removal evidence columns idempotently. Its existing `cursor` stores a keyset position only while rotating bounded missing-ledger cleanup; it does not checkpoint directory traversal. Earlier binaries may interpret that string as a filename cursor, but their bounded traversal wraps and remains safe; missing evidence remains unknown.
- Expose low-cardinality sweep state and per-slice counters in the optional System Status `rawOrphanSweep` object, keeping current state separate from the most recent settled pass and most recent nonzero removal. Successful unlink count/bytes are recorded before ledger cleanup so a cleanup failure cannot erase deletion evidence.

## Consequences

Each pass performs bounded iterator work instead of rescanning the entire directory for a 32-file selection. Restart may repeat entries from the beginning, and newly created files may wait until the next pass; the identity ledger makes both cases safe. The filesystem iterator is process-local, while the additive nullable cursor evidence preserves admission and settled progress across restart without a startup-wide backfill. The indexed link ledger is authoritative only after its existing seed marker proves the legacy rows were represented.

## Alternatives Rejected

- Keeping the bounded heap while walking the entire directory was rejected because retained memory is bounded but per-pass I/O still grows with the directory.
- Persisting a guessed lexical directory position was rejected because filesystem enumeration order is not a portable seek contract.
- Rechecking owner tables for every candidate was rejected because it repeats broad work under SQLite maintenance pressure and bypasses the indexed ownership ledger.
- Deleting based on inventory state or file age alone was rejected because neither proves that a live raw owner cannot resolve to the file.
