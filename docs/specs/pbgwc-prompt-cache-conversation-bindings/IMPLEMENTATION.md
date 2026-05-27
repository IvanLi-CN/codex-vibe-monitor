# Prompt Cache Conversation Bindings - Implementation

## Current State

- Status: implemented
- Canonical spec: `docs/specs/pbgwc-prompt-cache-conversation-bindings/SPEC.md`

## Delivery Checklist

- [x] Storage schema for `prompt_cache_conversation_bindings`.
- [x] HTTP binding API with validation.
- [x] Account-pool runtime routing constraints.
- [x] Forced upstream account binding bypasses sticky cut-in/cut-out policy while preserving health, quota, guard, concurrency, route-key, and forward-proxy checks.
- [x] Upstream account binding writes the corresponding sticky route immediately.
- [x] Prompt Cache conversation detail drawer controls.
- [x] Unit, integration, Storybook, and visual evidence coverage.

## Verification

- `cargo test --no-run`
- `cargo test prompt_cache_conversation_binding_patch_is_mutually_exclusive_and_clearable -- --nocapture`
- `cargo test resolver_applies_prompt_cache -- --nocapture`
- `cargo test resolver_forced_prompt_cache_account_binding -- --nocapture`
- `cargo test resolver_prompt_cache_group_binding_does_not_bypass_cut_in_policy -- --nocapture`
- `cd web && bunx vitest run src/lib/api.test.ts src/components/PromptCacheConversationTable.test.tsx`
- `cd web && bun run build`
