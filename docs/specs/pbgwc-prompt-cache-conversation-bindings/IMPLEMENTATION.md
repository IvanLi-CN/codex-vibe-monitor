# Prompt Cache Conversation Bindings - Implementation

## Current State

- Status: implemented
- Canonical spec: `docs/specs/pbgwc-prompt-cache-conversation-bindings/SPEC.md`

## Calls card projection

The shared conversation Calls view uses `InvocationCardList` (the compatibility export remains `InvocationTable`) so Live and conversation drawers consume the same card projection. The projection is presentation-only: terminal timing still comes from the persisted invocation fields, while one shared one-second clock supplies elapsed values only for in-flight rows. Virtual rows retain stable invocation keys and mount the existing `InvocationWorkflowDetailPanel` in the expanded card region.

## Delivery Checklist

- [x] Storage schema for `prompt_cache_conversation_bindings`.
- [x] HTTP binding API with validation.
- [x] Conversation timeout-only rows and per-field timeout PATCH semantics.
- [x] Conversation runtime policy override columns and per-field PATCH semantics.
- [x] Account-pool runtime routing constraints.
- [x] Conversation overrides for upstream switching, FAST mode, image tool, available models, and a hard multi-node forward-proxy binding list.
- [x] Conversation override for Codex imagegen rewrite mode, including source badges, explicit clear-to-inherit, and the shared four-mode selector.
- [x] Dashboard conversations multi-select, including persistent selection mode and temporary `Cmd`/`Ctrl` modifier selection keyed by `promptCacheKey`.
- [x] Dashboard floating bulk action bar with route binding, manual binding clear, FAST mode, and cancel-selection actions.
- [x] Bulk Prompt Cache conversation binding API for bind, manual binding clear, clear/reset affinity, and FAST mode writes with per-item result snapshots.
- [x] Forced upstream account binding bypasses sticky cut-in/cut-out policy while preserving health, quota, guard, concurrency, route-key, and forward-proxy checks.
- [x] Manual group binding bypasses sticky source cut-out policy while preserving target cut-in and target account eligibility.
- [x] Automatic sticky escape for non-explicit routes after account-global consecutive transport/decode-shaped `upstream_stream_error` failures, while preserving explicit upstream-account operator overrides and group-only reselection semantics.
- [x] Upstream account binding writes the corresponding sticky route immediately.
- [x] Automatic Sticky affinity is isolated by normalized conversation + model buckets with per-model generations and conversation-level epoch fencing.
- [x] Legacy Sticky rows migrate as all-model fallbacks; binding responses expose the fallback and all materialized model routes.
- [x] Manual account binding atomically rewrites the fallback and materialized model buckets, while full affinity reset clears every affinity row and owner lock.
- [x] The affinity-reset confirmation uses a padded content group and a separate safe-area action footer, with dedicated desktop and `393x852` Storybook states.
- [x] Automatic clear causes persist and are consumed at the matching all-model or normalized-model generation scope, so one model's failed route cannot annotate another model's fresh assignment.
- [x] Prompt Cache conversation detail drawer controls.
- [x] Prompt Cache conversation detail drawer uses `概览 / 调用 / 路由 / 设置 / 事件记录`, with route controls consolidated in 路由.
- [x] Current-route upstream-account values open the shared account-detail view when an account ID is available.
- [x] Current routing uses a desktop table and an overflow-free mobile field/value layout, with Storybook width assertions at `393x852`.
- [x] Prompt Cache conversation detail drawer title and Settings tab policy controls with effective-value rows, source badges, and field-level edit/clear behavior.
- [x] Prompt Cache conversation detail drawer sibling `事件记录` tab with categorized badges, lightweight filters, and paged per-conversation event loading.
- [x] Prompt Cache conversation detail drawer reuses the account-detail wide shell width class and the shared effective-routing form skeleton, while hiding account-only routing rows on the conversation surface.
- [x] FAST mode and image tool editors expose only concrete rewrite choices and remain expanded after a successful choice save.
- [x] Prompt Cache conversation timeout editor with source badges, collapsed inherited rows, and field-level expand/clear behavior aligned with the effective routing rule card.
- [x] Prompt Cache conversation history drawer loads retained invocation records in 50-row scroll pages instead of hydrating all pages on open.
- [x] Append-only Prompt Cache conversation operation-event storage, list API, and event emission across detail drawer writes, Dashboard bulk actions, and system auto group promotions.
- [x] `InvocationTable` virtualizes desktop table rows and mobile cards, mounting only the active breakpoint layout.
- [x] Unit, integration, Storybook, and visual evidence coverage.

## Realtime Detail Drawer Update

- Four tab-scoped topic descriptors now cover the shared conversation drawer: `invocation-history.window`, `invocation-history.overview`, `prompt-cache.conversation-binding.current`, and `prompt-cache.conversation-operations.window`.
- Calls merges the current 50-row topic window over the entire frozen HTTP snapshot, including page 1, by stable invocation key. Running rows update in place; deferred new rows preserve the reading anchor and surface a counted reveal action.
- Overview executes its current summary plus bounded chart samples through one SQLite read transaction and one captured runtime overlay, keeps the accepted page width fixed across internal pages, coalesces record-driven rebuilds for two seconds, and keeps last-good data on refresh failure. Its SSE-disabled HTTP fallback captures an unpinned head then re-reads that head with the returned snapshot for summary, every sample page, and the oldest page that retains full-history chart bounds.
- Calls resets drawer-local history before cached topic hydration, so direct Calls opens retain a synchronous replay. Away from the top, an authoritative head updates retained visible stable keys in place, preserves rows that have fallen outside the newest window, and queues only genuinely new stable keys.
- Binding and operations topics receive committed conversation-configuration broadcasts from detail saves, bulk changes, affinity resets, automatic sticky changes, and group promotions. Settings holds a dirty local draft until the operator explicitly adopts the external snapshot or saves last-write-wins; its SSE-disabled cached-payload baseline is reset for every conversation scope.
- Storybook covers deferred-call insertion and Settings conflict actions; the mock-only web demo verifies the Calls drawer at desktop and `393x852` mobile viewports.

## Dashboard Bulk Actions Update

- Dashboard conversation cards now support two selection entry points: explicit `选择模式` and temporary `Cmd`/`Ctrl` modifier selection that does not flip the page into persistent selection mode.
- The bottom bulk action bar is viewport-anchored, theme-aligned, and clears only successful items after each action so failed items remain selected for retry.
- The bulk route-bind dialog now uses compact dropdowns on a single row (`绑定到 / kind / target`); its destructive footer action clears only the manual binding and ignores the current target dropdowns, while the floating bulk action bar keeps the separate clear-and-reselect affinity shortcut.
- Dashboard bulk route-bind now keeps a client-only unified MRU list in browser localStorage, restores the newest valid target on dialog open, and silently prunes stale groups/accounts before showing recents.
- The route-bind dialog renders up to five recent group/account targets under the binding row using variable-width chips. The visible strip stays within two rows; when the full MRU list would overflow, the dialog omits the extra tail items instead of introducing any alternate selector or placeholder chip. Picking any recent target only refills the current kind + target selection, while successful `bind` actions are the only path that updates the MRU list.
- `POST /api/stats/prompt-cache-conversation-bindings/bulk-actions` validates the shared action payload first, then executes each selected `promptCacheKey` through the same save/clear helpers as the single-conversation surface and returns a per-item binding snapshot for UI recovery.
- The bulk clear confirmation dialog now consumes theme-scoped semantic surface tokens instead of `:root`-locked derived colors, so its header/footer chrome and destructive callout stay dark when Storybook or other nested theme hosts render the dialog under `data-theme='vibe-dark'`.
- The Storybook-local theme override now routes through `ThemeProvider`, keeping `html` and `body` theme attributes aligned before the clear-confirm interaction assertions evaluate the dialog.

## Conversation Operations Update

- `prompt_cache_conversation_operation_events` stores append-only detail events per `promptCacheKey`, including action, origin, categorized `infoTypes[]`, optional binding/sticky snapshots, and optional `invokeId`.
- `routing_scope_json` records `{kind:"all"}` or `{kind:"model",modelKey,requestModel}`; legacy routing rows are backfilled as all-model scope. Conversation-level multi-bucket changes additionally persist `sticky_transitions_json` for expandable before/after detail.
- `GET /api/stats/prompt-cache-conversation-binding-events/{encodedPromptCacheKey}` returns paged newest-first records, stable full-history model facets, and supports `infoType`, `routingScope`, and normalized `routingModel` filters.
- The Events tab keys its local source by both event category and routing-model scope, so changing back to unrestricted replaces a filtered HTTP subset with the live unfiltered head.
- Detail-drawer PATCH writes emit `detailDrawer` records, Dashboard bulk workflows emit `dashboardBulk` records, and automatic group-to-account promotions emit `systemAuto` records.
- Manual binding and full reset operations collapse their multi-bucket Sticky changes into one conversation-level event; policy-field PATCHes stay collapsed into one `conversationPolicyUpdated` summary event whose categories derive from the actual changed fields.
- Runtime Sticky writes now use the persisted affinity generation as an optimistic concurrency token. Target creation, replacement, and conditional automatic removal advance it under the SQLite writer lock; automatic removal also requires the original failed account. The first successful concurrent completion wins and later completions are audited without overwriting the target.
- New automatic routing events persist a structured, safe `routingContext` with reason code, routing source, HTTP status, and public cause/trigger attempt IDs. Existing rows remain unchanged and are identified by the UI as historical events without a recorded reason.
- Automatic routing writes use the exact normalized model bucket first, then the all-model fallback. Request writes capture both the conversation epoch and model generation; model buckets fence independently while manual switch/reset advances the conversation epoch.
- Fresh assignment now persists a `routing_selection_audit_json` snapshot on the request attempt before dispatch. The snapshot includes the selected and runner-up comparator values, including the numeric model-route penalties and their safe state codes. It is copied into the resulting Sticky operation event and rendered both in the Events tab and Records attempt card, so the selected account, decisive comparator, and normalized excluded-candidate reasons describe the decision at routing time rather than current account state.
- Automatic event links now carry the public attempt ID plus its invoke ID when available. The event also exposes a compact invoke-ID link with the full corresponding-invocation meaning in its accessible label and hover title, so operators do not have to infer that a routing-decision link opens Records. Attempt-only cause links first resolve the attempt to its invocation, then expand that invocation and focus the matching attempt. Records resolves compound targets directly, clears the default date bounds for exact targets, and expands the matching invocation detail instead of showing a broad search result. Historical events without the persisted score snapshot explicitly say that the comparison cannot be verified.

## Multi-Proxy Binding Update

- `prompt_cache_conversation_bindings.forward_proxy_keys_json` stores the conversation-local list while `forward_proxy_key` remains a legacy single-node compatibility column.
- PATCH accepts `forwardProxyKeys`; missing preserves the current list, `null` or an empty list clears it, and non-empty lists are canonicalized and validated against selectable existing binding nodes.
- GET returns both `forwardProxyKey` and `forwardProxyKeys`, with `forwardProxyKey` reflecting the first explicit key for compatibility.
- Runtime maps conversation lists to a `conversation:<promptCacheKey>` bound proxy scope. This scope is sticky to the current node and fails over only inside the explicit list after the existing consecutive network-failure threshold.
- Conversation proxy overrides outrank account and group proxy bindings. If the explicit conversation list has no selectable nodes at dispatch time, routing fails instead of falling back.

## Sticky Escape Update

- Candidate loading now inspects the latest two terminal pool `/v1/responses` attempts per upstream account and produces a shared account-to-expiry state only when both latest terminal attempts are `upstream_stream_error` and both occurred in the 300-second window. The active interval is `now < latest_failure_occurred_at + 300 seconds`; the exact boundary is expired.
- The escape signal is account-global for automatic routing, so different sticky keys stop reusing the same bad account once the threshold is reached.
- Routing and account roster/detail enrichment consume the same state map. Active entries expose `routingBlockUntil`, reason code `recent_upstream_stream_errors`, a readable message, `workStatus='degraded'`, and `healthStatus='normal'`; node-shunt-unassigned is still a higher-priority hard block without an expiry.
- The account-pool list and detail surfaces render a localized reason and shared `mm:ss` countdown, using one second-level clock only while an active expiry exists and one silent refresh when it reaches zero.
- Explicit `upstream_account` bindings ignore the automatic escape signal and continue to behave as operator overrides.
- `group` bindings keep the group constraint but may rotate from a failed sticky account to another eligible account inside the same group.

## Verification

- `cd web && bunx vitest run --project=unit src/features/account-pool/EffectiveRoutingRuleCard.test.tsx`
- `cd web && bunx vitest run --project=unit src/features/prompt-cache/PromptCacheConversationTable.test.tsx`
- `cd web && bun run test -- src/features/account-pool/AccountDetailDrawerShell.test.tsx src/features/prompt-cache/PromptCacheConversationTable.test.tsx`
- `cd web && bunx vitest run --project=unit`
- `cd web && bun run build`
- `cd web && bun run test-storybook -- --run src/features/invocations/PoolAttemptRecordCard.stories.tsx`
- `cargo test --no-run`
- `cargo test prompt_cache_conversation_binding_patch_is_mutually_exclusive_and_clearable -- --nocapture`
- `cargo test prompt_cache_conversation_binding_patch_is_mutually_exclusive_and_clearable`
- `cargo test ensure_schema_preserves_prompt_cache_binding_timeouts_when_adding_policy_columns`
- `cargo test ensure_schema_migrates_pre_timeout_prompt_cache_binding_table`
- `cargo test resolver_applies_prompt_cache -- --nocapture`
- `cargo test resolver_forced_prompt_cache_account_binding -- --nocapture`
- `cargo test resolver_prompt_cache_group_binding_does_not_bypass_cut_in_policy -- --nocapture`
- `cargo test resolver_non_explicit_sticky_escape_cuts_out_after_two_recent_upstream_stream_errors -- --nocapture`
- `cargo test resolver_prompt_cache_group_binding_reselects_within_group_after_recent_stream_errors -- --nocapture`
- `cargo test resolver_explicit_prompt_cache_account_binding_keeps_operator_override_after_recent_stream_errors -- --nocapture`
- `cargo test prompt_cache_conversation_proxy_override_bypasses_node_shunt_group_slots -- --nocapture`
- `cargo test bulk_prompt_cache_conversation_bindings_bind_to_upstream_account_across_keys`
- `cargo test bulk_prompt_cache_conversation_bindings_bind_none_clears_only_manual_binding`
- `cargo test bulk_prompt_cache_conversation_bindings_clear_and_reset_affinity_removes_all_affinity_rows`
- `cargo test bulk_prompt_cache_conversation_bindings_set_fast_mode_rewrite_mode_preserves_binding_kind`
- `cargo test bulk_prompt_cache_conversation_bindings_reject_invalid_target_without_partial_writes`
- `cd web && bunx vitest run src/features/account-pool/EffectiveRoutingRuleCard.test.tsx src/features/prompt-cache/PromptCacheConversationTable.test.tsx`
- `cd web && bunx vitest run src/lib/api.test.ts src/features/prompt-cache/PromptCacheConversationTable.test.tsx`
- `cd web && bun run test -- --run PromptCacheConversationTable.test.tsx api.test.ts`
- `cd web && bun run test -- DashboardWorkingConversationsSection.test.tsx`
- `cargo test ensure_schema_creates_prompt_cache_conversation_operation_events_table -- --nocapture`
- `cargo test model_scoped_sticky_clear_cause_does_not_cross_models -- --nocapture`
- `cargo test prompt_cache_conversation_operation_events_list_filters_by_info_type -- --nocapture`
- `cargo test bulk_prompt_cache_conversation_bindings_set_fast_mode_rewrite_mode_preserves_binding_kind -- --nocapture`
- `cd web && npm test -- --run PromptCacheConversationTable.test.tsx`
- `cd web && bun run build`
- `cd web && bun run build-storybook`
- `cargo test subscriptions -- --nocapture`
- `cd web && bun run test -- src/features/prompt-cache/PromptCacheConversationTable.test.tsx src/hooks/useConversationDetailTopics.test.tsx`
- `cd web && bun run test-storybook -- PromptCacheConversationTable.stories.tsx`
- `cd web && bun run test -- PromptCacheConversationTable.test.tsx`
- `cd web && bun run test -- src/demo/event-handlers.test.ts src/demo/handlers.test.ts src/demo/runtime.test.ts`
- `cd web && npm run build`
- `cd web && bun run test-storybook -- --run PromptCacheConversationTable.stories.tsx DashboardWorkingConversationsSection.stories.tsx`
- `cd web && bunx vitest run src/features/invocations/InvocationTable.test.tsx src/features/prompt-cache/PromptCacheConversationTable.test.tsx`
- Web demo `attention` scene evidence: `./assets/dashboard-bulk-actions-selection-panel-web-demo.png` shows viewport-bottom bulk actions and modifier-key selection without entering persistent selection mode.
- Web demo `attention` scene evidence: `./assets/dashboard-bulk-route-bind-dropdown-open-current.png` shows the compact one-line route-bind dialog while the route-bind kind dropdown is expanded with `分组` and `上游账号` choices.
- Storybook `ConversationBulkRouteBindDialogRecentTargets` mock evidence: `./assets/dashboard-bulk-route-bind-recents-desktop-chrome-page.png` captures the desktop layout in Google Chrome after restoring the last successful upstream-account target, showing the compact recent-chip strip under the binding row.
- Storybook `ConversationBulkRouteBindDialogRecentTargetsMobile` mock evidence: `./assets/dashboard-bulk-route-bind-recents-mobile-chrome-page.png` captures the compact-width mobile layout in Google Chrome, keeping the same variable-width chip language while omitting the recent-chip tail that cannot fit in two rows and preserving the same MRU restoration contract.
- Storybook `ConversationBulkPanelOpen` mock evidence: `./assets/dashboard-bulk-clear-affinity-button-restored-storybook.png` shows the floating bulk red button restored to the standalone `清空绑定并重选` entry point instead of the pure manual-binding clear flow.
- Storybook `ConversationBulkClearConfirm` mock evidence: `./assets/dashboard-bulk-clear-binding-confirm-storybook.png` shows the Dashboard destructive confirmation copy as `清空绑定` / `确认清空绑定`, with no `重选` wording and with sticky route / owner lock preservation stated in the dialog body.
- Storybook `ConversationBulkClearConfirm` also asserts that dark-theme footer and destructive-callout surfaces stay below the light-story lightness threshold, preventing regressions where `dialog-chrome` or destructive surface tokens silently resolve from root-light values.
- Storybook `LargeHistoryVirtualizedDrawer` browser evidence: 15,000 total retained records, 50 initial drawer records, 100 after one scroll-triggered page, 28 mounted table rows, first page still visible at the nested table offset, account-binding combobox opened in about 169 ms.
- Storybook `DrawerBindingAndTimeouts` mock evidence: one drawer shows binding controls plus the timeout subpanel, with mixed `conversation/account/root` source badges, collapsed inherited rows, expanded conversation-owned timeout rows, and editable timeout-only persistence when `bindingKind='none'`.
- Storybook `DrawerBindingAndTimeouts` mock evidence: one drawer shows the “对话详情” title, conversation-level policy override rows with source badges, binding controls, and the timeout subpanel in the Settings tab.
- Storybook `DrawerBindingAndTimeouts` mock evidence now also shows a multi-node conversation proxy list and the visual evidence at `./assets/conversation-settings-multi-proxy-story.png`.
- Storybook `DrawerBindingAndTimeouts` mock evidence now also captures the widened detail drawer and account-style conversation routing form at `./assets/conversation-settings-wide-drawer-story.png`, including hidden account-only rows, expanded conversation-owned policy/timeouts, and the separate route-binding block.
- Storybook `DrawerOperations` mock evidence preserves the legacy affinity-reset recovery sequence, including historical all-model `stickyTargetCleared` rows. New full resets instead emit one all-model `affinityReset` event with per-bucket `stickyTransitions`, while the epoch fence prevents stale in-flight success from resurrecting any route.
- Storybook `DrawerRouting`, `DrawerRoutingMobile`, `DrawerRoutingResetConfirm`, `DrawerRoutingResetConfirmMobile`, and `DrawerOperations` evidence at `./assets/conversation-routing-*-storybook.png` covers the complete route view, all-model fallback plus normalized buckets, five fitted mobile tabs, a padded full-reset confirmation at desktop and `393x852`, and concrete model-filtered routing events.
- Storybook `DrawerOperations` mock evidence also shows the explicit corresponding-invocation Records link beside the routing-decision link, with the compound `attemptId` plus `invokeId` target.
- Storybook `build-storybook` now succeeds after the Storybook-local Vite plugin merge deduplicates repeated React plugins, so the `DrawerOperations` evidence path remains usable for future UI reviews.
- Web demo `/#/live` evidence: `./assets/conversation-detail-realtime-desktop.png` and `./assets/conversation-detail-realtime-mobile-393x852.png` show the Calls topic hydrated with a responding row and terminal rows at desktop and exact mobile viewport sizes.

## 101 Read-only Follow-up

- App log correlation stays on the existing `[DEBUG-stream-rootcause-20260706]` failure-only lines plus `x-cvm-invoke-id`; this change does not add new schema or widen the log surface.
- Database sampling focuses on new `codex_invocations` terminal failures where `payload.failureKind IN ('upstream_stream_error', 'downstream_closed')`, then checks `payload.streamFailureOrigin`, `payload.downstreamClosePhase`, `payload.downstreamWriteErrorKind`, `payload.lastUpstreamChunkGapMs`, and the existing `x-cvm-invoke-id` linkage.
- Gateway validation remains an ops-only step: correlate the same `x-cvm-invoke-id` across application rows, application failure logs, and JSON access logs to confirm that `downstream_closed` remains an `after_first_byte` body-drop/client-or-middlebox cluster while the application-side fix only targets repeated non-explicit `upstream_stream_error` reuse.
