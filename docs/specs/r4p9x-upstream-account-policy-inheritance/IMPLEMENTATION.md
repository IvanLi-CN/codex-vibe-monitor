# Implementation

## Backend

The backend now treats upstream-account tags as internal system signals only.

- startup maintenance calls `cleanup_non_system_tags(...)` before normal runtime work
- cleanup removes every `pool_tags` row with `system_key IS NULL`
- cleanup removes matching `pool_upstream_account_tags` links
- cleanup clears historical `pool_oauth_login_sessions.tag_ids_json` payloads so stale draft metadata cannot revive deleted custom tags

Writable tag surfaces are removed from the HTTP contract.

- pool routes now expose only `GET /api/pool/tags`
- tag CRUD routes are no longer mounted
- bulk account actions no longer accept `add_tags` or `remove_tags`
- single-account and batch write paths reject non-empty `tagIds` with `400 manual tag assignment is no longer supported; omit tagIds`

Readable tag surfaces stay narrow.

- `GET /api/pool/tags` returns only rows where `system_key IS NOT NULL`
- account summaries and details still include `tags`, but only for read-only system badge display
- effective routing continues to surface `systemDeniedModels` and other system-derived signals without exposing tag editing

## Frontend

The account-pool UI is reduced to group/account policy editing plus read-only system tag visibility.

- `/account-pool/tags` route and module navigation entry are removed
- create flows no longer render tag fields for OAuth, batch OAuth, imported OAuth, or API key onboarding
- detail editing no longer renders tag pickers, create-tag actions, or delete-tag actions
- bulk add/remove tag actions are removed from the roster
- account cards and detail drawers still display system tags as read-only badges
- roster filtering still uses `usePoolTags`, but only against the system-tag directory

The account detail Routing tab exposes final effective rules as field-level inline account overrides.

- each editable effective-rule row has an icon-only account override control
- activating a row expands a field-local editor; clearing an active account override sends `null` and collapses the row
- existing account-level overrides render expanded by default when the account detail Routing tab opens
- boolean fields use positive operator-facing labels: `new conversations`, `cut-out`, and `cut-in`
- `new conversations` writes the positive `allowNewConversations` API payload and persists in `policy_allow_new_conversations`
- discrete policy fields use inline radio groups with an animated selected-state indicator and reduced-motion fallback
- `upstream 429 retry` is rendered as a single `0..5` inline count selector; `0` maps to disabled without a separate toggle control
- concurrency stays embedded in the expanded row; available models render as a tag-selector style control instead of repeated add buttons
- available-model overrides may store an empty list to explicitly allow no models
- `systemDeniedModels` stays a read-only system result and has no account override control
- timeout editors are shared across group/account surfaces and now use the same summary-row + source-badge + field-local expand interaction model as the effective routing rule card
- status-change reason toggles render as flat icon-and-label button tiles on both the group dialog and the account effective-rule card
- the group dialog no longer shows category headers or batch toggle rows for this policy family
- the account effective-rule card keeps the resolved badge summary and reason tiles, and adds one panel-level reset action that clears only account-layer reason overrides for this family

Status-change side effects are now gated by the resolved per-reason policy.

- both route-time failures and maintenance/manual sync failures consult the effective `statusChangeReasons` map after reason classification
- the root default for every listed reason is `true`, so unchanged deployments preserve existing behavior
- group and account storage persist one nullable override column per listed reason; system tags and conversation overrides do not write this family
- legacy `upstream_rejected` reads through the `upstream_http_402` toggle and is not exposed as a separate operator control
- when a reason is disabled, runtime preserves invocation / attempt evidence but writes a neutral suppression event instead of changing account status, cooldown, route-failure bookkeeping, counters, or latest-action state
- suppressed sync failures still advance the non-health sync timestamp so maintenance cadence does not collapse into immediate retries

## API and Resolution

Account and group routing policy writes distinguish missing, `null`, and value for nullable policy fields.

- missing preserves the stored override
- `null` clears the stored override
- value writes the override, including boolean `false`

Effective routing resolution applies group policy, read-only system signals, then account policy. Account-level `new conversations`, `cut-out`, and `cut-in` values replace inherited values directly; they no longer use a most-conservative merge at the account layer.

Effective routing now also exports:

- `statusChangeReasons`
- `statusChangeReasonFieldSources`

Group routing payloads and account routing payloads now accept `statusChangeReasons`, keyed by canonical `reasonCode`.

Request-path timeout resolution is evaluated after the final target account is known.

- group and account storage now persist nullable timeout overrides for the existing four request-path timeout fields
- runtime starts from the global/root pool timeout baseline, then applies `group -> account -> conversation` timeout overrides
- failover, replay, live HTTP dispatch, capture-target resolution, and WebSocket selection recompute effective timeouts for each newly selected target account

Local stale state is sanitized instead of preserved as a hidden write path.

- persisted roster filters drop tag IDs that are no longer present in the system-tag directory before they are applied back to the query
- restored batch/create drafts no longer replay legacy shared-tag sync metadata
- detail draft saves no longer write `tagIds`

Account-level forward-proxy bindings are now a first-class routing override.

- `pool_upstream_accounts.bound_proxy_keys_json` stores a nullable account-local list of canonical binding keys
- account update payloads accept `boundProxyKeys`; missing preserves, `null` or `[]` clears to inherit, and non-empty lists are validated against selectable existing binding nodes
- account summaries/details expose `boundProxyKeys`
- route resolution uses `conversation > account > group` proxy precedence
- account proxy lists use a dedicated `account:<id>` runtime scope so the current node remains sticky per account
- explicit account proxy lists are hard constraints; all-unavailable lists fail through the existing proxy readiness path rather than falling back to group or automatic routing
- the account detail Routing tab now shows the account proxy editor inline and no longer renders the separate "edit account policy" button

## Validation

Validation covers:

- backend startup cleanup removing non-system tags, account links, and pending OAuth session references
- backend 4xx rejection for manual `tagIds` on OAuth session create/update
- frontend unit regressions for account create and roster/detail flows after tag-editor removal
- Storybook states proving:
  - tag navigation is gone
  - create page no longer exposes tag editors
  - detail edit view keeps system tags read-only
  - roster filtering still works against system tags
  - inline account override rows show inherited, account override, expanded editor, saving/error, and empty-model override states
  - existing account overrides auto-expand on load, available models use the tag-selector control, and upstream 429 retry uses the `0..5` count selector without a separate switch
- timeout source badges and clear-to-inherit controls work across group, account, and conversation layers without involving tags
- timeout rows stay collapsed when the current layer does not override them; current-layer timeout overrides expand by default and can be cleared one field at a time without affecting untouched fields
- account route proxy binding Storybook evidence proves the inline account proxy editor, inherited/effective proxy chips, and removal of the old edit policy button
- backend regressions proving disabled reasons suppress account-state side effects for both route and sync paths while still creating neutral account events
- frontend regressions and Storybook states proving flat button-style reason toggles, the account panel-level reset behavior, and desktop / narrow-width readability
- `cargo test prompt_cache_conversation_proxy_override_bypasses_node_shunt_group_slots -- --nocapture`
- `cd web && npm test -- --run UpstreamAccounts.test.tsx`
- `cd web && npm run build`
