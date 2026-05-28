# Prompt Cache Conversation Bindings - History

## Key Decisions

- 2026-05-25: Created a dedicated topic spec because per-conversation routing bindings introduce a new stable runtime contract distinct from invocation observability.
- 2026-05-25: Chose hard-constraint routing semantics so bound conversations never silently fall back to unrelated accounts or groups.
- 2026-05-25: Added Storybook coverage and visual evidence for the drawer binding panel to keep the UI contract reviewable.
- 2026-05-27: Clarified that manual Prompt Cache account and group bindings are the only supported way to move a conversation out of a sticky source whose policy forbids cut-out; group targets still honor target cut-in policy.
- Upstream account bindings are operator-forced account assignments: they override sticky transfer policy only, not account health, quota, guard, concurrency, route-key, or forward-proxy readiness.
