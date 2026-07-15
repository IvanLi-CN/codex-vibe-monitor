# Prompt Cache Conversation Bindings - Implementation

## Current State

- Status: implemented
- Canonical spec: `docs/specs/pbgwc-prompt-cache-conversation-bindings/SPEC.md`

## Delivery Checklist

- [x] Storage schema for `prompt_cache_conversation_bindings`.
- [x] HTTP binding API with validation.
- [x] Conversation timeout-only rows and per-field timeout PATCH semantics.
- [x] Conversation runtime policy override columns and per-field PATCH semantics.
- [x] Account-pool runtime routing constraints.
- [x] Conversation overrides for upstream switching, FAST mode, image tool, available models, and a hard multi-node forward-proxy binding list.
- [x] Forced upstream account binding bypasses sticky cut-in/cut-out policy while preserving health, quota, guard, concurrency, route-key, and forward-proxy checks.
- [x] Manual group binding bypasses sticky source cut-out policy while preserving target cut-in and target account eligibility.
- [x] Automatic sticky escape for non-explicit routes after account-global consecutive transport/decode-shaped `upstream_stream_error` failures, while preserving explicit upstream-account operator overrides and group-only reselection semantics.
- [x] Upstream account binding writes the corresponding sticky route immediately.
- [x] Prompt Cache conversation detail drawer controls.
- [x] Prompt Cache conversation detail drawer title and Settings tab policy controls with effective-value rows, source badges, and field-level edit/clear behavior.
- [x] Prompt Cache conversation detail drawer reuses the account-detail wide shell width class and the shared effective-routing form skeleton, while hiding account-only routing rows on the conversation surface.
- [x] FAST mode and image tool editors expose only concrete rewrite choices and remain expanded after a successful choice save.
- [x] Prompt Cache conversation timeout editor with source badges, collapsed inherited rows, and field-level expand/clear behavior aligned with the effective routing rule card.
- [x] Prompt Cache conversation history drawer loads retained invocation records in 50-row scroll pages instead of hydrating all pages on open.
- [x] `InvocationTable` virtualizes desktop table rows and mobile cards, mounting only the active breakpoint layout.
- [x] Unit, integration, Storybook, and visual evidence coverage.

## Multi-Proxy Binding Update

- `prompt_cache_conversation_bindings.forward_proxy_keys_json` stores the conversation-local list while `forward_proxy_key` remains a legacy single-node compatibility column.
- PATCH accepts `forwardProxyKeys`; missing preserves the current list, `null` or an empty list clears it, and non-empty lists are canonicalized and validated against selectable existing binding nodes.
- GET returns both `forwardProxyKey` and `forwardProxyKeys`, with `forwardProxyKey` reflecting the first explicit key for compatibility.
- Runtime maps conversation lists to a `conversation:<promptCacheKey>` bound proxy scope. This scope is sticky to the current node and fails over only inside the explicit list after the existing consecutive network-failure threshold.
- Conversation proxy overrides outrank account and group proxy bindings. If the explicit conversation list has no selectable nodes at dispatch time, routing fails instead of falling back.

## Sticky Escape Update

- Candidate loading now inspects the latest two terminal pool `/v1/responses` attempts per upstream account and marks the account for automatic sticky escape when both failures are `upstream_stream_error`.
- The escape signal is account-global for automatic routing, so different sticky keys stop reusing the same bad account once the threshold is reached.
- Explicit `upstream_account` bindings ignore the automatic escape signal and continue to behave as operator overrides.
- `group` bindings keep the group constraint but may rotate from a failed sticky account to another eligible account inside the same group.

## Verification

- `cd web && bunx vitest run --project=unit src/features/account-pool/EffectiveRoutingRuleCard.test.tsx`
- `cd web && bunx vitest run --project=unit src/features/prompt-cache/PromptCacheConversationTable.test.tsx`
- `cd web && bun run test -- src/features/account-pool/AccountDetailDrawerShell.test.tsx src/features/prompt-cache/PromptCacheConversationTable.test.tsx`
- `cd web && bunx vitest run --project=unit`
- `cd web && bun run build`
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
- `cd web && bunx vitest run src/features/account-pool/EffectiveRoutingRuleCard.test.tsx src/features/prompt-cache/PromptCacheConversationTable.test.tsx`
- `cd web && bunx vitest run src/lib/api.test.ts src/features/prompt-cache/PromptCacheConversationTable.test.tsx`
- `cd web && bun run test -- --run PromptCacheConversationTable.test.tsx api.test.ts`
- `cd web && npm test -- --run PromptCacheConversationTable.test.tsx`
- `cd web && bun run build`
- `cd web && npm run build`
- `cd web && bun run test-storybook -- --run PromptCacheConversationTable.stories.tsx DashboardWorkingConversationsSection.stories.tsx`
- `cd web && bunx vitest run src/features/invocations/InvocationTable.test.tsx src/features/prompt-cache/PromptCacheConversationTable.test.tsx`
- Storybook `LargeHistoryVirtualizedDrawer` browser evidence: 15,000 total retained records, 50 initial drawer records, 100 after one scroll-triggered page, 28 mounted table rows, first page still visible at the nested table offset, account-binding combobox opened in about 169 ms.
- Storybook `DrawerBindingAndTimeouts` mock evidence: one drawer shows binding controls plus the timeout subpanel, with mixed `conversation/account/root` source badges, collapsed inherited rows, expanded conversation-owned timeout rows, and editable timeout-only persistence when `bindingKind='none'`.
- Storybook `DrawerBindingAndTimeouts` mock evidence: one drawer shows the “对话详情” title, conversation-level policy override rows with source badges, binding controls, and the timeout subpanel in the Settings tab.
- Storybook `DrawerBindingAndTimeouts` mock evidence now also shows a multi-node conversation proxy list and the visual evidence at `./assets/conversation-settings-multi-proxy-story.png`.
- Storybook `DrawerBindingAndTimeouts` mock evidence now also captures the widened detail drawer and account-style conversation routing form at `./assets/conversation-settings-wide-drawer-story.png`, including hidden account-only rows, expanded conversation-owned policy/timeouts, and the separate route-binding block.

## 101 Read-only Follow-up

- App log correlation stays on the existing `[DEBUG-stream-rootcause-20260706]` failure-only lines plus `x-cvm-invoke-id`; this change does not add new schema or widen the log surface.
- Database sampling focuses on new `codex_invocations` terminal failures where `payload.failureKind IN ('upstream_stream_error', 'downstream_closed')`, then checks `payload.streamFailureOrigin`, `payload.downstreamClosePhase`, `payload.downstreamWriteErrorKind`, `payload.lastUpstreamChunkGapMs`, and the existing `x-cvm-invoke-id` linkage.
- Gateway validation remains an ops-only step: correlate the same `x-cvm-invoke-id` across application rows, application failure logs, and JSON access logs to confirm that `downstream_closed` remains an `after_first_byte` body-drop/client-or-middlebox cluster while the application-side fix only targets repeated non-explicit `upstream_stream_error` reuse.
