# OpenAI 兼容 WebSocket 代理（#w5s2x）

## 状态

- Status: active
- Created: 2026-05-04
- Last: 2026-07-05

## 背景 / 问题陈述

本项目的 `/v1/*` 代理需要支持 OpenAI Responses WebSocket traffic。历史实现已具备下游 upgrade、账号池上游连接、透明帧中继和 terminal usage 解析，但仍偏向 HTTP/隧道模型：部分请求会在读取首个 Responses WS turn 之前就完成账号选择和上游握手，导致 payload-only `prompt_cache_key`、encrypted owner guard、handshake failover 与 per-turn usage 的边界不稳定。

当前契约将 WebSocket 代理定义为 Responses WS 协议感知 passthrough relay：代理不做 HTTP/SSE bridge，不共享 upstream connection pool，但必须理解首帧 `response.create` 和上游 terminal event，以便正确路由、观测和失败收口。

## Goals

- `/v1/*` downstream WebSocket 在通过 HTTP 层鉴权、开关和并发门禁后，先读取首个 text JSON `response.create` payload，再执行账号池路由和上游 WS 握手。
- 首帧 payload 的 `model`、`prompt_cache_key`、`previous_response_id` 和 encrypted content 参与路由、owner guard 和观测；header `prompt_cache_key` 只作为 fallback，sticky-only header 不得被当作 prompt cache key。
- 上游握手成功后才发送被保留的首帧；握手、429、unsupported 或网络失败仍在同一个 downstream WS session 内按账号池 failover 尝试后续候选。
- relay 以 `response.create` 到 terminal event 为 active turn 边界，继续透传 text、binary、ping、pong、close。
- 上游 text terminal event 只在包含完整 usage 时生成现有 `codex_invocations` / cost / pool attempt 记录；缺字段或类型错误时跳过，不产生半字段记录。
- downstream 已断开且 active turn 尚未 terminal 时，代理在短窗口内 drain 上游 terminal/usage，持久化 usage，并停止写 downstream。
- 上游在 active turn terminal 前 close/error 时，代理向 downstream 发送 close `1013`，reason 包含 `upstream_unavailable; retry`，并记录 pool route failure。
- 继续支持 `unsupported_transport:websocket` / `不支持 WS` 系统 tag、WS 全局开关、账号级 upstream base URL `http/https` 到 `ws/wss` 映射、安全 header 转发和连接级 pool attempt。

## Non-goals

- 不实现 OpenAI Responses WebSocket 与 HTTP/SSE 之间的事件级 bridge。
- 不实现跨 downstream 客户端共享 upstream WebSocket connection pool。
- 不实现 sub2api 风格的 `ctx_pool` / shared / dedicated 多模式。
- 不改 Dashboard SSE、普通 HTTP proxy 路径、SQLite schema 或账号管理 UI。
- 不对任意 WebSocket 帧做深度计费，只解析已知 Responses terminal usage event。

## 接口契约

- Downstream: `GET /v1/*` 携带标准 WebSocket upgrade headers，且必须携带现有 pool route key。
- Downstream gate: `websocketEnabled=false` 时返回 HTTP `503` JSON error，不进入 WS upgrade。
- Upstream gate: `upstreamWebsocketDefaultEnabled=false` 时，已 upgrade 的 downstream WS 收到 retryable close，不建立不可靠上游隧道。
- Initial frame: upgrade 成功后，第一个 downstream frame 必须是 text JSON，`type` 必须为 `response.create`。非 text、非 JSON 或非 `response.create` 首帧以 close `1011` 结束。
- Routing keys: payload `prompt_cache_key` / `promptCacheKey` 优先；无 payload key 时才使用 header `x-prompt-cache-key` / `prompt-cache-key` / `x-openai-prompt-cache-key`；`x-sticky-key` 只影响 sticky routing，不参与 prompt-cache owner guard。
- Model: payload `model` 优先于 query `model` 参与账号池模型约束选择。
- Owner guard: 若首帧或后续 `response.create` payload 带 encrypted content 且 prompt-cache owner 已锁定，候选账号必须匹配 owner；不匹配时返回 retryable close `encrypted_session_owner_unavailable; retry`。
- Failover: 上游 WS 握手失败时，代理记录当前候选失败、释放 reservation、排除失败 account/route key，并在保留首帧的同一个 downstream session 中尝试下一个候选。候选耗尽时 downstream 收到 retryable close。
- Relay: relay 继续透传 text/binary/ping/pong/close；上游 terminal text 在写 downstream 前先完成 terminal/usage 观察，避免 downstream 断开导致 usage 丢失。
- Drain: downstream close/error/EOF 发生在 active turn 中时，代理停止读取 downstream，短窗口读取 upstream；若期间收到 terminal usage，则按成功 turn 持久化；若 upstream 在 terminal 前 close/error 或 drain 超时，则按 transport failure 收口。
- Failure close: active turn terminal 前的 upstream close/error 必须转成 downstream close `1013 upstream_unavailable; retry`。

## 验收标准

- 无 header `prompt_cache_key` 时，首帧 payload 的 `prompt_cache_key` 能决定 owner/binding 路由。
- sticky-only header 不会被当作 prompt cache key，也不会触发 encrypted owner guard。
- 首个候选上游 handshake 失败时，同一 downstream WS session 内切到下一候选，并把保留首帧发送给成功候选。
- 多个 `response.create` turn 能逐 turn relay，terminal usage 分别落入 invocation/cost 统计。
- 缺字段 usage 不产生半字段 usage/cost 记录。
- downstream 已断开但 upstream 随后发出 terminal usage 时，系统完成 bounded drain、持久化 usage，并不再尝试写 downstream。
- upstream terminal 前 close/error 时，downstream 收到 `1013` 且 reason 包含 `retry`，pool route failure 与 attempt status 可查询。

## 参考输入

- Historical source: `docs/archive/specs/w5s2x-openai-websocket-proxy/`
- CPA reference: `router-for-me/CLIProxyAPI@5afc0f1d5e9ed8d47809a1bd1f54834bc7e75375`
- sub2api reference: `Wei-Shaw/sub2api@b650bdd68d25bad3e502b2e34efe775555da2eba`
