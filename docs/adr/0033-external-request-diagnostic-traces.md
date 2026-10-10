# External Request Diagnostic Traces

Status: Accepted

Extends the observability service boundary of [ADR 0025](0025-external-performance-observability.md).

Aggregate histograms and function reports cannot reconstruct one downstream request's resource waits, upstream attempts, response completion, and associated asynchronous terminal persistence. The owner approved an independent request diagnostic trace store, with Grafana providing case search and visualization while Prometheus continues to own aggregate metric history. This adds storage, collection, retention, capacity, and access responsibilities in exchange for individual causal evidence, rather than expanding the business database into a diagnostic history engine.

## Consequences

- The observed downstream response ends at its body completion, error, or cancellation boundary; associated terminal persistence retains its own interval and outcome.
- Trace collection and export must remain bounded and must not become a proxy, terminal durability, database admission, or task execution dependency. Failure and incomplete coverage are observable.
- Ordinary Prometheus labels retain their fixed cardinality contract. Diagnostic identities do not authorize exporting business request identities, payloads, credentials, SQL parameters, or account/user details.
- [ADR 0034](0034-shared-tempo-request-tracing.md) selects shared Tempo with OpenTelemetry/OTLP and initial direct bounded batch export. Exact capacity and storage deployment remain subject to implementation and platform validation.
- The owner selected complete bounded lightweight collection under normal capacity, with only a small classified set of cases shown in the interface. This separates case selection from collection sampling; capacity loss and incomplete traces remain explicit.
- Diagnostic traces are retained for 24 hours under an explicit storage cap, prioritizing storage cost. Aggregate metric history keeps its existing 30-day retention; cases cannot be reconstructed after their trace expires.

The confirmed scope, source evidence, alternatives, and remaining decisions are in [the lifecycle observability design](../design/request-lifecycle-observability.md).
