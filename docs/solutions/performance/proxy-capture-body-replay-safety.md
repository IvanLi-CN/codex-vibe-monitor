---
title: Proxy capture body replay safety
module: proxy
problem_type: performance
component: OpenAI-compatible proxy capture
tags:
  - proxy
  - raw
  - replay
  - streaming
status: active
related_specs:
  - docs/specs/q8h3n-proxy-hot-path-streaming-stability/SPEC.md
---

# Proxy capture body replay safety

## Context

Proxy capture endpoints need to forward requests quickly, but they also own raw payload retention, usage parsing, prompt-cache routing, encrypted-session owner binding, body rewrite, failover replay, terminal overlay, and failure classification.

## Symptoms

- Large request bodies make the first upstream attempt look late because capture waits for request body read and parse before sending.
- Operators need to know whether latency is body read, route context, upstream first byte, raw IO, or record flush.
- A tempting optimization is to stream every body immediately, but that can send encrypted or rewritten requests through the wrong account or lose replay safety.

## Root cause

The capture path historically used one full in-memory request body as both routing input and raw capture input. That makes large bodies expensive, but replacing it with unconditional live-first would bypass semantic checks that require full request knowledge.

## Resolution

- Put capture request reads on the same replay snapshot control plane as pool routing: memory for small bodies, file-backed replay for large bodies.
- Centralize every `Bytes` / `Vec<u8>` to `PoolReplayBodySnapshot` conversion in one threshold helper. Use memory only at or below `POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES`; above the threshold, write a `cvm-pool-replay-*` temp file and return a file-backed snapshot.
- Replace direct large-body `PoolReplayBodySnapshot::Memory(...)` construction in capture outbound, route-selection prebuffer fallback, and rewritten-body paths. A direct memory snapshot is only acceptable for small bodies, empty bodies, or fail-soft temp-file failure.
- Preserve bounded partial body evidence on read timeout, client stream errors, and body-limit failures; do not retain the whole body in memory after switching to file-backed replay.
- Consume file-backed snapshots with a single materialization step only when the existing capture semantics require full JSON parse/rewrite; do not add an extra `Bytes -> Vec` full-body copy.
- If rewrite is required but produces no body changes, return the original snapshot instead of serializing the body back into memory. If rewrite changes the body, pass the rewritten bytes through the same threshold helper.
- Log `body_read_done`, `body_size_bucket`, `request_body_snapshot_kind`, and `live_first_reason` before materializing the snapshot for full parse/rewrite.
- Keep response streaming ordered as “forward chunk downstream first, finish raw writer later”; log `downstream_first_byte_elapsed` and `raw_response_write_elapsed` separately.
- Make production evidence thresholded: large or slow request body reads, slow downstream first byte, and slow or large raw response writes should be visible at `info`; ordinary small requests can remain `debug`.
- Enable live-first for capture only when tests prove encrypted owner binding, prompt-cache binding, body rewrite, failover replay, raw completeness, and terminal record fields remain identical to fallback behavior.
- Treat direct-image replay as evidence retention, not permission to retry. Image generation/edit may have started before the first response byte, so a first-byte timeout must terminate after one attempt and preserve the real timeout classification.

## Guardrails / Reuse notes

- Do not claim a request is live-first just because it uses file-backed replay; upstream send still starts after full semantic checks unless eligibility is proven.
- File-backed replay is not zero-copy for capture until the downstream capture pipeline can parse, rewrite, raw-capture, and failover from a shared replay snapshot without rebuilding a full request body.
- Do not infer “no encrypted content” from a prefix scan; absence is only safe after full parse or an equivalent explicit contract.
- Do not drop or truncate raw payload as a performance optimization. If raw writer fails, log and classify it, but keep business response semantics separate.
- Keep fallback reason values stable enough for production log comparison.
- Treat `snapshot_kind="memory"` on multi-MiB requests as a regression unless the same log window shows a temp-file create/write/flush warning explaining the fail-soft fallback.
- Do not convert a terminal direct-image timeout into a generic no-account result; return a traceable gateway timeout and release the account reservation without replaying the body.

## References

- `src/proxy/dispatch.rs`
- `src/proxy/request_entry.rs`
- `docs/specs/q8h3n-proxy-hot-path-streaming-stability/SPEC.md`
