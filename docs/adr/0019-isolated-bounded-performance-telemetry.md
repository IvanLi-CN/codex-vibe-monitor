# Isolated and Bounded Performance Telemetry

Status: Superseded by [ADR 0025](0025-external-performance-observability.md)

This records the previous local telemetry architecture. ADR 0025 owns the accepted replacement design; the existing runtime remains on this implementation until its controlled cutover completes.

Runtime system performance trends live in a separate local SQLite database, written by a bounded, low-priority aggregator. The proxy, terminal ACK, current Dashboard projection, and main SQLite write coordinator never wait for it. Per-invocation proxy stages, results, retries, and bytes remain in the business database as the source for request-level investigation; the telemetry database records system overhead and cross-layer backpressure that the business records do not provide. The decision trades unrestricted diagnostic dimensions for predictable collection cost: recent data keeps one-minute buckets for seven days and five-minute buckets through day 30; selected core metrics keep hourly buckets through 13 months. Dimensions are fixed, low-cardinality function and layer categories, with no persistent account or model series. Missing collection remains visible as missing coverage rather than zero. This preserves enough temporal detail to relate queueing, SQLite pressure, memory, maintenance progress, projection/SSE health, and browser experience without reproducing request events in the observability store.

## Consequences

Collection reads existing counters and bounded process metadata; it cannot scan the main database, raw payloads or archives. Rollups merge counts and distributions before deleting finer buckets. Any later account/model breakdown or longer fine-grained retention needs an explicit capacity and overhead review. The existing invocation records remain the source for individual request investigation.
