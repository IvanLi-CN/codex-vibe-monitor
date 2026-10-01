# Proxy Invocation Identity History

> This file records topic-local compatibility and lifecycle context. The durable decision rationale remains in `docs/adr/0020-proxy-invocation-identity.md`.

## Lifecycle / Compatibility

- Introduced as a backend-only identity boundary. Existing invocation rows remain historical input and are not rewritten.
- Public API and frontend consumers are intentionally reserved for a follow-up contract.

## Replacements / Background

- The prompt-cache conversation master replaces request-time derivation as the durable owner of conversation identity and delayed aggregate statistics. Historical materialization is completed by an ordered background task; its 400-key logical pages use adaptive committed micro-batches and yield only at transaction boundaries.
- WebSocket pre-upstream and per-turn diagnostics now use the same compact identifier contract as HTTP proxy capture.

## Compatibility Follow-up

- Proxy allocation now uses per-normalized-key async locks with weak idle-entry cleanup, registers active references before waits, and releases them on failure or cancellation. The global cache mutex is held only for short in-memory operations; identity and sequence SQL use one `InteractiveProxy` admission and cache state is updated after admission is released.
- Unbound hourly prefixes are protected by an independent process-local namespace during initialization and exclude issued prefixes, conversation masters, and invocation prefixes. The route-health latest-success lookup gains one idempotent partial covering index; no public response, account state, timeout, retry budget, or existing business row is changed.
- The long-wait acceptance harness now covers same-key, different-key, unbound, and cancelled calls with 179-second success and 180-second timeout-boundary samples, while observing each fast terminal promptly and gating client cancellation status and duration.

## Related Changes

- None recorded until delivery creates the signed-off commit and pull request.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
