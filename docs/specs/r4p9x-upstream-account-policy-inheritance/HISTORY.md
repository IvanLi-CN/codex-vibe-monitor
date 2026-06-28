# History

## r4p9x

Account-pool routing policy moved from isolated group/tag behavior to a layered effective policy model. The resolver now computes one effective policy per account and downstream routing code reads that policy instead of separate group or tag fragments.

2026-06-27: Removed user-defined upstream-account tags. Startup now deletes legacy custom tags and pending session references, `/api/pool/tags` is read-only system-only, and the account-pool UI no longer exposes tag management or manual tag mutation.

2026-06-28: Added field-level inline account overrides to the effective routing rule card. Account/group routing PATCH payloads now distinguish missing, `null`, and value for nullable policy fields; account-level `new conversations`, `cut-out`, and `cut-in` directly override inherited values instead of using most-conservative merging. Available-model overrides may now explicitly store an empty list to deny every model.

2026-06-23: Split image-tool request rewrite from image capability discovery, added four-state image intent routing (`yes|direct_image|no|unknown`) for Responses and direct image endpoints, and exposed the new group/account image-tool controls in Storybook.

2026-05-27: Clarified and enforced sticky transfer boundaries: `allow_cut_out=false` blocks automatic timeout/failover migration even when the current route key is excluded, while explicit Prompt Cache bindings remain the only manual cut-out override. HTTP 4xx responses no longer count as sticky route successes.

2026-06-12: Expanded the inherited routing surface with `availableModels`, added tag-level model intersection plus account/group override semantics, and folded `unsupported_model:*` discovery into generic `systemDeniedModels` so automatic routing and sticky migration both exclude unsupported requested models before scoring.
