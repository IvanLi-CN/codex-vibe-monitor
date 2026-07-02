# Prompt Cache Conversation Bindings - History

## Key Decisions

- 2026-05-25: Created a dedicated topic spec because per-conversation routing bindings introduce a new stable runtime contract distinct from invocation observability.
- 2026-05-25: Chose hard-constraint routing semantics so bound conversations never silently fall back to unrelated accounts or groups.
- 2026-05-25: Added Storybook coverage and visual evidence for the drawer binding panel to keep the UI contract reviewable.
- 2026-05-27: Clarified that manual Prompt Cache account and group bindings are the only supported way to move a conversation out of a sticky source whose policy forbids cut-out; group targets still honor target cut-in policy.
- 2026-05-28: Locked retained-history drawer paging to 50 records per page and moved `InvocationTable` to active-breakpoint virtualization after a 15,000-record drawer proved that full-page hydration and dual hidden layouts could block route-binding controls.
- 2026-06-29: Expanded the binding row contract to support timeout-only `binding_kind='none'` rows. Conversation timeout overrides now persist independently from the manual binding target, are applied after target account selection, and do not trip encrypted-session owner guard logic.
- 2026-07-01: Aligned the conversation timeout editor with the effective-rule interaction model. Timeout rows now render as collapsed summaries until the conversation layer overrides a field, while timeout-only `binding_kind='none'` rows still preserve their explicit per-field override state.
- 2026-07-02: Expanded conversation bindings into runtime policy overrides. The Settings tab can now override upstream switching (“切出”), FAST mode, image tool, available models, and one existing forward-proxy binding node for the current prompt cache key.
- Upstream account bindings are operator-forced account assignments: they override sticky transfer policy only, not account health, quota, guard, concurrency, route-key, or forward-proxy readiness.
- “切出” is a conversation-level permission to switch away from the original/sticky upstream account, preserving user-facing terminology from account settings without introducing a conversation-level cut-in permission.
