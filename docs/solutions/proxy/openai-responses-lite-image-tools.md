---
title: OpenAI Responses Lite image-tool boundaries
module: proxy
problem_type: protocol_compatibility
component: responses_image_tools
tags: [openai, responses-lite, image-tools, proxy]
status: active
related_specs: [r4p9x, z9h7v, pbgwc]
---

# Codex Responses Imagegen Boundaries

## Context

Codex Responses Lite is not a Full Responses payload with a different model name. Codex owns the image-generation executor; CVM only advertises its `image_gen.imagegen` namespace using the protocol-specific request shape.

## Symptoms

A proxy that routes Codex through hosted `image_generation` can make a valid Codex request return a result the Codex UI cannot render. A Lite request can also fail validation when a top-level hosted tool is injected. Neither failure is account capability evidence.

## Root Cause

The legacy hosted rewrite policy was applied before Codex protocol handling. It advertised a server-hosted executor to a client that expects its own `image_gen.imagegen` namespace and result path.

## Resolution

1. Detect Lite exclusively from `X-OpenAI-Internal-Codex-Responses-Lite: true`; detect Full only from `originator: Codex Desktop` or a `Codex Desktop/…` user agent. Do not infer either from a model, body shape, or session id.
2. Apply the independent `codexImagegenRewriteMode`: `keep_original | fill_missing | force_add | force_remove`. Root defaults to `keep_original`; group, account, and conversation can inherit or override it.
3. For Full, merge the fixed `image_gen.imagegen` snapshot into top-level `tools`. For Lite, normalize `input` to an array, merge developer `additional_tools`, set `reasoning.context=all_turns`, and set `parallel_tool_calls=false`.
4. When the Codex policy is not `keep_original`, remove hosted `image_generation` and its matching `tool_choice` before applying the Codex policy. Preserve unrelated namespaces and tools.
5. Use the OpenAI Codex commit `61a44880a85d2fd0d8770908dea5733495e571c8` schema snapshot. `fill_missing` preserves an existing same-name tool; `force_add` replaces it and records fingerprints plus differing JSON paths.
6. Persist `codexImagegenRewrite` on the originating workflow attempt as well as the invocation summary, so failover rows never inherit the final account's audit. It contains protocol, client match, effective mode, outcome, hosted removal, snapshot fingerprint, and conflict-only fingerprints/diff paths. It never contains prompts, image bytes, or full requests.
7. Treat the namespace as its own upstream capability. If an actual injection receives the known `502 Upstream request failed` response, mark that account unsupported and fail over without retrying it; do not downgrade the request by silently removing the namespace or record the shape mismatch as generic account health failure. Preserve the observed reason and expose an explicit operator override for deliberate recovery after the upstream is fixed; do not infer recovery from a request that did not inject the namespace.

## Error and Retry Boundary

Responses Lite protocol detection and upstream error retry classification are separate concerns. A successful HTTP status can still carry a terminal `response.failed` event, so the proxy must inspect the structured upstream error before deciding whether to forward or retry.

- `server_is_overloaded` remains an explicitly retryable transient overload.
- `rate_limit_exceeded` enters the same-account Responses retry budget only when the accompanying message explicitly describes a concurrency limit being exceeded (or too many concurrent requests).
- A generic RPM, quota, billing, or account-rate-limit message remains a normal upstream failure; it must not be inferred to be an account-level concurrency condition or silently converted into the overload cooldown path.
- The same narrow classifier is used for streamed `/v1/responses` and JSON `/v1/responses/compact`, preserving the existing same-account retry budget before route failover.

## Guardrails

- Do not infer Lite from `gpt-5.6` or any model identifier.
- Do not implement an image executor, response-stream conversion, or historical hosted-result backfill in CVM.
- Keep the snapshot immutable until the referenced Codex commit is deliberately refreshed; do not synthesize a schema at runtime.
- Run the same contract for compressed and file-backed replay bodies when a Codex rewrite is active; `keep_original` continues to preserve its original snapshot.

## References

- OpenAI Codex image-generation tool extension, commit [`61a4488`](https://github.com/openai/codex/blob/61a44880a85d2fd0d8770908dea5733495e571c8/codex-rs/ext/image-generation/src/tool.rs)
- sub2api Lite tool normalization, commit [`cb24522`](https://github.com/Wei-Shaw/sub2api/blob/cb24522dd53f8f363d008e3afdc8e4baf9788cab/backend/internal/service/openai_responses_lite_tools.go)
