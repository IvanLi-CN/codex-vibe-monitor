# Codex Vibe Monitor Context

This context freezes the invocation-observability terms that the product uses in diagnostics, badges, and companion docs. It exists to keep transport paths, request semantics, and response outcomes from drifting into overloaded labels.

## Invocation Compaction

**Compact**:
The legacy dedicated compaction request path backed by `/v1/responses/compact`.
_Avoid_: 远程压缩V1, remote compaction v1, `/v1/responses` compaction

**远程压缩V2**:
Server-side compaction semantics that run inside `/v1/responses` without changing the transport endpoint.
_Avoid_: Compact, `/v1/responses/compact`, normal Responses badge

**压缩请求**:
The request-side declaration that a `/v1/responses` call enabled remote compaction V2 semantics.
_Avoid_: 压缩响应, 最终已压缩

**压缩响应**:
The response-side proof that the upstream actually emitted a compaction item for the invocation.
_Avoid_: 压缩请求, 已启用压缩
