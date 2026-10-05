# External Performance Observability

Status: Accepted

Supersedes [ADR 0019](0019-isolated-bounded-performance-telemetry.md).

Performance troubleshooting needs reusable charts, code attribution, CPU profiles, and reliable Agent access without maintaining a project-owned time-series database or charting application. Prometheus owns metric history, Grafana owns performance charts, and hotpath-rs supplies function, SQL, route, and selected lock diagnostics inside the Rust process. The legacy telemetry collector, SQLite writer, rollups, history APIs, and performance page are retired together in a controlled cutover; useful observations move to a bounded SDK adapter, while authoritative business and task records remain in their existing stores.

Grafana is available over public HTTPS: humans use the existing interactive authentication path, and Agents use a separate read-only Grafana service-account token through explicitly scoped machine API routes. Agents query Grafana without SSH or direct Prometheus access. The application retains separately authenticated, read-only hotpath report endpoints; SSH is permitted for hotpath diagnostics and bounded samply capture of the running application, so no profiler helper service is introduced.

## Consequences

- Prometheus scrapes private application and hotpath exporters every 15 seconds. Online history defaults to 30 days subject to the allocated storage limit; it does not preserve the previous selected 13-month online view.
- Legacy time buckets are not converted into cumulative counters or individual latency samples. A consistent offline archive remains outside the running application's mounts for 90 days, under the operator's backup policy.
- The metrics adapter has no durable queue, schema, rollup, or application-facing history query. Monitoring failure cannot become a proxy completion, terminal ACK, database admission, or task execution dependency.
- Request-head timing, complete stream lifetime, invocation identity, upstream attempts, database admission, queue waiting, and SQL execution retain separate meanings. Averages from the legacy collector cannot be treated as raw latency samples.
- Public read-only diagnostics and private scrape access use separate credentials. Raw bodies, headers, request identities, SQL parameters, and raw SQL logs are excluded. Formal dashboards and rules are provisioned from version-controlled files.
- CPU profiles are short-lived files produced on demand with matching binary symbols. They are neither time-series history nor evidence of off-CPU waiting.
- Removing legacy APIs, environment variables, and the task performance-summary field is a breaking release. Accepted design status does not mean the replacement has been implemented or deployed.

The owner-confirmed component, access, retirement, recovery, and acceptance contracts are in [the complete design](../design/performance-observability.md). Per-metric migration decisions are in [the migration map](../design/performance-observability-metrics.md).
