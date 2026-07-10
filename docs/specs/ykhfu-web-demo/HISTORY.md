# 全产品路由 Web Demo 演进历史（#ykhfu）

> 这里记录影响长期理解的关键演进原因；规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- Demo is a distinct build-time runtime selected by `VITE_APP_RUNTIME`, not a route-level switch.
- GitHub Pages hosts docs, Storybook and the demo as separate static surfaces in one assembled artifact.

## Key Reasons / Replacements

- Storybook remains component QA; the demo owns end-to-end product-route preview and interaction evidence.
- The existing Records Overlay E2E context keeps its live Vite fixture regression and starts a separate demo Vite server for mock-runtime coverage; this avoids MSW overriding the fixture contract while preserving the required check name.
- MSW HTTP handlers are importable in node tests, while the SSE handler is browser-only so unit tests do not require the browser EventSource API.
- GitHub Pages scopes service workers to the demo deployment base. Demo API, SSE and public asset URLs therefore remain under that base, while handlers normalize the scoped API pathname back to the existing API contract.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
