# Proxy Invocation Identity History

> This file records topic-local compatibility and lifecycle context. The durable decision rationale remains in `docs/adr/0020-proxy-invocation-identity.md`.

## Lifecycle / Compatibility

- Introduced as a backend-only identity boundary. Existing invocation rows remain historical input and are not rewritten.
- Public API and frontend consumers are intentionally reserved for a follow-up contract.

## Replacements / Background

- The prompt-cache conversation master replaces request-time derivation as the durable owner of conversation identity and delayed aggregate statistics.
- WebSocket pre-upstream and per-turn diagnostics now use the same compact identifier contract as HTTP proxy capture.

## Related Changes

- None recorded until delivery creates the signed-off commit and pull request.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
