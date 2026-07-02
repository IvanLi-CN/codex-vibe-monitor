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
- [x] Conversation overrides for upstream switching, FAST mode, image tool, available models, and one pinned forward-proxy binding node.
- [x] Forced upstream account binding bypasses sticky cut-in/cut-out policy while preserving health, quota, guard, concurrency, route-key, and forward-proxy checks.
- [x] Manual group binding bypasses sticky source cut-out policy while preserving target cut-in and target account eligibility.
- [x] Upstream account binding writes the corresponding sticky route immediately.
- [x] Prompt Cache conversation detail drawer controls.
- [x] Prompt Cache conversation detail drawer title and Settings tab policy controls with effective-value rows, source badges, and field-level edit/clear behavior.
- [x] Prompt Cache conversation timeout editor with source badges, collapsed inherited rows, and field-level expand/clear behavior aligned with the effective routing rule card.
- [x] Prompt Cache conversation history drawer loads retained invocation records in 50-row scroll pages instead of hydrating all pages on open.
- [x] `InvocationTable` virtualizes desktop table rows and mobile cards, mounting only the active breakpoint layout.
- [x] Unit, integration, Storybook, and visual evidence coverage.

## Verification

- `cargo test --no-run`
- `cargo test prompt_cache_conversation_binding_patch_is_mutually_exclusive_and_clearable -- --nocapture`
- `cargo test prompt_cache_conversation_binding_patch_is_mutually_exclusive_and_clearable`
- `cargo test ensure_schema_preserves_prompt_cache_binding_timeouts_when_adding_policy_columns`
- `cargo test ensure_schema_migrates_pre_timeout_prompt_cache_binding_table`
- `cargo test resolver_applies_prompt_cache -- --nocapture`
- `cargo test resolver_forced_prompt_cache_account_binding -- --nocapture`
- `cargo test resolver_prompt_cache_group_binding_does_not_bypass_cut_in_policy -- --nocapture`
- `cd web && bunx vitest run src/lib/api.test.ts src/components/PromptCacheConversationTable.test.tsx`
- `cd web && bun run test -- --run PromptCacheConversationTable.test.tsx api.test.ts`
- `cd web && bun run build`
- `cd web && bun run test-storybook -- --run PromptCacheConversationTable.stories.tsx DashboardWorkingConversationsSection.stories.tsx`
- `cd web && bunx vitest run src/components/InvocationTable.test.tsx src/components/PromptCacheConversationTable.test.tsx`
- Storybook `LargeHistoryVirtualizedDrawer` browser evidence: 15,000 total retained records, 50 initial drawer records, 100 after one scroll-triggered page, 28 mounted table rows, first page still visible at the nested table offset, account-binding combobox opened in about 169 ms.
- Storybook `DrawerBindingAndTimeouts` mock evidence: one drawer shows binding controls plus the timeout subpanel, with mixed `conversation/account/root` source badges, collapsed inherited rows, expanded conversation-owned timeout rows, and editable timeout-only persistence when `bindingKind='none'`.
- Storybook `DrawerBindingAndTimeouts` mock evidence: one drawer shows the “对话详情” title, conversation-level policy override rows with source badges, binding controls, and the timeout subpanel in the Settings tab.
