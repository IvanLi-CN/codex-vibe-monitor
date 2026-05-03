# OpenAI 兼容 WebSocket 代理实现状态（#w5s2x）

## Coverage

- 已实现：`/v1/*` WebSocket upgrade 检测与 pool 鉴权复用；非 upgrade HTTP 请求继续走原 HTTP proxy。
- 已实现：`OPENAI_PROXY_WEBSOCKET_ENABLED` 全局启用开关，默认关闭；关闭时 WS upgrade 返回 `503` 且不连接上游。
- 已实现：账号池选择、downstream upgrade 前的上游 WS 握手 failover、上游 `ws/wss` URL 构造与透明 text/binary/ping/pong/close 帧中继。
- 已实现：API key 与 OAuth 账号的 upstream `Authorization` 覆盖、安全 header 转发、HTTP/HTTPS CONNECT 与 SOCKS5/SOCKS5H forward-proxy 隧道。
- 已实现：连接级 pool attempt 记录、reservation 释放与广播。

## Account Failover

- 上游 WS 连接或握手失败时，代理在同一个 downstream 请求内完成切号，不依赖 downstream 客户端先失败再重连。
- 每个失败账号都会记录 transport failure attempt、释放 routing reservation、记录 forward-proxy network failure，并通过现有 pool route failure 机制降低该 route 的后续优先级。
- 失败账号 id 与 upstream route key 会被排除，下一轮继续调用现有 pool resolver；候选耗尽或达到 distinct-account retry budget 后才向 downstream 返回 HTTP error。
- downstream WebSocket upgrade 只在某个上游账号握手成功后发生；已建立隧道中途断开后不做透明换号。

## Validation

- `cargo check`
- `cargo test websocket_ -- --nocapture`
- `cargo test -- --test-threads=1`
