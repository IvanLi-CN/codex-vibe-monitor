# ADR 0011: Prompt Cache In-Flight Phase Index

Status: Accepted

## Context

Dashboard working-conversation cards need an authoritative conversation-level summary of active calls. The visible recent preview is intentionally capped at 16 records, so it cannot be used to count active identities. A phase transition must also retract the previous phase exactly once, including calls that never appear in the preview.

## Decision

The working-conversations materializer owns an in-memory index keyed by invocation identity and a counter map keyed by prompt-cache key. The initial index is hydrated from the exact `invocation_in_progress_live` query in the same cold baseline transaction; runtime overlays are added from the server runtime store. Typed deltas remove the identity's previous contribution before adding its current key and phase, and terminal/removal deltas remove it without synthesizing a zero.

`PromptCacheConversationResponse.inFlightPhaseCounts` is emitted by the backend and is copied into SSE frames. Steady-state publication reads only the materializer projection. If hydration, replay, or reconcile is incomplete, the last-good summary remains published and the projection is marked dirty until bounded recovery succeeds.

## Alternatives considered

- Aggregate the Web card's recent slots: rejected because the 16-record cap loses active identities and makes the client an authority.
- Scan SQLite for every mutation: rejected because it defeats the hot-topic latency and load contract.
- Persist a rollup table: rejected because this is a volatile projection and would add schema and recovery complexity.

## Consequences

The server keeps a bounded identity map for active working-conversation subscribers and performs exact I/O only at baseline, bounded key hydration, or dirty recovery. The additive response field is backward-compatible for clients that ignore it, while the current Web client treats it as the sole source for the header status cluster.
