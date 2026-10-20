# Dashboard Performance Context

This context describes the Dashboard's live data surfaces and the performance contract used to improve them without changing owner-facing meaning or interaction.

## Language

**Dashboard Current State**:
The owner-facing data for the currently selected Dashboard window, including live activity, working conversations, parallel work, and open-window timeseries.
_Avoid_: dashboard snapshot, which can also mean a closed historical response.

**Hot Projection**:
A continuously maintained current-state read model whose updates are driven by typed runtime revisions and bounded deltas.
_Avoid_: live snapshot, which hides whether the update is incremental or a rebuilt full payload.

**Closed Snapshot**:
An exact response for a completed time window that does not receive unrelated current-runtime updates.
_Avoid_: historical hot topic.

**Bounded Cold Hydrate**:
A limited read used to establish or repair a projection selection, with explicit bounds on the keys, rows, and work it may process.
_Avoid_: fallback scan, which implies an unbounded recovery path.

**Full Frame**:
A serialized topic payload containing the complete selected read model for one revision.
_Avoid_: delta, which describes only a change from an established baseline.

**Delta Frame**:
A serialized change set that updates an established read model without repeating unaffected records or buckets.
_Avoid_: partial snapshot.

**Incremental SSE Frame**:
A Delta Frame delivered through the Dashboard's existing SSE connection after a complete baseline snapshot, with a revision and topic identity that let the client validate and merge it.
_Avoid_: notification, because the frame carries data and does not require a follow-up full fetch.

**Browser Performance Baseline**:
A fixed warm-up and measurement window that records browser workload, live-data volume, main-thread stalls, rendering cadence, and memory for comparison before and after a change.
_Avoid_: one-off performance impression.

## Proxy Routing Language

**Same-account retry**:
Repeating the current request on the already selected upstream account without selecting another account, route, or channel.
_Avoid_: upstream switch, account failover.

**Upstream switch**:
Selecting a different upstream account or route/channel after the current upstream attempt is no longer used for the request.
_Avoid_: same-account retry.

**Responses overload signal**:
A structured Responses failure that matches a recognized upstream overload code or a narrowly defined overload message under the relevant error code.
_Avoid_: every `server_error`, generic overload text.
