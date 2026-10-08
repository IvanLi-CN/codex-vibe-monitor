# Prompt-Cache Live Reads Use the Working-Set Overlay During Materialization

## Status

Accepted

## Context

Prompt-cache identity and delayed aggregate statistics are materialized by an ordered background task. A non-empty refresh queue or a settling statistics marker can therefore coexist with a valid durable working-set row, in-flight runtime records, and usable request totals. Treating that state as globally unavailable made current HTTP reads and dashboard SSE baselines disappear even though the existing durable/runtime merge path could serve them.

## Decision

Current prompt-cache reads use the existing durable working-set queries, in-memory runtime overlay, and invocation identity deduplication without requiring the global materialization-complete marker. This applies to legacy live selections, the first current working-conversation page, bounded current-key hydration, and the dashboard current SSE baseline. Runtime-only and in-flight rows remain visible, while delayed aggregate fields stay absent or reflect the last materialized value rather than being replaced with zeroes.

Public reads with an explicit `snapshotAt` remain behind the materialization-complete gate, preserving the historical contract. Current paginated reads may create an internal boundary for repeatable ordering, but that boundary does not make the read historical; snapshot filtering still prevents statistics newer than the boundary from being published. The subscription baseline keeps its transaction-pinned invocation row ceiling and uses the same snapshot filtering.

The materialization scheduler, queue fairness, allocator, range reservations, and durable schema remain unchanged. Database and control-plane errors continue to propagate as errors; only the false unavailable result caused by incomplete background materialization is removed from current/live paths.

## Consequences

- Current HTTP and SSE working-conversation lists remain available during identity backfill, statistics rebuild, queue drain, and delayed-statistics settling.
- Consumers can distinguish list availability from statistics freshness through the existing optional aggregate fields; the Live and dashboard surfaces show a non-blocking freshness notice when those fields are absent or the latest terminal preview is newer than the last materialized invocation.
- Explicit historical snapshots retain fail-closed materialization behavior and must not publish post-boundary aggregate statistics.
- Existing runtime overlay identity and persisted-row deduplication remain the single merge contract; no second overlay or scheduler path is introduced.

## References

- [REQ-PII-011](../specs/proxy-invocation-identity/SPEC.md#req-pii-011)
- [ADR 0021](0021-prompt-cache-background-materialization.md)
- [ADR 0028](0028-prompt-cache-continuous-statistics-checkpoints.md)
