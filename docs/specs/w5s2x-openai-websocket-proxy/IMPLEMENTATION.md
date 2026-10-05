# OpenAI 兼容 WebSocket 代理实现状态（#w5s2x）

## 生命周期

- Status: retired
- 当前退役实现边界以 [ADR 0020](../../adr/0020-retire-downstream-websocket-proxy.md) 为准。
- 下列 coverage/test 条目记录退役前的历史实现，不代表当前继续提供 WebSocket 能力。

## Pre-retirement coverage

- 已实现：`/v1/*` WebSocket upgrade 检测、pool 鉴权、downstream/upstream 双开关和 proxy request concurrency gate。
- 已实现：`/v1/responses` downstream WS session 在 upgrade 后读取首个 text JSON `response.create`，再执行 prompt-cache routing、encrypted owner guard、账号池选择和上游 WS 握手。
- 已实现：非 Responses 的 `/v1/*` WebSocket（例如 `/v1/realtime`）在 upgrade 后立即执行账号池选择和上游 WS 握手，不等待 downstream `response.create`，支持上游先发事件 passthrough。
- 已实现：`/v1/responses` 首帧读取超时、读取错误和首帧协议拒绝会写入 `pool_upstream_request_attempts` transport failure，并广播 attempt snapshot；客户端未开始 turn 的正常主动关闭不记失败。
- 已实现：payload `prompt_cache_key` 优先于 header prompt cache key；sticky-only header 不会进入 prompt-cache owner guard。
- 已实现：首帧 payload `model` 优先于 query `model` 进入账号池选择；`previous_response_id` 作为 turn metadata 解析与日志观测输入保留。
- 已实现：上游握手成功后发送保留首帧；握手失败、timeout、unsupported HTTP 状态和 transport error 仍复用账号池 failover，在同一个 downstream session 内尝试下一个候选。
- 已实现：downstream 请求 subprotocol 时，代理为客户端兼容性选择第一个请求值，并在上游握手后要求上游返回同一 subprotocol；不匹配候选在发送首帧前被标记为 retryable transport failure。
- 已实现：Responses WS turn-aware relay。downstream `response.create` 打开 active turn，上游 `response.completed` / `response.done` / `response.failed` terminal event 关闭 active turn。
- 已实现：terminal usage 观察先于 downstream 写入；完整 `input_tokens` + `output_tokens` 才进入现有 invocation/cost 持久化路径，缺字段 usage 被跳过。
- 已实现：downstream active turn 断开后进行 bounded upstream drain；drain 收到 terminal usage 时持久化 usage 并按成功 turn 收口。
- 已实现：upstream 在 active turn terminal 前 close/error 时向 downstream 发送 close `1013 upstream_unavailable; retry`，并记录 pool route transport failure 与 attempt failure。
- 已实现：`/v1/responses` 第三方兼容 API-key upstream 若已握手成功但在 terminal 前 clean close/EOF，系统在保留 retryable downstream close 和 route failure 记录的同时，自动给该账号打 `unsupported_transport:websocket` / `不支持 WS` tag，后续 WS 路由跳过该候选。
- 已实现：`unsupported_transport:websocket` / `不支持 WS` 系统 tag、WS unsupported auto-tagging、API key/OAuth header 覆盖、安全 header 转发、`http/https` 到 `ws/wss` URL 映射和 forward proxy 隧道。

## Retirement implementation contract

- `/v1/*` WebSocket upgrade 在进入鉴权、账号路由和上游连接前返回 HTTP `501`，错误码为 `websocket_proxy_removed`。
- 拒绝不创建 invocation、upstream attempt 或上游连接；结构化遥测只保留脱敏请求边界信息。
- 删除 WebSocket runtime、settings、tag ensure、usage refresh、依赖和测试路径；保留历史 `transport="websocket"` 读路径，并以 `WebSocket（历史）` 展示。
- 迁移 `retire_openai_websocket_proxy_v1` 必须按专题 Spec 与持久化迁移记录执行。

## Current implementation

- `src/proxy/request_entry.rs` detects HTTP/1 `Upgrade: websocket` and HTTP/2 Extended CONNECT `:protocol = websocket` before invoking the shared HTTP proxy entrypoint, returning the fixed `501` JSON envelope for either form.
- The Axum WebSocket feature, relay/dialer module, direct tungstenite dependencies, WebSocket settings initialization, capability tag ensure/learning, and WebSocket-only usage refresh/persistence paths are removed.
- The SQLite migration retains the three legacy settings columns, clears the exact retired system tag and associations in one `BEGIN IMMEDIATE` transaction even when its completion marker already exists, preserves non-scalar JSON values while removing retired integer tag IDs, records the named migration marker, and logs only affected-row counts.
- Legacy tag cleanup now removes only non-system integer session references, so existing system-tag JSON survives long enough for the retirement migration to remove only the exact retired WebSocket tag.
- Settings request deserialization tolerates old WebSocket fields as unknown input while response serialization omits them. Historical `transport="websocket"` rows remain readable and are labeled `WebSocket（历史）` in the UI.
- The demo and account-pool fixtures no longer create live WebSocket records or capability tags; historical records remain in Records, invocation, and dashboard read fixtures.
- After rebasing onto the current `origin/main` baseline, the remaining orphaned WebSocket message-conversion test and the unused pre-upstream WebSocket persistence helper were removed; the current baseline-only clippy fixes remain unrelated to the retired transport contract.

## Validation

- Targeted Rust regression: the early `501` rejection, Settings compatibility, migration idempotency/rollback plus forward re-entry, and legacy system-tag cleanup tests pass.
- `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and `cargo clippy --locked --all-targets --all-features -- -D warnings` pass.
- The repository Rust source-quality policy check passes after synchronizing the explicit inventory, suppression counts, and budgets for the retired surface.
- Web unit tests pass: `1779 passed / 6 skipped`; typecheck, lint, and production build pass. The Storybook suite passes with `200/200` tests and `48` intentional skips.
- Backend resource profiles: lightweight `1239/1239` and stateful-sqlite `1397/1397` pass. The required Archive/File I/O CI check also passes; a prior local timing variance in baseline system-storage concurrent-deletion coverage is not used as release evidence.
- CI PR run `37269645818` passed for runtime candidate `06b2876fdb8dd3fb5a7417e8aa6229686a6dc159` on base `324f9988cdc8794754f982ca97e8f474cb344778`, with final metadata head `bc6464cc5741ba50c122aaea501784a58e456039`; it includes Rust source quality/Clippy, all backend profiles, Web, Storybook, E2E, docs, tooling, policy, smoke, and build artifacts.
- `bun run lint:docs` and `git diff --check` pass. UI evidence covers the Settings page without WebSocket controls, the Records historical transport filter, and the historical transport chip. The chip visibly renders `WebSocket（历史）` rather than the retired `WS` abbreviation.

## Retirement regression coverage

- `websocket_upgrade_is_rejected_before_auth_routing_and_persistence` verifies ordinary, comma-separated, and repeated Upgrade headers plus HTTP/2 Extended CONNECT `:protocol = websocket` return the exact raw `501` JSON body without a CVM header or Invocation/Attempt rows. Because the rejection returns before the shared proxy handler, no upstream connection or retry path can run; the structured rejection log contains only method and URI path.
- `proxy_model_settings_api_preserves_upstream_429_max_retries_when_field_missing` verifies legacy WebSocket request fields are ignored and omitted from responses.
- `retire_websocket_proxy_migration_is_idempotent_and_preserves_unrelated_state` verifies legacy columns, exact integer tag cleanup, unrelated-tag preservation, OAuth session JSON cleanup, malformed/non-integer value preservation, and the completion marker.
- The retirement migration and non-system tag cleanup regressions also preserve JSON object/array/boolean values instead of re-encoding them as strings or integers.
- `retire_websocket_proxy_migration_rolls_back_and_reenters_after_failure` verifies a mid-transaction failure leaves settings, tag associations, tag rows, and the marker unchanged, then a newer attempt repairs forward and commits the retirement.
- Direct multi-major skips remain unsupported by the declared source compatibility range; an explicit intermediate upgrade is required and this release does not claim a direct-skip migration path.
- `cleanup_non_system_tags_removes_custom_tags_links_and_session_references` verifies custom tag references are removed while system-tag session references survive for exact retirement cleanup.
- The demo model regression verifies retired WebSocket Settings request fields are ignored and omitted from the response/state.
- `normal_http_terminal_persistence_does_not_emit_retired_websocket_state` verifies a normal HTTP terminal record does not recreate WebSocket transport, stream-terminal state, or the exact retired tag and association.
- `counted_http_transport_reports_network_bytes_through_dashboard_projection` verifies streamed HTTP upload/download bytes reach global and account network buckets and schedule the network projection.
- Request-entry and hourly trace tests verify query parameters are excluded from request logs, while historical workflow detail tests verify structured transport fields use `WebSocket（历史）`.
- Existing historical invocation query/filter and serialization fixtures continue to use `transport="websocket"` as read-only compatibility coverage.
