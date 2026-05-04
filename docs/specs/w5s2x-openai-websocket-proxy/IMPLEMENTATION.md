# OpenAI 兼容 WebSocket 代理实现状态（#w5s2x）

## Coverage

- 已实现：`/v1/*` WebSocket upgrade 检测与 pool 鉴权复用；非 upgrade HTTP 请求继续走原 HTTP proxy。
- 已实现：`websocketEnabled` downstream 全局设置，默认关闭；关闭时 WS upgrade 返回 `503` 且不连接上游。`OPENAI_PROXY_WEBSOCKET_ENABLED` 只作为首次初始化默认值。
- 已实现：`upstreamWebsocketDefaultEnabled` upstream 默认 WS 全局设置，默认关闭；关闭时 downstream WS 不会连接上游 WS。`OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED` 只作为首次初始化默认值。
- 已实现：受保护系统 tag `unsupported_transport:websocket` / `不支持 WS`；带 tag 账号退出 WS 上游候选，HTTP 路由不受影响。
- 已实现：上游 WS 握手返回明确不支持 WS 的 HTTP 状态时，自动给该账号补写 `unsupported_transport:websocket` / `不支持 WS`，后续 WS 调度跳过该账号；网络 reset、timeout、502 等不确定故障不自动标记。
- 已实现：账号池选择、downstream upgrade 前的上游 WS 握手 failover、上游 `ws/wss` URL 构造与透明 text/binary/ping/pong/close 帧中继。
- 已实现：上游在 `response.completed` 前主动 close 时，不再透传原始 close 给下游，而是改发 `1013 upstream_unavailable; retry` 并记录路由失败。
- 已实现：API key 与 OAuth 账号的 upstream `Authorization` 覆盖、安全 header 转发、HTTP/HTTPS CONNECT 与 SOCKS5/SOCKS5H forward-proxy 隧道。
- 已实现：连接级 pool attempt 记录、reservation 释放与广播。
- 已实现：Responses WS terminal usage 事件的保守计费解析，完整 `input_tokens` + `output_tokens` 才生成 invocation/cost 记录。
- 已实现：upstream WS text 帧先转发给 downstream，再执行 terminal usage 持久化，避免计费写入阻塞 `response.completed` 交付。
- 已实现：设置页可保存 downstream WS 与 upstream WS 默认开关，后续 WS 请求运行时读取持久化全局设置；账号池 Storybook 展示 `不支持 WS` 系统标签。

## Account Failover

- 上游 WS 连接或握手失败时，代理在 downstream upgrade 前在同一个 HTTP upgrade 请求内完成切号，不依赖 downstream 客户端先失败再重连。
- 每个失败账号都会记录 transport failure attempt、释放 routing reservation、记录 forward-proxy network failure，并通过现有 pool route failure 机制降低该 route 的后续优先级。
- 失败账号 id 与 upstream route key 会被排除，下一轮继续调用现有 pool resolver；候选耗尽或达到 distinct-account retry budget 后才向 downstream 返回 HTTP error。
- downstream WebSocket upgrade 只在某个上游账号握手成功后发生。
- 已建立隧道中途断开后不做透明帧级换号；代理发送 downstream close `1013` / `upstream_unavailable; retry`，让支持重连的客户端进入下一次连接，并由服务端重新走号池选择。

## Validation

- `cargo check`
- `cargo test websocket_ -- --nocapture`
- `cargo test app_config_from_sources_reads_websocket_enabled_env -- --nocapture`
- `cargo test proxy_websocket_settings_initialize_from_env_once_then_persist -- --nocapture`
- `cd web && bun run test -- useSettings lib/api`
- `cd web && bun run build`
- Storybook visual evidence:
  - `docs/specs/w5s2x-openai-websocket-proxy/assets/upstream-account-ws-unsupported-badge.png`
  - `docs/specs/w5s2x-openai-websocket-proxy/assets/settings-websocket-global-switches.png`
