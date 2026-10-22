# Dashboard Performance Context

This context describes the Dashboard's live data surfaces and the performance contract used to improve them without changing owner-facing meaning or interaction.

## Language

**Dashboard Current State**:
The owner-facing data for the currently selected Dashboard window, including live activity, working conversations, parallel work, and open-window timeseries.
_Avoid_: dashboard snapshot, which can also mean a closed historical response.

**Visible Model Generation**:
The complete version segment shown in a model identity, such as `5.6`, `6`, or `6.1`, used to distinguish model generations independently from the family glyph and reasoning effort.
_Avoid_: model level, reasoning effort, generation icon.

**Compact Model Context**:
The owner-facing model identity grouping used in a compact working-conversation or invocation surface, combining the visible model generation, family glyph, and available reasoning effort while retaining the full model ID as secondary identity information.
_Avoid_: simplified display, model label, model level.

**Model Identity Fallback**:
The explicit textual representation used when a model cannot be recognized as a supported identity or when its value is absent; it never infers a generation or family from incomplete data.
_Avoid_: guessed model, default model, blank identity.

**Model Routing Mismatch**:
A request model and response model that remain different after normalization, requiring both model identities and their directional relationship to stay understandable in compact diagnostics.
_Avoid_: model error, generation mismatch.

**Missing Reasoning Effort**:
An invocation has no collected reasoning-effort value, or only the display placeholder `—`; compact model context omits the effort segment, while an explicit `none` remains a real value.
_Avoid_: default reasoning effort, inferred effort, no reasoning.

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
