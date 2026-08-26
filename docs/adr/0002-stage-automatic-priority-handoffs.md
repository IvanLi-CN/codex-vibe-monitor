# ADR 0002: Stage automatic priority handoffs through local permits

## Status

Accepted

## Context

A recovered or newly higher-priority upstream account-model pair can attract many sticky conversations at once. A successful sticky write alone does not limit those concurrent migration attempts, so an unreliable target can accumulate long-running work before ordinary health feedback excludes it. The source upstream remains usable for priority-driven movement, so delaying a migration must not delay the client request.

## Decision

- Treat automatic priority movement as a conversation-model handoff from an existing `Fallback` route, not an account-wide or whole-conversation move. Do not expand automatic movement from other priority tiers.
- Apply the admission gate to HTTP pool requests only. WebSocket routing, retry, and session-completion semantics remain unchanged.
- Gate automatic priority handoffs and fresh assignments per target API Key account-model pair during a priority attraction epoch. A non-admitted sticky request continues on its authoritative source and does not migrate to a lower-ranked target; a fresh request bypasses to another healthy candidate or terminates without waiting. Other account types retain their existing routing behavior.
- Make a handoff a single target attempt with no automatic retry. Only a complete terminal success commits the new sticky route. A replay to the source is allowed only when the target definitely did not receive the request.
- Use a process-local permit as the runtime authority. Database coordination is optional and must never block admission or release. Cancellation releases the permit; process restart discards it and new priority movement begins recovery verification again.
- A temporary account-model handoff failure immediately enters the existing model-route cooldown ladder. Cooldown expiry and manual health reset begin a serialized recovery verification phase; unrestricted priority admission resumes only after three gate-admitted automatic handoff or fresh-assignment successes.
- A route successfully rebound during verification continues normally on its new sticky target. The permit serializes new target admission, not subsequent traffic from a route already admitted.
- Manual bindings and ordinary fault failover do not wait on the handoff permit, while ordinary account-model health eligibility still applies.
- Record handoff admission, deferral, recovery progress, and cooldown through existing routing audit paths with safe structured reason codes. Persistence is best-effort and cannot affect routing or permit state.
- Expose one global operator switch through the existing settings surface, enabled by default. Disabling returns to the pre-gate routing behavior; re-enabling starts a new local verification generation without cancelling in-flight requests. Persist the desired setting, but make its locally mirrored runtime value the routing authority so a database outage cannot interrupt active behavior.

## Consequences

- Priority recovery cannot create an HTTP request queue for sticky conversations whose source remains eligible.
- The system intentionally keeps some conversations on a lower-priority source while validating a target.
- A process restart or future multi-instance deployment can permit an exceptional duplicate handoff attempt; correctness and availability do not depend on restoring a database lease.
