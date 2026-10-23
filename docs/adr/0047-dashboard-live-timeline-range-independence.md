# Dashboard live timeline range independence

## Status

Accepted

The dashboard invocation timeline is an independent live view. It defaults to the most recent 30 minutes and remains continuous across local midnight while the overview range changes between today, yesterday, and a rolling 24 hours. The selected statistics range continues to control summary, rate, timeseries, parallel-work, cost, token, network, and trend reads.

The timeline keeps the invocation timeline contract from ADR 0019: account scope, invocation identity, opaque cursor pagination, one immutable `asOf` snapshot per traversal, bounded manual viewports, live revisions, loading/unavailable/stale states, and disconnect recovery. The selected natural-day scope remains accepted as a compatibility alias for callers that still send it, while the client uses bounded `scopeStart` and `scopeEnd` for the independent viewport scope.

## Decision

- Keep the timeline query path separate from the statistics-range context and use a fixed recent live window by default.
- Permit explicit UTC `from`/`to` bounds for summary, timeseries, and parallel-work HTTP reads, with paired bounds and a maximum span of 48 hours.
- Preserve legacy named ranges and natural-day timeline parameters so existing clients and SSE contracts remain readable during rollout.
- Keep the 180-day local-date activity calendar on the statistics page as a separate global metric surface.

## Consequences

- Switching from today to yesterday no longer turns off timeline updates or changes the timeline viewport.
- Non-timeline charts must carry the selected statistics range and account scope through their request context.
- A future client may replace the compatibility natural-day aliases after all consumers use `scopeStart`/`scopeEnd`; this ADR does not require that migration.

## References

- [Dashboard timeline replacement contract](0019-dashboard-timeline-replacement-contract.md)
- [Dashboard activity range and history Spec](../specs/dashboard-activity-range-and-history/SPEC.md)
