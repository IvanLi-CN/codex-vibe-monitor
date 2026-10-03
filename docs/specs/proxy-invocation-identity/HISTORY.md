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
- The long-wait acceptance harness now covers same-key, different-key, unbound, and cancelled calls with 179-second success and 180-second timeout-boundary samples, while observing each fast terminal promptly and requiring the cancelled invocation to persist a downstream-closed/client-abort terminal row within a bounded client-close duration.
- Prompt-cache materialization enablement now has one control source: the task's maintenance-database `managed_tasks.enabled` row. Both control PATCH paths update it and its maintenance checkpoint atomically, then publish a process-local generation; both return the existing 503 Unavailable response when the control transaction fails. A one-time origin marker preserves an existing maintenance control; the old business-database enable bit may seed only a newly created managed row, is never consulted by the run path, and is not mirrored. Incomplete statistics pages retain their committed staging cursor and resume after a bounded 15-second follow-up; only actual disablement reports `operator_disabled`, and stale same-generation wakes do not erase a pending deadline. Existing HTTP fields and table structure remain unchanged.
- The control-consistency acceptance observer distinguishes scheduled retry windows and notification-driven coordinator pressure from enabled, eligible work. It preserves the separate exact completion, empty-queue, complete-statistics, and online latency gates so a priority wait cannot be treated as successful completion.

- Bound maintenance checkpoint finalization and wake persistence to the committed control generation, preserving newer pause/resume checkpoints and scheduler wakes across late results.

## Related Changes

- `8d39f04c`: unified materialization control and typed statistics continuation.
- `14a3f800`: preserved bounded retry deadlines across same-generation wakeups.
- [ADR 0027](../../adr/0027-prompt-cache-materialization-step-boundaries.md) records the short identity/statistics steps authorized by the control-consistency repair, including single ownership of large-key statistics scans.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
