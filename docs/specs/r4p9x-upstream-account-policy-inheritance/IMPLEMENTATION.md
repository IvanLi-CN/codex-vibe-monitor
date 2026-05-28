# Implementation

## Backend

SQLite schema adds nullable account and group `policy_*` columns for policy overrides and extends tags with block-new-conversations and upstream 429 retry fields. Legacy rolling guard columns may remain in existing databases but runtime code no longer reads or writes them.

Runtime policy resolution now builds one effective `EffectiveRoutingRule` per account, records field-level sources for the final values, and feeds the effective rule to routing selection, sticky behavior, FAST rewriting, concurrency limiting, and upstream 429 retry.

Sticky routing enforces `allow_cut_out=false` before automatic failover can select a different account. Route-key exclusion from handshake or first-byte timeout does not relax the source cut-out boundary. Explicit Prompt Cache bindings are passed as operator constraints and remain the only supported cut-out override.

Pool route success recording only updates sticky routes for actual successful upstream responses. HTTP 4xx responses continue through the HTTP failure recording path, preserving invocation and attempt detail without rebinding `pool_sticky_routes`.

## Frontend

The API client normalizes the expanded routing policy surface on tags, groups, and effective account rules.

The tag rule dialog edits the hard "block new conversations" switch and upstream 429 retry alongside the existing routing controls. Group settings expose a routing policy editor entry, account detail exposes an account policy editor from the routing tab, and the effective routing card displays block-new-conversations, concurrency, upstream 429 retry state, and a field source breakdown for root, group, tag, and account layers.

The routing policy dialog now treats an opened editor as a stable draft session. Background account refreshes can update the parent detail object without replacing in-progress account policy edits; closing/reopening or switching to a different target still reinitializes from the latest source data.

## Validation

Validation covers:

- backend policy resolution across group, tag, and account layers
- fresh routing exclusion for blocked-new-conversations accounts while preserving existing sticky reuse
- upstream 429 retry in the final effective policy
- tag-layer override of group policy plus account override source tracking
- OR merge behavior for block-new-conversations across group, tag, and account policy
- frontend payload normalization for routing policy fields
- tag dialog submission with expanded policy payloads
- account policy drafts surviving background refresh while preserving changed-fields-only payloads
