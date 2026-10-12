---
status: accepted
---

# Dashboard incremental SSE frames

The Dashboard keeps its existing snapshot payload and owner-facing semantics, but live and replay delivery may use a backward-compatible delta envelope after the initial snapshot. This is accepted because the production baseline shows that complete working-conversations and open-window timeseries frames alone exceed the browser budget; the delta path must demonstrate a material reduction in frontend JSON parsing, state commits, and main-thread blocking before it can replace full-frame delivery for a topic.

The browser remains responsible for merging a validated delta into the existing topic model, while the server remains responsible for revision ordering, replay continuity, and falling back to a complete snapshot when a delta cannot be applied safely.
