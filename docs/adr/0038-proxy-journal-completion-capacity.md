---
status: accepted
---

# ADR 0038: Proxy journal completion capacity

This decision refines the capacity admission in [ADR 0037](0037-isolated-supplemental-recovery-and-continuous-attempt-reads.md). Its recovery policy, durability confirmation, runtime read ownership, routing fences and SQLite materialization boundaries remain applicable.

## Context

Reserving a maximum 4 MiB frame for each unmatched begin exhausts the 512 MiB journal budget at 127 small concurrent invocations, while the runtime store still has room and the actual journal contains only about 214 KiB. Removing reservations would let new requests consume space needed by already accepted requests. Increasing the disk cap alone does not resolve this coupling.

## Decision

The journal owns a completion-credit ledger keyed by stable attempt public ID. It reserves persisted metadata strings with six bytes per source byte for worst-case JSON escaping, a 4096-byte schema/numeric envelope, 64 KiB times six terminal-growth headroom, and 256 bytes for a maximum-width acknowledgement frame. URL metadata additionally allows threefold percent-encoding before JSON escaping. Redacted bodies, summaries, raw errors and credentials contribute no persisted bytes. The maximum valid frame remains 4 MiB; an encoded frame is always checked against this limit before append.

Each invocation identity additionally owns 64 KiB times six envelope headroom, shared by its retries and released only with its last credited attempt. Producer-selected request models and route metadata are counted in attempt credit; response-capture paths and retained metadata changes acquire increased credit before publication under the runtime-store lock. Credit does not shrink on progress updates, so another start cannot steal an accepted request's completion allowance. Ordinary terminal status, fixed timestamps, interruption reasons, timing and byte counters fit the terminal headroom. The runtime store keeps its independent 32 MiB hard bound and 64 KiB unfinished-attempt memory allowance.

Admission compares actual retained journal bytes plus all outstanding credit plus the newly encoded begin against 512 MiB. Terminal admission subtracts only the credit for starts acknowledged by that terminal, then includes the actual encoded terminal and its deduplicated ACK frames. Credit release occurs only after successful append of each ACK. Partial append and sync failures retain their existing fail-closed confirmation boundary. Capacity refusal is observable and may recover after backlog drains; persistent append failures cannot be cleared by a low byte count.

Invocation envelopes can contain provider metadata or multiple attempt snapshots beyond the ordinary credited growth. The exact terminal frame and aggregate disk-bound checks remain authoritative: oversized or uncredited critical terminals enter the explicit warning-visible recovery/backpressure path, retain their recovery obligation, and cannot be acknowledged as memory-only success. This decision neither truncates terminal business metadata nor increases body, replay or disk limits.

The ledger is volatile bookkeeping rebuilt from unmatched starts in the existing journal during startup; it introduces no SQLite schema or journal-format change. No credit bookkeeping acquires SQLite write admission. Reserved bytes are exported separately from actual journal bytes, with no request identity labels and without changing the HTTP/SSE response shape.

## Validation

- 256 independent small attempts complete in reverse order or recover as interrupted after restart while remaining within memory and disk bounds.
- Metadata growth is rejected before runtime publication at the hard capacity boundary, preserves the prior snapshot, and succeeds after capacity becomes available.
- Retry attempts share one invocation envelope and duplicate ACK identities cannot release credit twice.
- Escaped metadata, terminal growth, maximum numeric widths and acknowledgement encoding fit credited capacity.
- The unchanged 32-request SQLite-pressure scenario requires zero InteractiveProxy lifecycle admission and zero AttemptProgress SQL, p95 below two seconds, and complete terminal commit below five seconds.
