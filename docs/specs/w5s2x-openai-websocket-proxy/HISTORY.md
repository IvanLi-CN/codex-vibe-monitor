# OpenAI 兼容 WebSocket 代理演进记录（#w5s2x）

## 2026-07-05

- 将历史透明隧道式 WS 契约提升为 Responses WS 协议感知 passthrough relay。
- 修正路由边界：所有 downstream WS session 在读取首个 `response.create` payload 后才执行 prompt-cache routing、owner guard 和上游 WS 握手。
- 修正 turn 边界：relay 以 downstream `response.create` 和 upstream terminal event 维护 active turn，支持同一连接中的多 turn usage 记录。
- 修正断连语义：downstream active turn 断开后 bounded drain upstream terminal/usage；upstream terminal 前 close/error 统一转成 `1013 upstream_unavailable; retry`。
- 补齐握手 failover 测试：首个候选 handshake 失败时保留首帧并在同一 downstream session 内切到下一候选。
- 修正 subprotocol 语义：downstream 请求 subprotocol 时，上游必须选择同一值后才发送保留首帧；不匹配候选按 retryable failure 处理。

## 2026-05-04

- 建立历史 WebSocket 代理 topic spec：扩展 OpenAI 兼容 `/v1/*` pool proxy，不替换 Dashboard `/events` SSE。
- 引入 downstream WS 开关、upstream WS 默认开关和 `unsupported_transport:websocket` / `不支持 WS` 系统 tag。
- 实现基础 WebSocket upgrade、上游 URL 映射、透明帧中继、连接级 pool attempt 和 terminal usage 保守计费。
