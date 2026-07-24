---
title: OpenAI Responses Lite image-tool boundaries
module: proxy
problem_type: protocol_compatibility
component: responses_image_tools
tags: [openai, responses-lite, image-tools, proxy]
status: active
related_specs: [r4p9x, z9h7v, pbgwc]
---

# OpenAI Responses Lite Image-Tool Boundaries

## Context

Codex Responses Lite is not a Full Responses payload with a different model name. The client owns its image-generation extension and declares it through `input.additional_tools`.

## Symptoms

A proxy that injects top-level `tools: [{"type":"image_generation"}]` or `tool_choice` can make an otherwise valid Lite request fail with a 400 validation error about a top-level image-generation tool. Treating that response as account capability evidence incorrectly excludes healthy upstream accounts.

## Root Cause

The legacy rewrite policy operated on the Full Responses top-level tool contract without first identifying the Lite protocol. The resulting malformed request was then classified as an unsupported image-tool capability.

## Resolution

1. Detect Lite exclusively from `X-OpenAI-Internal-Codex-Responses-Lite: true`.
2. For Lite, leave all `input.additional_tools` content, top-level tools, and `tool_choice` untouched for every policy mode, including `force_remove`.
3. Continue Full Responses `keep_original | fill_missing | force_add | force_remove` behavior unchanged.
4. Persist `imageToolRewrite` audit data on invocation and workflow-attempt request summaries. Lite uses `responses_lite`, `skipped`, and `responses_lite_client_owned_tools`.
5. Do not learn `unsupported` from an error matching `responses lite`, `top-level tool type`, and `image_generation`; repair only already-observed rows with that exact signature and retain manual overrides.

## Guardrails

- Do not infer Lite from `gpt-5.6` or any model identifier.
- Do not recreate the Codex image-generation schema in CVM.
- Preserve the rule for compressed and file-backed replay bodies; preserving the snapshot is preferable to materializing it merely for a skipped rewrite.

## References

- OpenAI Codex Lite suite, commit [`5c94796`](https://github.com/openai/codex/blob/5c94796dc9e88580fdf0b05ef9ce9d975a86e1a6/codex-rs/core/tests/suite/responses_lite.rs)
- OpenAI Codex image-generation tool extension, commit [`5c94796`](https://github.com/openai/codex/blob/5c94796dc9e88580fdf0b05ef9ce9d975a86e1a6/codex-rs/ext/image-generation/src/tool.rs)
- sub2api Lite tool normalization, commit [`cb24522`](https://github.com/Wei-Shaw/sub2api/blob/cb24522dd53f8f363d008e3afdc8e4baf9788cab/backend/internal/service/openai_responses_lite_tools.go)
