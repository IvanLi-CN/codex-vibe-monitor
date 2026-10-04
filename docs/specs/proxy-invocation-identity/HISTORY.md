# Proxy Invocation Identity History

> This file records topic-local compatibility and lifecycle context. The durable decision rationale remains in `docs/adr/0020-proxy-invocation-identity.md`.

## Lifecycle / Compatibility

- Introduced as a backend-only identity boundary. Existing invocation rows remain historical input and are not rewritten.
- Public API and frontend consumers are intentionally reserved for a follow-up contract.

## Replacements / Background

- The prompt-cache conversation master replaces request-time derivation as the durable owner of conversation identity and delayed aggregate statistics. Historical materialization is completed by an ordered background task; its 400-key logical pages use adaptive committed micro-batches and yield only at transaction boundaries.
- WebSocket pre-upstream and per-turn diagnostics now use the same compact identifier contract as HTTP proxy capture.

## Allocation Reservation Boundary

- The range reservation contract assigns the conversation master's `last_invoke_sequence` to committed reservation authority, independently of actual invocation statistics. This corrects the gap where avoiding existence queries still allowed a database transaction for every issued sequence; current implementation gaps remain explicit in `IMPLEMENTATION.md`.
- Cache sizing starts at the minimum before a bounded asynchronous history seed, then follows memory-only hourly activity estimates. Eviction attempts to return never-issued tails within a fixed budget; uncertain returns preserve durable recovery rather than reusing memory ranges.
- [ADR 0029](../../adr/0029-conversation-invocation-range-reservations.md) locks the 64-sequence reservation, refill, return, and bounded cache contract. [ADR 0030](../../adr/0030-durable-hourly-invocation-prefixes.md) places unbound hourly ownership in the business SQLite database and the same manager. This supersedes the process-local-only hourly authority described below without rewriting historical IDs.
- The allocation correction preserves known legacy sequence floors on demand, keeps structural migration separate from statistics materialization, and uses forward repair for stopped releases. Earlier writers unaware of reservation ownership are outside the newly migrated state's supported writer contract; planned migration and compatibility evidence are recorded separately from implementation results.

## Compatibility Follow-up

- Proxy allocation now uses per-normalized-key async locks with weak idle-entry cleanup, registers active references before waits, and releases them on failure or cancellation. The global cache mutex is held only for short in-memory operations; identity and sequence SQL use one `InteractiveProxy` admission and cache state is updated after admission is released.
- Unbound hourly prefixes are protected by an independent process-local namespace during initialization and exclude issued prefixes, conversation masters, and invocation prefixes. The route-health latest-success lookup gains one idempotent partial covering index; no public response, account state, timeout, retry budget, or existing business row is changed.
- The long-wait acceptance harness now covers same-key, different-key, unbound, and cancelled calls with 179-second success and 180-second timeout-boundary samples, while observing each fast terminal promptly and requiring the cancelled invocation to persist a downstream-closed/client-abort terminal row within a bounded client-close duration.
- Prompt-cache materialization enablement now has one control source: the task's maintenance-database `managed_tasks.enabled` row. Both control PATCH paths update it and its maintenance checkpoint atomically, then publish a process-local generation; both return the existing 503 Unavailable response when the control transaction fails. A one-time origin marker preserves an existing maintenance control; the old business-database enable bit may seed only a newly created managed row, is never consulted by the run path, and is not mirrored. Incomplete statistics pages retain their committed staging cursor and resume after a bounded 15-second follow-up; only actual disablement reports `operator_disabled`, and stale same-generation wakes do not erase a pending deadline. Existing HTTP fields and table structure remain unchanged.
- The control-consistency acceptance observer uses current service-process pressure-denial logs and scheduled retry windows to distinguish pressure from enabled, eligible work; historical defer reasons cannot exempt later stalls. Pressure deadlines retain fractional seconds through their exact expiry boundary, covered by `python3 .github/scripts/test-prompt-cache-control-progress-clock.py`. It preserves the separate exact completion, empty-queue, complete-statistics, and online latency gates so a priority wait cannot be treated as successful completion.
- The service fixture explicitly queues an interactive writer behind a synthetic locked business step and requires a fresh priority-defer result before steady input; probe calls remain included in the allocation and terminal latency gates.
- The live Prompt working-set update trigger now declares its projection dependencies, avoiding redundant history scans for the terminal writer's timing-only follow-up. Existing installations receive the corrected definitions and an additive completion marker atomically, without changing source rows or rebuilding an already initialized projection. Relevant source updates retain old/new-key reconciliation and the existing live-window guard.

- Bound maintenance checkpoint finalization and wake persistence to the committed control generation, preserving newer pause/resume checkpoints and scheduler wakes across late results.

## Related Changes

- `8d39f04c`: unified materialization control and typed statistics continuation.
- `14a3f800`: preserved bounded retry deadlines across same-generation wakeups.
- [ADR 0027](../../adr/0027-prompt-cache-materialization-step-boundaries.md) records the short identity/statistics steps authorized by the control-consistency repair, including single ownership of large-key statistics scans.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`

- Mainline integration preserves the generation-fenced disabled-result path and constructs maintenance test stores through the shared injected-control constructor.
- The statistics rebuild cursor used to wait for the full logical key batch. A completed large conversation could have its staging deleted, then be selected and scanned again whenever a later large key yielded. The successor rule in [ADR 0028](../../adr/0028-prompt-cache-continuous-statistics-checkpoints.md) advances the continuous key prefix in the same transaction as final-page aggregate publication and matching queue/staging deletion; queue drain continues to use queue deletion as its checkpoint. Unchanged-source pages continue inside the existing run/query budgets, and incomplete statistics ETA is null. Existing 2.85.1 partial staging and empty outer cursors remain directly resumable; no durable schema or API fields change.
