# History

## r4p9x

Account-pool routing policy moved from isolated group/tag behavior to a layered effective policy model. The resolver now computes one effective policy per account and downstream routing code reads that policy instead of separate group or tag fragments.

2026-07-06: Added inherited per-reason status-change toggles for upstream auth, quota, transport, overload, and 5xx failure families. Group and account policy now resolve `statusChangeReasons` with root defaults of all-enabled, route and sync state mutations consult the resolved toggle before touching account health/cooldown/latest-action fields, and suppressed reasons create neutral account events instead of mutating account state.
2026-07-07: Refined the owner-facing status-change reason UI after review. Reason controls are now flat icon-and-label button tiles, category and batch-toggle rows were removed, the account detail Routing tab widened the tile layout for the available drawer width, and account-level rollback is exposed as one panel-level reset action instead of per-tile inherit buttons.
2026-07-07: Split the group settings dialog into group info, routing settings, and proxy nodes tabs. Group routing policy now uses the shared inline routing-rule editor inside the routing tab, keeps policy edits draft-local across tab switches, and saves them through the unified group settings payload.
2026-07-07: Aligned group-level upstream 429 retry editing with the account detail routing surface. The group routing tab now uses a single `0..5` inline selector, where `0` means no retry and writes disabled retry payload fields.
2026-07-07: Renamed Dashboard upstream-account Fast quick policy chip states to a clear rewrite-policy axis: `不改Fast / 补Fast / 强制Fast / 禁Fast`. Tooltip and aria copy now identify the control as the Fast rewrite policy instead of using objectless wording.
2026-07-08: Collapsed the independent new-conversation policy into `priorityTier=no_new`. Legacy forbid-new group/account rows migrate to `no_new`, API/UI surfaces no longer expose `allowNewConversations` or `blockNewConversations`, and the account detail effective-rule card edits a single four-state priority row aligned with the Dashboard account-card chip.

2026-06-29: Added field-level request-path timeout inheritance for the existing four timeout fields. Group and account policy now persist nullable timeout overrides, UI surfaces expose timeout source badges and clear-to-inherit controls, and runtime recomputes effective timeouts for each newly selected target account during failover.
2026-07-01: Reworked owner-facing timeout editing to match the effective-rule interaction model. Group/account dialogs and the account effective-rule card now render timeout rows as collapsed summaries by default, treat field expansion as the local override affordance, auto-expand existing local timeout overrides, and preserve explicit overrides even when the entered value matches the inherited number.

2026-07-03: Added account-level multi forward-proxy bindings and removed the account detail Routing tab's separate "edit account policy" button. Proxy binding precedence is now conversation > account > group; explicit lists are hard constraints with sticky current-node reuse and existing network-failure based failover inside the same list.

2026-06-27: Removed user-defined upstream-account tags. Startup now deletes legacy custom tags and pending session references, `/api/pool/tags` is read-only system-only, and the account-pool UI no longer exposes tag management or manual tag mutation.

2026-06-28: Added field-level inline account overrides to the effective routing rule card. Account/group routing PATCH payloads now distinguish missing, `null`, and value for nullable policy fields; account-level priority, cut-out, and cut-in directly override inherited values instead of using most-conservative merging. Available-model overrides may now explicitly store an empty list to deny every model.

2026-06-23: Split image-tool request rewrite from image capability discovery, added four-state image intent routing (`yes|direct_image|no|unknown`) for Responses and direct image endpoints, and exposed the new group/account image-tool controls in Storybook.

2026-05-27: Clarified and enforced sticky transfer boundaries: `allow_cut_out=false` blocks automatic timeout/failover migration even when the current route key is excluded, while explicit Prompt Cache bindings remain the only manual cut-out override. HTTP 4xx responses no longer count as sticky route successes.

2026-06-12: Expanded the inherited routing surface with `availableModels`, added tag-level model intersection plus account/group override semantics, and folded `unsupported_model:*` discovery into generic `systemDeniedModels` so automatic routing and sticky migration both exclude unsupported requested models before scoring.
