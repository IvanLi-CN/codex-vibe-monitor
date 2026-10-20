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

## Forward Proxy Context

**Forward Proxy Node**:
An endpoint that the service may select as the egress route for outbound requests.

**XHTTP Transport**:
An Xray transport that carries proxy traffic through HTTP-shaped sessions and has its own path, mode, and extension settings.

**XHTTP Share Link**:
A VLESS share URL that declares XHTTP as its transport and carries the transport's effective connection settings.

**Validation Probe**:
A real request sent through a candidate node to determine whether that node can reach the configured upstream.

**Available Candidate**:
A forward proxy node that satisfies the validation policy and may be offered for import or routing.

**Node Identity**:
The stable identifier used to recognize the same forward proxy node across normalization, deduplication, bindings, history, and runtime state.
