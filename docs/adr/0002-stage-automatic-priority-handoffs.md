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
- Treat a strictly higher-priority target demoted only by existing temporary model-route failure evidence as a recovery attraction candidate after ordinary hard eligibility checks. Let it seek the same local permit before the ordinary model-penalty winner is returned, without changing the regular candidate comparator or making the target an unrestricted winner. Cache protection, unsupported capability, hard account failure, and caller cancellation do not create this exception.
- Choose only the highest-ranked recovery attraction candidate for each request. A busy or cooling permit keeps an eligible sticky request on its authoritative source and makes a fresh assignment use an ordinary healthy alternative; it never cascades into a second recovery candidate for the same request.
- A degraded recovery attraction candidate may seek request-driven recovery admission immediately; a cooling target remains excluded until cooldown expiry. Recovery is driven only by eligible real requests and never by a background probe or waiting queue.
- Every newly accepted temporary model-route failure starts a new local verification generation. An older in-flight permit stays exclusive until completion, but its terminal result cannot contribute evidence to the new generation; failure evidence rejected by existing temporal fences does not reset verification.
- A temporary account-model handoff failure immediately enters the existing model-route cooldown ladder. The first complete terminal recovery success restores ordinary model health, commits only that conversation-model route, and counts as the first of three consecutive gate-admitted successes. Unrestricted priority admission resumes only after the third success.
- Keep target account-model success evidence independent from sticky ownership. A complete success may recover health and count in the current verification generation even when a newer sticky generation prevents rebinding that conversation; it never overwrites the newer binding.
- Order overlapping health evidence by the existing request-start and reset fences, not completion time. A success that began no later than a newer accepted failure only releases its old permit and cannot recover health or advance verification.
- A route successfully rebound during verification continues normally on its new sticky target. The permit serializes new target admission, not subsequent traffic from a route already admitted.
- Manual bindings and ordinary fault failover do not wait on the handoff permit, including when fault failover independently selects the same target while a recovery request is in flight; ordinary account-model health eligibility still applies.
- Record the admission trigger independently from the gate decision: `priorityAttraction` and `modelRouteRecovery` explain why admission was considered, while the existing decision, phase, generation, and success count explain the gate outcome. Use `requestDrivenRecoveryAdmission` as the recovery winner reason. Persistence is best-effort and cannot affect routing or permit state.
- On process restart, discard permits and verification counts, begin local verification at `0/3`, and use persisted model health only to determine degraded, cooling, or ordinary target eligibility. Do not restore a lease or infer `open` from persisted availability.
- Expose one global operator switch through the existing settings surface, enabled by default. Disabling returns to the pre-gate routing behavior without cancelling in-flight requests; a valid terminal result may still update model health and a generation-safe sticky binding, but cannot contribute to a later switch generation. Re-enabling starts verification at `0/3`. Persist the desired setting, but make its locally mirrored runtime value the routing authority so a database outage cannot interrupt active behavior.

## Consequences

- Priority recovery cannot create an HTTP request queue for sticky conversations whose source remains eligible.
- The system intentionally keeps some conversations on a lower-priority source while validating a target.
- A process restart or future multi-instance deployment can permit an exceptional duplicate handoff attempt; correctness and availability do not depend on restoring a database lease.
