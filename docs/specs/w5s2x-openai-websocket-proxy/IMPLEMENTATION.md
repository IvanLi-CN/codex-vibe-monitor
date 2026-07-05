# OpenAI 兼容 WebSocket 代理实现状态（#w5s2x）

## Coverage

- 已实现：`/v1/*` WebSocket upgrade 检测、pool 鉴权、downstream/upstream 双开关和 proxy request concurrency gate。
- 已实现：所有 downstream WS session 统一在 upgrade 后读取首个 text JSON `response.create`，再执行 prompt-cache routing、encrypted owner guard、账号池选择和上游 WS 握手。
- 已实现：payload `prompt_cache_key` 优先于 header prompt cache key；sticky-only header 不会进入 prompt-cache owner guard。
- 已实现：首帧 payload `model` 优先于 query `model` 进入账号池选择；`previous_response_id` 作为 turn metadata 解析与日志观测输入保留。
- 已实现：上游握手成功后发送保留首帧；握手失败、timeout、unsupported HTTP 状态和 transport error 仍复用账号池 failover，在同一个 downstream session 内尝试下一个候选。
- 已实现：Responses WS turn-aware relay。downstream `response.create` 打开 active turn，上游 `response.completed` / `response.done` / `response.failed` terminal event 关闭 active turn。
- 已实现：terminal usage 观察先于 downstream 写入；完整 `input_tokens` + `output_tokens` 才进入现有 invocation/cost 持久化路径，缺字段 usage 被跳过。
- 已实现：downstream active turn 断开后进行 bounded upstream drain；drain 收到 terminal usage 时持久化 usage 并按成功 turn 收口。
- 已实现：upstream 在 active turn terminal 前 close/error 时向 downstream 发送 close `1013 upstream_unavailable; retry`，并记录 pool route transport failure 与 attempt failure。
- 已实现：`unsupported_transport:websocket` / `不支持 WS` 系统 tag、WS unsupported auto-tagging、API key/OAuth header 覆盖、安全 header 转发、`http/https` 到 `ws/wss` URL 映射和 forward proxy 隧道。

## Validation

- `cargo fmt --check`
- `cargo check`
- `cargo test websocket_ -- --nocapture`

## Tests Added Or Updated

- 首帧 `response.create` 校验与 `model` / `prompt_cache_key` / `previous_response_id` 解析。
- payload-only prompt cache key owner routing 使用 `response.create` 首帧验证。
- 多 `response.create` turn 分别 relay 并分别持久化 terminal usage。
- downstream active turn 断开后 drain upstream terminal usage 并落库。
- upstream terminal 前 close 转换为 downstream `1013` retryable close，并记录 attempt failure。
- 上游 handshake failure 在同一 downstream session 内 failover 到下一候选，并发送保留首帧。
