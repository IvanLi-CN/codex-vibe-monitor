# Dashboard timeline replacement contract

## Status

Accepted

The screenshot's Dashboard activity chart is replaced only for the natural-day “今日/昨日 + 次数” views, globally and by upstream account. The invocation timeline is the sole chart in that surface; the legacy aggregate chart remains only for metrics and ranges outside the topic and is never a runtime fallback for timeline errors, disconnection, invalid bounds, or high volume. A temporary conversion exception is not part of the model because the timeline endpoint reads existing invocation records directly.

The timeline endpoint uses opaque cursor pagination with one immutable `asOf` snapshot across a traversal. The client merges pages by logical Invocation identity, keeps one bar per Invocation, and coalesces live revisions while a request is in flight. A failed refresh retains and freezes the last successful timeline, while an initial failure stays inside the new chart surface as an explicit unavailable state.

The endpoint also receives the selected natural-day UTC bounds separately from the shorter viewport. It rejects scopes and viewports wider than 24 hours and uses at most the preceding 24 hours for a cross-midnight boundary overlap. The duration safety limit for malformed records is not a query lookback.

## Considered Options

- Keep the legacy chart as a generic fallback: rejected because it violates the replacement requirement and hides whether the new detail model is unavailable.
- Split the requested time range into client-selected windows: rejected because live data can move between windows and boundary overlap can produce inconsistent snapshots.
- Raise a fixed response limit: rejected because future concurrency can exceed any fixed threshold and the chart would eventually regress to truncation.
- Abort every live request on each revision: rejected because dense SSE traffic can prevent any authoritative request from settling.

## Consequences

- The new chart must own explicit loading, unavailable, stale, and high-volume states.
- Backend and frontend share a stable snapshot contract, cursor traversal, and logical Invocation merge key.
- Non-target aggregate charts remain available without being confused with the replacement surface.
