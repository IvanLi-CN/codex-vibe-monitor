# ADR 0014: Account effective routing rule control-plane projection

## Status

Accepted

## Context

`effectiveRoutingRule` is part of the authoritative Dashboard account payload, but it is neither an invocation nor a terminal statistic. The Dashboard activity snapshot cache can therefore retain an earlier rule while the live topic materializer advances only its current, network, and terminal slices. A successful Dashboard chip write can consequently be overwritten by a delayed earlier snapshot even though the database already contains the new account rule.

## Decision

- Treat an Account Effective Routing Rule Change as a low-frequency control-plane event, distinct from Priority Handoff and from the high-frequency Dashboard runtime data plane.
- After the durable write commits, refresh routing runtime state and publish one complete post-commit snapshot of every affected `(account_id, effective_routing_rule)` paired with a `RoutingStateVersion`. The version is a service-instance epoch plus the existing monotonic routing-cache generation.
- Account-level, tag, group, account-membership, and pool-default writes use this same publication path. A broad source change publishes every affected account rather than leaving consumers to recompute inheritance independently.
- Active `dashboard.activity.current` materializers patch their typed bases from that event and fan out a new frame without a database read. Cached Dashboard snapshots that contain account rules are invalidated at the same time; without an active owner, the next subscription establishes a new authoritative snapshot instead of replaying an old one.
- The Dashboard and account-write responses carry the additive `routingStateVersion`. Clients retain a successful local rule until they receive an equal-or-newer version, reject older frames or delayed PATCH responses, and accept a later committed change. Concurrent writes use last-commit-wins semantics.
- A committed database write remains successful if best-effort publication fails. The system invalidates/marks the affected current state dirty, retries or reports the delivery failure, and obtains the correct value on the next authoritative snapshot.

## Considered Options

- Shorten or clear only the 60-second Dashboard cache. Rejected because active topic bases can still fan out an earlier frame, and a control write would unnecessarily force a database rebuild on a live path.
- Delay or debounce client acceptance of SSE. Rejected because it hides the symptom in one page but leaves other consumers and write sources inconsistent.
- Publish only account IDs and let the materializer re-read the current cache or database. Rejected because a later refresh can pair a newer rule with an older event version; a complete post-commit event keeps value and order inseparable.
- Add a durable database-wide version counter. Rejected because the existing process-local routing generation already orders one service instance, while the instance epoch makes restart a fresh snapshot boundary.

## Consequences

- Dashboard account-rule delivery gains an additive versioned wire field and requires explicit out-of-order regression coverage.
- Healthy high-frequency Dashboard materialization continues to perform zero database reads; configuration writes, not invocation traffic, bear the routing refresh work.
- Future rule sources must join the shared post-commit publication path rather than invalidating only their own page state.
