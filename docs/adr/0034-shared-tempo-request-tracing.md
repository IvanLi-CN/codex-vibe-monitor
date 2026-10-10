# Shared Tempo for Request Diagnostic Traces

Status: Accepted

The owner selected Grafana Tempo as the shared request diagnostic trace backend permitted by [ADR 0033](0033-external-request-diagnostic-traces.md), with codex-vibe-monitor as its first client. OpenTelemetry instrumentation and OTLP export keep application collection independent of the storage product, while native Grafana integration and TraceQL provide a common investigation workflow across projects. This prioritizes reusable platform operations and flexible stage queries over the fewer storage dependencies of a project-local Jaeger and Badger deployment.

## Consequences

- Tempo is owned as shared monitoring infrastructure. Applications own their instrumentation, bounded export, context propagation, and coverage; they do not use Tempo queries or availability to complete business work.
- Start with a monolithic Tempo deployment and bounded asynchronous Rust SDK batch export to its authenticated private ingestion entry. No separate Collector is required for the initial complete lightweight collection and interface-only case selection; centralized processing or additional clients may justify one later.
- The shared platform supports scoped tenant reads and writes. CVM uses its own tenant with 24-hour diagnostic retention; other projects define their own collection, access, retention, and ingestion budgets when they join. Tenant routing must be authorized by the authenticated entry, not trusted solely from a caller-supplied header.
- Prometheus retains aggregate metric history, Grafana provides statistics and request case views, and business persistence retains authoritative invocation and terminal facts. Ordinary metric labels do not gain request or trace identities.
- Instrumented downstream response completion and associated asynchronous terminal durability remain distinct boundaries. Complete bounded lightweight collection, all HTTP proxy endpoint coverage, classified case presentation, privacy constraints, and observable capacity loss follow ADR 0033 and [the lifecycle design](../design/request-lifecycle-observability.md).
- Version, storage backend, host allocation, tenant identifiers, access mapping, and numerical limits require deployment facts and capacity validation. Official sizing guidance and hypothetical trace sizes are not measured overhead; combined clients, query peaks, block merging, and failures must be covered.

This records the accepted architecture and product choice. Application implementation, platform deployment, and delivery are separate work scopes.
