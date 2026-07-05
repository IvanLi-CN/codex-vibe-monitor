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
- Preserve bounded partial body evidence on read timeout, client stream errors, and body-limit failures; do not retain the whole body in memory after switching to file-backed replay.
- Log `body_read_done`, `body_size_bucket`, `request_body_snapshot_kind`, and `live_first_reason` before materializing the snapshot for full parse/rewrite.
- Keep response streaming ordered as “forward chunk downstream first, finish raw writer later”; log `downstream_first_byte_elapsed` and `raw_response_write_elapsed` separately.
- Enable live-first for capture only when tests prove encrypted owner binding, prompt-cache binding, body rewrite, failover replay, raw completeness, and terminal record fields remain identical to fallback behavior.

## Guardrails / Reuse notes

- Do not claim a request is live-first just because it uses file-backed replay; upstream send still starts after full semantic checks unless eligibility is proven.
- Do not infer “no encrypted content” from a prefix scan; absence is only safe after full parse or an equivalent explicit contract.
- Do not drop or truncate raw payload as a performance optimization. If raw writer fails, log and classify it, but keep business response semantics separate.
- Keep fallback reason values stable enough for production log comparison.

## References

- `src/proxy/dispatch.rs`
- `src/proxy/request_entry.rs`
- `docs/specs/q8h3n-proxy-hot-path-streaming-stability/SPEC.md`
