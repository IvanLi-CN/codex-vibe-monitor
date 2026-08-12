# Upstream Account Policy Inheritance

Spec ID: r4p9x

## Goal

Upstream account routing policy is resolved through four editable layers:

1. Root defaults
2. Group policy
3. Account policy
4. Conversation policy

Read-only system tag signals are applied after the selected model rule and cannot be overridden.

Only root, group, account, and conversation model policy are operator-editable. Tags are no longer a user-managed policy layer. The account-pool UI may display and filter system tags, but tag creation, editing, deletion, manual attach/detach, and tag-based policy authoring are not supported.

## Policy Surface

The editable inherited policy covers:

- priority tier
- FAST mode rewrite mode
- image tool rewrite mode
- upstream request compression algorithm for API-key upstream HTTP requests
- cut-out
- cut-in
- concurrency limit
- upstream 429 retry count (`0..5`)
- available models
- available models mode (`allowlist` or `denylist`)
- status-change trigger reasons for:
  - `upstream_http_401`
  - `upstream_http_402`
  - `upstream_http_403`
  - `reauth_required`
  - `upstream_http_429_rate_limit`
  - `upstream_http_429_quota_exhausted`
  - `usage_snapshot_exhausted`
  - `quota_still_exhausted`
  - `transport_failure`
  - `upstream_server_overloaded`
  - `upstream_http_5xx`
- request-path timeout overrides for:
  - `responsesFirstByteTimeoutSecs`
  - `compactFirstByteTimeoutSecs`
  - `imageFirstByteTimeoutSecs`
  - `responsesStreamTimeoutSecs`
  - `compactStreamTimeoutSecs`

Request compression uses this fixed algorithm set:

- `follow`
- `identity`
- `gzip`
- `deflate`
- `zstd`

`follow` is owner-facing "跟随". It means reuse the downstream request-body encoding semantics and re-encode the final upstream request body after proxy-side rewrites. Unsupported downstream encodings fail explicitly instead of silently falling back.

`priorityTier` is the owner-facing usage lane and accepts exactly `primary | normal | fallback | no_new`. `no_new` means the account does not admit fresh automatic assignments; it is carried by the priority field rather than a separate new-conversation policy field.

Root defaults preserve existing behavior:

- priority tier: normal
- FAST mode rewrite mode: keep original
- cut-out: allowed
- cut-in: allowed
- concurrency limit: unlimited
- upstream 429 retry: disabled
- upstream 429 max retries: 0
- image tool rewrite mode: keep original
- upstream request compression algorithm: identity
- upstream request compression level preset: balanced
- available models: unrestricted
- available models mode: denylist
- every status-change reason toggle: enabled
- request-path timeouts continue to use the existing global pool defaults

Accounts also track read-only system signals alongside editable policy:

- `systemDeniedModels`
- observed Responses endpoint capability
- observed Chat Completions endpoint capability
- observed direct image endpoint capability
- observed Responses image-tool capability
- observed Codex `image_gen` namespace capability
- observed API-key standalone search endpoint capability
- transport capability badges such as `unsupported_transport:websocket`

## Resolution

Effective account policy is computed in this order:

1. Start with root defaults (`denylist + []`).
2. Apply the group model rule when explicitly stored.
3. Apply the account model rule when explicitly stored.
4. Apply the conversation model rule when explicitly stored.
5. Apply `systemDeniedModels` as a final, non-overridable deny set.

Each explicit lower-level model rule replaces both the inherited mode and list. Missing or `null` lower-level fields inherit. An allowlist with an empty list rejects every model; a denylist with an empty list adds no restriction. Legacy records with a defined `availableModels` list are interpreted as allowlists, including legacy clients that submit only the list.

Clearing a lower-level model rule is atomic: a legacy client that sends only `availableModels: null` clears both the stored list and mode. Read-only legacy tag model constraints, when present, remain allowlists and never become an editable tag policy; they retain their constraint source while inheriting the four editable levels.

Request compression has one scope restriction:

- root settings may define the default algorithm and the global compression level preset
- group and account may override only the algorithm
- the group/account algorithm override applies only when the final target upstream account kind is `api_key_codex`
- OAuth upstreams ignore group/account request-compression overrides
- conversation overrides do not participate in request compression

Status-change reason toggles follow the same `group -> system -> account` resolution envelope with one restriction:

1. Start with root defaults where every listed reason is enabled.
2. Apply group per-reason overrides.
3. Ignore the system tag layer for this policy family.
4. Apply account per-reason overrides.

`conversation` overrides do not participate in this policy family.

The resolved toggle controls the health scope owned by the classified failure. For API Key live requests, enabled 5xx, 429, logical overload, and transport-shaped reasons may change only the exact model route state; they must not create account cooldown or remove the sticky route. For OAuth requests and explicit account-level authentication/payment failures, the same toggles retain their account-health behavior. A disabled reason remains evidence-only at either scope. API Key background sync failures without an exact model are always evidence-only for temporary reasons.

Request-path timeouts are resolved per field through a separate inheritance chain:

1. Start with the global/root pool timeout defaults.
2. Apply group timeout overrides.
3. Apply account timeout overrides.
4. Apply conversation timeout overrides.

Timeout inheritance is field-local:

- missing or unset means inherit
- a positive integer stores an explicit override
- clearing one timeout field only clears that field

Tags and system-tag signals never contribute timeout values or timeout sources.

Request compression level is global-only:

- root stores `requestCompressionLevelPreset`
- group/account never store or override a level
- level presets are `fast | balanced | best`
- runtime maps the preset to encoder-specific quality levels for `gzip`, `deflate`, and `zstd`

`imageFirstByteTimeoutSecs` applies to `/v1/images/generations` and `/v1/images/edits`, defaults to `300`, and follows the same field-local root -> group -> account -> conversation inheritance contract. Direct-image first-byte timeout is terminal: the proxy must not retry the same account or switch accounts after the image operation may already have started upstream.

Forward-proxy bindings are resolved independently from routing policy and timeout inheritance:

1. Conversation proxy override
2. Account proxy override
3. Group proxy binding

Each layer may store a list of existing forward-proxy binding keys, including `__direct__`. An empty account list means inherit the group list; an empty conversation list means inherit the selected account/group scope.

An explicit proxy list is a hard constraint. Runtime must select only from the configured list, keep the current selected node sticky for that scope, and switch to the next best selectable node from the same list only after the existing consecutive network-failure threshold is reached. If every explicit node is unavailable, routing fails with the existing proxy/account readiness error instead of falling back to an upstream layer or automatic proxy routing.

System tags are not an editable routing authoring surface. Their current contract is:

- `unsupported_model:<model>` appends `<model>` to `systemDeniedModels`
- `unsupported_transport:websocket` remains a read-only transport signal for display and filtering
- future system tags may add internal signals, but they must remain operator read-only

`availableModels` follows root -> group -> account -> conversation inheritance semantics:

- missing or `null` means inherit the upstream layer
- there is no tag-level allowlist editing
- account policy may replace the inherited group/root model set with its own list
- an explicit empty allowlist means no models are allowed
- an explicit empty denylist means no models are denied

The regular model candidate catalog includes `gpt-5.4-mini` without enabling it by default. Image candidates are independent from `/v1/models` hijacking and currently recommend `gpt-image-2`; historical image IDs, private aliases, and custom IDs remain valid policy values.

## Image Tool Routing

Endpoint capability routing is split into four independent account-level axes:

- `responseEndpointCapability` covers only `/v1/responses` and `/v1/responses/compact`
- `chatCompletionsCapability` covers only `/v1/chat/completions`
- `imageEndpointCapability` covers only `/v1/images/generations` and `/v1/images/edits`
- `responseImageToolCapability` applies only to `/v1/responses` and `/v1/responses/compact` when image intent is confirmed

Capability learning and gating follow the real request endpoint:

- successful or explicit unsupported `/v1/responses` and `/v1/responses/compact` requests update only the Responses axis plus the Responses image-tool axis when image intent is `yes`
- successful or explicit unsupported `/v1/chat/completions` requests update only the Chat Completions axis
- successful or explicit unsupported direct-image requests update only the direct image axis
- Chat Completions does not have a separate image-tool axis
- `standaloneSearchCapability` applies only to API-key accounts and exact `/v1/alpha/search` requests. Success learns `supported`; bare `404`/`405` learns `unsupported`; `400` learns `unsupported` only when the error explicitly identifies the search endpoint/path/route as unsupported. Authentication, rate-limit, other client, server, timeout, and transport failures preserve the prior observation.
- standalone search uses the same persistent `supported | unsupported | null(auto)` operator override contract as ordinary endpoint capabilities. It does not use the one-shot Codex imagegen retest claim.

Startup schema maintenance performs one capability-axis cutover:

- legacy mixed `responseEndpointCapability` observed values and overrides are reset once to `unknown`/`null`
- new `chatCompletionsCapability` state starts as `unknown` with no override
- the cutover is one-time only and must not erase states learned under the split-axis contract on later startups

The image-tool layer remains separate from the system-tag signal model:

- `imageToolRewriteMode` exists on group and account routing rules only
- `codexImagegenRewriteMode` is an independent root -> group -> account -> conversation policy with `keep_original | fill_missing | force_add | force_remove`; root defaults to `keep_original`
- Codex namespace compatibility is learned per upstream account. A `502` containing `Upstream request failed` after an actual canonical `image_gen` namespace is injected, replaced, or retained marks only that account as unsupported, skips same-account retries, and makes later active Codex rewrites select another compatible account. Automatic candidate selection and header-sticky reuse apply the same final rewrite-aware requirement. A successful request that carries the canonical namespace records `supported`; an operator's explicit `supported` override permits exactly one such retest and is atomically claimed before that upstream attempt, so concurrent callers cannot bypass the learned incompatibility. This signal does not alter ordinary response, hosted image-tool, or account-health capability axes. Its observed state, reason, and explicit supported/unsupported override are available in account detail so an operator can deliberately retest an upstream after it is repaired.
- account records persist a read-only `imageToolCapability`
- `image intent` classification is runtime four-state: `yes`, `direct_image`, `no`, or `unknown`
- `yes` routes only to image-compatible accounts
- `direct_image` represents direct image endpoints such as `/v1/images/generations|edits`; it also routes only to image-compatible accounts
- `unknown` keeps ordinary routing semantics and does not force image filtering
- `keep_original` treats `supported` and `unknown` accounts as image-compatible, and excludes `unsupported`
- `fill_missing` and `force_add` make the account image-compatible for routing
- `force_remove` makes the account image-incompatible for routing
- `fill_missing` only injects image tools when image intent is confirmed
- `force_add` always injects image tools
- `force_remove` always strips image tools
- `/v1/responses` and `/v1/responses/compact` may rewrite request bodies to satisfy the final account's rewrite mode
- Codex Lite is identified only by `X-OpenAI-Internal-Codex-Responses-Lite: true`; Codex Full is identified only by `originator: Codex Desktop` or a `Codex Desktop/…` user agent. Model names, body shape, and session ids are not protocol signals.
- For an identified Codex request with a non-default Codex policy, hosted `image_generation` and its matching `tool_choice` are removed before Codex imagegen handling.
- Codex Full merges the fixed `image_gen.imagegen` namespace snapshot into top-level `tools`. Codex Lite normalizes `input` to an array, merges developer `additional_tools`, and enforces `reasoning.context=all_turns` plus `parallel_tool_calls=false`.
- `fill_missing` adds the snapshot only when image intent is explicit and no same-name tool exists; `force_add` replaces a same-name conflicting schema; `force_remove` removes only Codex imagegen while retaining unrelated tools.
- `keep_original` does not decode, replay, or otherwise mutate a recognized Codex request body. A body that is otherwise eligible for live-first forwarding remains eligible; selecting an account with a non-default effective Codex policy falls back to the replayable-body path before any rewrite.
- a Lite validation message containing `responses lite`, `top-level tool type`, and `image_generation` is a request-shape error, not evidence that the upstream account lacks image-tool capability
- startup repair resets only historical observed `unsupported` entries matching that exact signature; it retains any manual capability override
- `/v1/images/generations` and `/v1/images/edits` classify as `direct_image`, only filter by capability, and do not rewrite the body
- successful image-intent requests learn `imageToolCapability=supported`
- explicit unsupported image responses learn `imageToolCapability=unsupported`

## Sticky Transfer Policy

`allow cut-out` is an automatic-routing boundary for the sticky source account. When the effective source policy forbids cut-out, the resolver must keep the conversation assigned to that account and fail there rather than automatically selecting another account, even when the sticky account has a transport failure, first-byte timeout, temporary route-key exclusion, cooldown, or other failover pressure.

When a sticky source account has effective `priorityTier=fallback`, has no explicit conversation binding, and cut-out is allowed, every subsequent request must compare that reusable source against eligible higher-priority candidates. The proactive comparison is enabled only after the sticky source itself passes the existing reusable-account resolution; an unavailable, capability-rejected, or otherwise non-reusable source continues through the existing fresh failover path. Only `normal` and `primary` candidates participate in this proactive handoff; fallback and `no_new` candidates cannot displace the current fallback source. The existing composite routing comparator remains authoritative, so capacity lane, route/model penalties, node readiness, and other health constraints may keep the fallback source selected.

A higher-priority handoff is temporary until the request succeeds. The selected fresh candidate carries the captured sticky generation, and only its successful route completion may replace the sticky target through the existing generation-guarded mutation. Failed or 4xx attempts leave the fallback target unchanged, and a stale completion cannot overwrite a newer sticky target.

The only supported exception is an explicit Prompt Cache conversation binding written by an operator. A manual upstream-account or group binding may move the conversation out of a no-cut-out sticky source; the target side still honors the binding contract and its existing target eligibility rules.

HTTP 4xx responses are not route-health successes for sticky routing. They remain recorded as failed invocations and upstream attempts with the real account, status, and error details, but they must not update `pool_sticky_routes`.

Fresh automatic assignment treats `priorityTier=no_new` as the only owner-facing forbid-new state. `no_new` accounts are excluded from fresh candidate admission and sorted after `fallback`; existing sticky reuse, explicit bindings, cut-out, and cut-in behavior keep their existing boundaries. Legacy database rows with `policy_allow_new_conversations=0` or `policy_block_new_conversations=1` are migrated to `policy_priority_tier='no_new'` during startup maintenance and are no longer exposed as API or UI fields.

`priorityTier`, `cut-out`, and `cut-in` are direct group/account overrides, not most-conservative merges. A lower editable layer that stores a value replaces the inherited value for that field. System tags may only add read-only deny/signal state; they are not a user-editable escape hatch.

Legacy rolling guard fields (`guardEnabled`, `lookbackHours`, `maxConversations`, and `guardRules`) are not part of the policy surface. Existing stored rolling guard data is ignored rather than migrated into the hard block.

## Tag Lifecycle Contract

The upstream-account module now treats tags as internal-only system data:

- application startup must delete every `pool_tags` row where `system_key IS NULL`
- startup cleanup must also delete matching `pool_upstream_account_tags` rows
- startup cleanup must clear any historical `pool_oauth_login_sessions.tag_ids_json` payloads
- account create, edit, relink, imported OAuth, external OAuth upsert, and batch account mutation requests must reject non-empty `tagIds` with a 4xx
- `GET /api/pool/tags` remains available only as a read-only system tag directory for list filtering and badge display

No migration, export, or policy flattening is performed for deleted custom tags.

## API Contract

Group summaries expose `routingRule`. Group update payloads accept `routingRule`.

Pool routing settings expose:

- `requestCompressionAlgorithm`
- `requestCompressionLevelPreset`
- `codexImagegenRewriteMode`

Account summaries and detail responses expose:

- read-only `tags` for system badge display
- read-only `responseEndpointCapability`
- read-only `chatCompletionsCapability`
- read-only `imageEndpointCapability`
- read-only `responseImageToolCapability`
- account-level `boundProxyKeys`
- effective-rule field sources including `systemDeniedModels`
- effective `requestCompressionAlgorithm`
- effective request-path `timeouts`
- request-path `timeoutFieldSources`

Account update payloads accept `routingRule` and `boundProxyKeys`. Missing `boundProxyKeys` preserves account-level proxy overrides; `null` or an empty list clears the account override and inherits the group proxy binding; a non-empty list stores an account-level hard proxy list after canonicalization and selectable-node validation.

Missing `routingRule` preserves account-level overrides. Inside a present `routingRule`, every account-policy field is tri-state:

- missing field: preserve that account override as stored
- `null`: clear that account override and inherit the upstream effective value
- value: store that value as the account override

`statusChangeReasons` uses the same nested tri-state semantics per reason key:

- missing object: preserve all stored per-reason overrides
- missing reason key inside a present object: preserve that stored reason override
- `null`: clear that reason override and inherit
- `true|false`: store that reason override

The same tri-state semantics apply to group policy updates for nullable policy fields. Boolean `false` is a stored override value and must not be treated as absent.

Capability override writes follow the same preserve / clear / set shape:

- `responseEndpointCapabilityOverride` applies only to the Responses axis
- `chatCompletionsCapabilityOverride` applies only to the Chat Completions axis
- `imageEndpointCapabilityOverride` applies only to the direct image axis
- `responseImageToolCapabilityOverride` applies only to the Responses image-tool axis

`requestCompressionAlgorithm` uses the same preserve / clear / set contract for group/account writes:

- missing field: preserve the stored algorithm override
- `null`: clear the stored override and inherit
- value: store one of `follow | identity | gzip | deflate | zstd`

`requestCompressionLevelPreset` exists only on root pool-routing settings updates and accepts `fast | balanced | best`.

`codexImagegenRewriteMode` follows the ordinary policy inheritance contract: group, account, and conversation fields are tri-state (`missing` preserves, `null` clears to inherit, a concrete mode overrides); root always stores a concrete mode.

Timeout writes use the same preserve / clear / set contract, but per timeout field:

- missing field: preserve the stored timeout override
- `null`: clear that timeout override and inherit
- positive integer: store that timeout override

Direct-image timeout returns `504 Gateway Timeout` with the additive machine field `code: "upstream_handshake_timeout"`, while preserving the existing `error` and `cvmId` fields.

UI may render `root` as `global`, but the wire/source token remains `root`.

Legacy `upstream_rejected` remains read-compatible only. Runtime must resolve it through the `upstream_http_402` toggle and must not expose a separate editable reason key.

When a listed reason resolves to `false`, runtime still records invocation and upstream-attempt evidence and must add a neutral account event carrying the original `reasonCode`, `httpStatus`, and message. Suppressed reasons must not mutate account status, cooldown, route-failure bookkeeping, failure counters, or latest-action fields that feed health/work derivation. Sync bookkeeping may still advance the non-health `lastSyncedAt` timestamp so maintenance cadence remains stable.

`GET /api/pool/tags` returns only system tags and reports the directory as non-writable.

Automatic candidate selection and sticky reuse must filter by the final model policy before scoring candidates:

- explicit account or group bindings still bypass automatic candidate filtering as they do today
- unconstrained routing first checks exact model ID matches
- if exact match fails, dated aliases may fall back to the existing base-model alias rule
- accounts denied for the requested model must be excluded from automatic and sticky migration candidates before retry/failover scoring

Model policy summaries and mode controls may use the shared compact chip presentation used by the account-pool surfaces. This is a presentation detail only: the wire values remain `allowlist`/`denylist`, and chip styling must not change inheritance, filtering, or system-deny semantics.

The root model-policy editor presents one in-place mode button immediately left of the model multi-select at desktop widths. Below `769px`, it keeps the allowlist/denylist segmented control above the multi-select. Neither presentation changes the selected model IDs.

## Owner-Facing UI Contract

Status-change trigger reasons use the same flattened reason list on every owner-facing surface.

Request compression editing follows the existing root -> group -> account inheritance model with one asymmetry:

- system settings expose global default algorithm and global level preset
- group settings expose only the algorithm override plus clear-to-inherit
- account detail routing exposes only the algorithm override plus clear-to-inherit
- owner-facing text labels `follow` as `跟随`
- mixed groups must explain that the group override only affects API-key members
- owner-facing outbound telemetry must distinguish the downstream request-body encoding from the actual upstream request-body encoding

- reason controls render as pressed/unpressed button-style tiles with icon + name only
- they do not use slider switches, category headers, or separate batch-toggle rows
- group policy surfaces keep per-reason editing only
- the account detail Routing tab exposes one panel-level reset action that clears just the account-layer reason overrides for this policy family
- per-reason account edits still happen by pressing the individual tiles; reset is the only bulk clear affordance on the account detail surface

Legacy `unsupported_model:gpt-5.5` handling is treated as one instance of the generic system deny rule rather than a special-case routing branch.

## Non-Goals

- Forward-proxy binding, node shunt, and notes are not part of system tag policy.
- User-maintained tag policies, tag ordering, or tag routing dialogs are not reintroduced.
- Historical custom tag strategies are not migrated onto groups or accounts.
- Image capability is not an editable account control.
- There is no separate image-only pool or tag-level image-tool field. `codexImagegenRewriteMode` controls advertisement of the Codex client executor only; it is not a hosted image capability or local executor.
- `/v1/chat/completions` image intent detection is not covered.
- Splitting text reasoning and image generation across two upstreams in the same Responses request is not introduced.
- OAuth/API key credential behavior is unchanged apart from rejecting manual `tagIds`.
- Global reverse-proxy `/v1/*` settings are unchanged.
- OAuth upstream requests, WebSocket routes, and conversation-level request compression overrides are not introduced.

## Visual Evidence

The effective model-policy evidence is bound to the dedicated Storybook canvas
`account-pool-components-effective-routing-rule-card--editable-available-models`
at the current implementation commit. The capture is mock-only, element-level
component evidence from the Storybook canvas at a `1440x1600` desktop CSS
viewport. It uses the light theme and passed `trim_whitespace.py` with
`--margin-policy require_margin --evidence-surface component`.
Evidence binding commit: `9a4f2bb5cf6c2785a06ee709f61a862e3579d8a6`.

PR: include
![Effective model policy desktop](./assets/effective-model-policy-desktop.png)

Visual evidence is captured from stable Storybook scenarios for:

- account-pool layout with the tag navigation entry removed
- upstream account create page without any tag editing controls
- upstream account detail edit view showing system tags as read-only badges
- upstream account list filtering by system tags while keeping system badges visible
- effective routing rule card inherited state, account override state, expanded inline editor state, field-level saving/error state, and explicit empty available-model override
- effective routing rule card showing `priorityTier` as one four-state row (`primary | normal | fallback | no_new`) with no separate new-conversation row
- effective routing rule card opening every existing account override panel by default
- effective routing rule card rendering available-model overrides as a tag selector
- effective routing rule card rendering upstream 429 retry as a `0..5` inline count selector without a separate toggle
- group/account routing dialogs showing mixed inherited/global timeout defaults with timeout rows collapsed until the current layer explicitly overrides a field
- account effective-rule card showing timeout source badges, inherited timeout rows collapsed by default, account-owned timeout rows expanded by default, and single-field clear-to-inherit rollback
- account detail Routing tab showing account-level forward-proxy bindings, inherited group bindings, and sticky failover semantics without the old "edit account policy" button
- Groups page opening the shared group routing policy dialog with flat status-change reason toggle tiles
- Upstream Accounts grouped roster opening the shared group routing policy dialog with the same flat status-change reason toggle tiles
- Upstream account detail Routing tab showing page-level status-change reason toggle tiles plus the panel-level account reset action inside the full drawer context
- Group settings Routing tab showing the embedded routing-policy upstream 429 retry count as the same integrated `0..5` selector used by account detail, with `0` representing no retry, in both desktop and narrow layouts
- Group settings Routing tab rendering priority, FAST mode, image-tool rewrite, and request compression as inline radio groups on desktop, while retaining compact Select controls at widths of `768px` and below
- Dashboard upstream-account quick policy chips showing explicit Fast rewrite labels for `force_add` and `keep_original`
- system settings page showing the global request compression defaults and request-path timeouts with `zstd` + `best` persisted after save
- group routing policy dialog showing the API-key-only request compression override row with mixed-group helper copy and `follow`
- effective routing rule card showing the resolved request compression row and account-owned source badge
- upstream account detail Overview showing independent endpoint/image cards plus the Codex `image_gen` namespace capability, whose observed failure reason and operator override remain distinct from hosted image-tool support
- API-key upstream account detail Overview showing six capability cards in a three-column desktop grid, including standalone search observation, effective state, reason, and persistent override

![Codex imagegen capability override](./assets/codex-imagegen-capability-override-final.png)

PR: include
![Account pool layout without tags nav](./assets/account-pool-layout-no-tags-nav.png)

PR: include
![Upstream account create page without tag editors](./assets/upstream-account-create-no-tag-editor.png)

PR: include
![Upstream account detail read-only system tags](./assets/upstream-account-detail-read-only-system-tags.png)

PR: include
![Upstream account list system tag filter](./assets/upstream-account-list-system-tag-filter.png)

PR: include
![Effective routing rule inline account overrides](./assets/effective-rule-inline-overrides-trimmed.png)

PR: include
![Effective routing rule account overrides default expanded](./assets/effective-rule-multiple-account-overrides-default-expanded.png)

PR: include
![Account route proxy bindings](./assets/account-route-proxy-bindings-story.png)

PR: include
![Effective routing rule available-model tag selector](./assets/effective-rule-available-models-tag-selector.png)

PR: include
![Effective routing rule upstream 429 retry count selector](./assets/effective-rule-429-retry-count-selector.png)

PR: include
![Group timeout mixed inheritance dialog](./assets/group-timeout-mixed-inheritance-story.png)

PR: include
![Account timeout source badges and overrides](./assets/account-timeout-source-badges-story.png)

PR: include
![Groups page routing policy dialog status change reasons](./assets/status-change-reasons-page-groups.png)

PR: include
![Upstream Accounts grouped roster routing policy dialog status change reasons](./assets/status-change-reasons-page-upstream-accounts-grouped.png)

PR: include
![Upstream account detail routing tab status change reasons](./assets/status-change-reasons-page-upstream-account-detail.png)

PR: include
![Group routing tab upstream 429 retry selector desktop](./assets/group-retry-selector-enabled-desktop.png)

PR: include
![Group routing tab upstream 429 retry selector mobile](./assets/group-retry-selector-enabled-mobile.png)

PR: include
![Group routing policy desktop inline selectors](./assets/group-routing-desktop-inline-selectors.png)

PR: include
![Group routing policy mobile selectors](./assets/group-routing-mobile-selectors.png)

PR: include
![Fast rewrite quick policy force add chip](./assets/fast-policy-force-fast-chip.png)

PR: include
![Fast rewrite quick policy leave unchanged chip](./assets/fast-policy-leave-fast-chip.png)

PR: include
![System settings routing defaults](./assets/system-settings-routing-defaults.png)

PR: include
![Group request compression follow override](./assets/2026-07-15-group-routing-follow-compression.png)

PR: include
![Effective routing rule request compression row](./assets/2026-07-15-effective-rule-request-compression.png)

PR: include
![Upstream account detail capability overview split](./assets/upstream-account-detail-capability-overview-split.png)

- source_type: storybook_canvas
  story_id_or_title: `Account Pool/Components/Effective Routing Rule Card / Editable Image Tool Help`
  state: account image-tool policy help open
  requested_viewport: none
  viewport_strategy: storybook-viewport
  margin_policy: trim_only
  evidence_surface: page
  evidence_note: verifies the real effective-rule card exposes the Full Responses-only policy boundary through the image-tool help affordance.
  PR: include
  target_program: mock-only
  capture_scope: browser-viewport
  sensitive_exclusion: fixture-only routing policy data
  submission_gate: approved
  image:
  ![Account image-tool policy help](./assets/responses-lite-image-tool-help-account.png)

- source_type: storybook_canvas
  story_id_or_title: `Account Pool/Components/Upstream Account Group Settings Dialog / Routing Policy Inline Editor`
  state: group routing settings image-tool help open
  requested_viewport: none
  viewport_strategy: storybook-viewport
  margin_policy: trim_only
  evidence_surface: page
  evidence_note: verifies the actual group settings dialog renders the same Lite client-owned-tools boundary beside the image-tool selector.
  PR: include
  target_program: mock-only
  capture_scope: browser-viewport
  sensitive_exclusion: fixture-only group policy data
  submission_gate: approved
  image:
  ![Group image-tool policy help](./assets/responses-lite-image-tool-help-group.png)

- source_type: storybook_canvas
  story_id_or_title: `Account Pool/Components/Effective Routing Rule Card / Editable Imagegen Rewrite Policies`
  state: account Codex imagegen rewrite policy set to force add
  requested_viewport: 819x1391
  viewport_strategy: chrome_storybook_iframe
  margin_policy: trim_only
  evidence_surface: component
  evidence_note: verifies the effective-rule card distinguishes hosted image tools from the independently inherited Codex imagegen policy and renders all four Codex rewrite modes.
  PR: include
  target_program: mock-only
  capture_scope: storybook iframe
  sensitive_exclusion: fixture-only routing policy data
  submission_gate: approved
  image:
  ![Codex imagegen rewrite policies](./assets/codex-imagegen-rewrite-policies.png)

- source_type: storybook_canvas
  story_id_or_title: `Account Pool/Pages/Upstream Accounts Page Overlays / Detail Drawer Overview`
  state: API-key account overview with the six capability cards visible
  requested_viewport: 1920x1080
  viewport_strategy: chrome_storybook_iframe
  margin_policy: trim_only
  evidence_surface: page
  evidence_note: verifies the desktop capability area uses a three-column, two-row layout and includes Standalone Search as the sixth independent capability.
  candidate_sha: `9f0e90f3193132112e68c75989a34765fddfb58d`
  PR: include
  target_program: mock-only
  capture_scope: storybook iframe
  sensitive_exclusion: fixture-only API-key account data
  submission_gate: approved
  image:
  ![Standalone Search capability desktop grid](./assets/standalone-search-capability-desktop-grid.png)

- source_type: storybook_canvas
  story_id_or_title: `Account Pool/Pages/Upstream Accounts Page Overlays / Detail Drawer Overview`
  state: API-key account overview scrolled to the Standalone Search capability card
  requested_viewport: 1920x1080
  viewport_strategy: chrome_storybook_iframe
  margin_policy: trim_only
  evidence_surface: page
  evidence_note: verifies the card exposes the exact endpoint, observed value, persistent override, effective value, observation time, and reason.
  candidate_sha: `9f0e90f3193132112e68c75989a34765fddfb58d`
  PR: include
  target_program: mock-only
  capture_scope: storybook iframe
  sensitive_exclusion: fixture-only API-key account data
  submission_gate: approved
  image:
  ![Standalone Search capability desktop details](./assets/standalone-search-capability-desktop-1920.png)

- source_type: storybook_canvas
  story_id_or_title: `Account Pool/Pages/Upstream Accounts Page Overlays / Detail Drawer Overview Mobile`
  state: narrow API-key account overview scrolled to the Standalone Search capability card
  requested_viewport: 390x844
  viewport_strategy: chrome_storybook_iframe
  margin_policy: trim_only
  evidence_surface: page
  evidence_note: verifies the capability cards collapse to one column without horizontal overflow while preserving all Standalone Search controls and state.
  candidate_sha: `9f0e90f3193132112e68c75989a34765fddfb58d`
  PR: include
  target_program: mock-only
  capture_scope: storybook iframe
  sensitive_exclusion: fixture-only API-key account data
  submission_gate: approved
  image:
  ![Standalone Search capability mobile](./assets/standalone-search-capability-mobile-390.png)

- source_type: storybook_canvas
  story_id_or_title: `Account Pool/Pages/Upstream Accounts Page Overlays / Detail Drawer Overview`
  state: API-key account overview with compact six-card capability layout
  requested_viewport: 1920x1080
  viewport_strategy: chrome_storybook_iframe
  margin_policy: trim_only
  evidence_surface: page
  evidence_note: verifies the compact desktop layout keeps six capability cards in three columns and two rows while retaining the status summary, override control, timestamp, and reason fields.
  candidate_sha: `9a5ffcb6594629aa06b7c8943eebc0ae65dcf87a`
  PR: include
  target_program: mock-only
  capture_scope: storybook iframe
  sensitive_exclusion: fixture-only API-key account data
  submission_gate: approved
  image:
  ![Compact standalone search capability desktop](./assets/standalone-search-capability-compact-desktop-viewport.png)

- source_type: storybook_canvas
  story_id_or_title: `Account Pool/Pages/Upstream Accounts Page Overlays / Detail Drawer Overview`
  state: API-key account overview focused on the compact Standalone Search card
  requested_viewport: 1920x1080
  viewport_strategy: chrome_storybook_iframe
  margin_policy: trim_only
  evidence_surface: page
  evidence_note: verifies the compact card preserves the exact endpoint, three-value status summary, persistent override selector, observation time, and reason without excessive vertical padding.
  candidate_sha: `9a5ffcb6594629aa06b7c8943eebc0ae65dcf87a`
  PR: include
  target_program: mock-only
  capture_scope: storybook iframe
  sensitive_exclusion: fixture-only API-key account data
  submission_gate: approved
  image:
  ![Compact standalone search capability details](./assets/standalone-search-capability-compact-desktop-search.png)

- source_type: storybook_canvas
  story_id_or_title: `Account Pool/Pages/Upstream Accounts Page Overlays / Detail Drawer Overview Mobile`
  state: narrow API-key account overview focused on the compact Standalone Search card
  requested_viewport: 390x844
  viewport_strategy: chrome_storybook_iframe
  margin_policy: trim_only
  evidence_surface: page
  evidence_note: verifies the compact single-column card remains readable on narrow screens, keeps the override control usable, and produces no horizontal overflow.
  candidate_sha: `9a5ffcb6594629aa06b7c8943eebc0ae65dcf87a`
  PR: include
  target_program: mock-only
  capture_scope: storybook iframe
  sensitive_exclusion: fixture-only API-key account data
  submission_gate: approved
  image:
  ![Compact standalone search capability mobile](./assets/standalone-search-capability-compact-mobile-390.png)
