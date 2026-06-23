# Implementation

## Backend

SQLite schema now persists group/account routing overrides for `image_tool_rewrite_mode` and account-level observed `image_tool_capability`, alongside the existing policy columns and tag retry/block fields. Legacy rolling guard columns may remain in existing databases but runtime code no longer reads or writes them.

Runtime policy resolution now builds one effective `EffectiveRoutingRule` per account, records field-level sources for the final values, and feeds the effective rule to routing selection, sticky behavior, FAST rewriting, image-tool rewriting, concurrency limiting, and upstream 429 retry.

`availableModels` remains part of the shared group/tag/account policy surface:

- group policy stores an inherited model allowlist override
- tag policy stores per-tag model allowlists that merge by intersection when multiple tags apply
- account policy can replace the inherited/tag-intersected allowlist with its own non-empty list
- empty and missing payloads both mean inherit, so there is no explicit “clear to unrestricted” override state

System unsupported-model tags are folded into the same effective model constraint:

- `unsupported_model:<model>` contributes to `systemDeniedModels`
- deny matches run before candidate scoring for automatic routing and sticky migration
- matching prefers exact requested model IDs first and then reuses the existing dated-alias fallback
- explicit manual bindings still keep their existing bypass behavior

Image routing is split from capability discovery:

- `imageToolRewriteMode` is resolved on the group/account policy path, with `keep_original` consulting the account's observed `image_tool_capability`
- image compatibility for `image intent=yes` is derived from both rewrite mode and observed capability:
  - `fill_missing` and `force_add` are always image-compatible
  - `force_remove` is never image-compatible
  - `keep_original` allows `supported` and `unknown`, but excludes `unsupported`
- `fill_missing` only injects image tools when image intent is confirmed
- `force_add` always injects image tools
- `force_remove` always strips image tools
- direct image endpoints only filter by capability and do not rewrite request bodies
- Responses family rewrites are applied only when the request body can be parsed safely
- successful image-intent requests write back `image_tool_capability=supported`
- explicit unsupported image responses write back `image_tool_capability=unsupported`

Sticky routing enforces `allow_cut_out=false` before automatic failover can select a different account. Route-key exclusion from handshake or first-byte timeout does not relax the source cut-out boundary. Explicit Prompt Cache bindings are passed as operator constraints and remain the only supported cut-out override.

Pool route success recording only updates sticky routes for actual successful upstream responses. HTTP 4xx responses continue through the HTTP failure recording path, preserving invocation and attempt detail without rebinding `pool_sticky_routes`.

## Frontend

The API client now normalizes the expanded routing policy surface on tags, groups, and effective account rules, including `imageToolRewriteMode`, `imageToolCapability`, and the new field-source entry for image tools.

The shared routing policy editor now keeps the existing tag surface while exposing the new image-tool selector only on group/account policy:

- tag creation/editing keeps the existing routing controls
- group routing policy editing adds the image-tool rewrite selector
- account routing policy editing adds the image-tool rewrite selector

The editor reuses proxy preset model candidate options, allows custom model ID entry, and keeps selected custom IDs visible as chips so inherited clearing works consistently even when a model is not in the preset list.

The account detail drawer now shows the read-only image capability badge and explanation alongside the other overview fields.

The effective routing card now displays both:

- `availableModels` with field source provenance
- `systemDeniedModels` with the non-editable `system` source
- `imageToolRewriteMode` with its own source provenance

The wider account-pool surfaces reuse the same dialog labels, payload shape, and story runtime mocks so the three entry points stay aligned.

The routing policy dialog now treats an opened editor as a stable draft session. Background account refreshes can update the parent detail object without replacing in-progress account policy edits; closing/reopening or switching to a different target still reinitializes from the latest source data.

## Validation

Validation covers:

- backend policy resolution across group, tag, and account layers
- backend model policy inheritance across group, tag, and account layers
- backend image-tool rewrite resolution across group and account layers
- tag intersection semantics for available models
- system unsupported-model tags folded into effective deny state
- automatic and sticky routing exclusion for unsupported requested models while preserving explicit binding bypass
- fresh routing exclusion for blocked-new-conversations accounts while preserving existing sticky reuse
- upstream 429 retry in the final effective policy
- tag-layer override of group policy plus account override source tracking
- OR merge behavior for block-new-conversations across group, tag, and account policy
- frontend payload normalization for routing policy fields
- frontend image-tool routing policy controls and account capability display
- tag dialog submission with expanded policy payloads and custom-model dedupe
- account policy drafts surviving background refresh while preserving changed-fields-only payloads
- Storybook canvas captures for the group routing policy editor, account overview image capability state, and effective routing rule card
