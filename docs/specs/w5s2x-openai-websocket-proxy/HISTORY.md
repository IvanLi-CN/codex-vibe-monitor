# OpenAI 兼容 WebSocket 代理演进记录（#w5s2x）

## 2026-07-07

- 101 线上只读诊断确认：CIII、TeeTime 等第三方兼容 API-key upstream 能完成 `/v1/responses` WS 握手，但会在 `response.completed` 前关闭连接，客户端表现为 `websocket closed by server before response.completed`。
- 修正 post-upgrade capability 收口：这类 terminal 前 clean close/EOF 现在被识别为账号 WebSocket 能力缺失，自动打 `unsupported_transport:websocket` / `不支持 WS` 系统 tag，让后续客户端 retry 跳过坏候选；普通网络错误、OAuth/官方账号与 downstream 主动断开后的 drain 失败不参与自动 no-WS 标记。

## 2026-07-06

- 修正协议分流：`/v1/responses` 保持首帧驱动的 turn-aware relay；`/v1/realtime` 等非 Responses WS 改为即时上游 passthrough，避免 Realtime 连接因等待 downstream `response.create` 而超时。
- 补齐首帧前失败观测：`/v1/responses` 在上游建连前因首帧超时、读取错误或协议拒绝失败时写入 pool attempt failure，避免线上只看到请求日志而看不到失败 attempt。
- 线上只读诊断确认：101 当前部署的 WebSocket 开关已启用，历史失败集中在第三方 `api_key_codex` 兼容上游的 `/v1/responses`，表现为 `426 Upgrade Required` 或握手后在 terminal 前 close；这类上游能力问题不应掩盖代理对 `/v1/realtime` 的 server-first passthrough 分流缺陷。

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
