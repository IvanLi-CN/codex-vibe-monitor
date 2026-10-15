---
status: accepted
---

# ADR 0039: Terminal journal startup recovery

Supersedes [ADR 0037](0037-isolated-supplemental-recovery-and-continuous-attempt-reads.md). Its bounded runtime, routing projection, generation fences, terminal read visibility, supplemental recovery and materialization decisions remain in effect; this decision changes main terminal journal verification and historical damage handling.

## Context

A deployed legacy terminal frame contains the valid JSON number `0.009548800000000001`. Its original-byte checksum is correct, but parsing the number and serializing it again can change its representation. Treating that difference as corruption prevented the service from starting. A single historical damaged frame also must not discard later valid events or turn an otherwise writable journal into a startup failure.

## Decision

Verify checksums over the original JSON values covered by the writer, before decoding business records. Recognize the existing legacy `(entry, acknowledged_sequence)` checksum and version-one `(event_version, entry, attempt_start, acknowledged_sequence)` checksum without changing the emitted format. Use round-trip-safe float decoding to retain the original business values during replay.

At startup, warn and ignore an incomplete final frame; warn and isolate malformed, invalid-shape, or checksum-invalid complete frames while retaining all other valid frames in source order. Before replacing any damaged segment, durably copy the complete original to a unique evidence file, sync a cleaned temporary file, atomically replace the active path, and sync the containing directory. A stopped cleanup can be re-entered from the original active segment or the fully published cleaned segment. Never rename the only recoverable source out of the active namespace before publication. Sequence allocation includes verified ACK watermarks, so removing a damaged event cannot make an old ACK acknowledge a newly reused sequence.

Unsupported event versions remain a compatibility error that preserves the source rather than being classified as damaged data. Failures to read the primary source, preserve evidence, or durably publish a writable active journal remain errors: new proxy events must never be accepted with memory-only durability. New begin, terminal and ACK append/group-sync failures retain their existing admission fences. Historical frame isolation does not weaken these live-write guarantees.

An isolated frame is not declared materialized or acknowledged. Existing valid ACKs retain their meaning; an unacknowledged valid record is replayed through the existing idempotent SQLite materializer. Isolated evidence is retained for an explicit forward repair and is never automatically restored over active files or interpreted as a backup rollback.

## Consequences

Valid legacy floats cannot trigger startup rejection. A torn tail or an isolated bad historical frame no longer prevents startup when evidence preservation and active-journal publication succeed. Real loss of the writable durable sink still blocks new acceptance. Warning logs expose only path, line number, byte counts and static failure categories, never record contents.

No SQLite migration, new event version, HTTP/SSE change, retry change or capacity increase is required. Compatibility and recovery tests use synthetic records; production payloads are excluded from fixtures.
