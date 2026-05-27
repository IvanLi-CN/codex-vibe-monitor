# Prompt Cache Conversation Bindings - History

## Key Decisions

- 2026-05-25: Created a dedicated topic spec because per-conversation routing bindings introduce a new stable runtime contract distinct from invocation observability.
- 2026-05-25: Chose hard-constraint routing semantics so bound conversations never silently fall back to unrelated accounts or groups.
- 2026-05-25: Added Storybook coverage and visual evidence for the drawer binding panel to keep the UI contract reviewable.
- Upstream account bindings are operator-forced account assignments: they override sticky transfer policy only, not account health, quota, guard, concurrency, route-key, or forward-proxy readiness.
