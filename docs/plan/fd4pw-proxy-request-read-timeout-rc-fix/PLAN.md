# 代理请求读体超时与失败分型修复（RC 止血）

## Goal

降低 Codex App 在代理链路中的 `stream disconnected before completion` / `error decoding response body` 触发概率，并将代理失败稳定分型为“请求读体中断/超时”与“上游连接失败”两类可观测事件。

## In / Out

### In

- 新增 `OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS`（默认 `180`）并接入 `/v1/responses`、`/v1/chat/completions` 捕获路径请求体读取。
- 请求读体超时改为返回 `408 Request Timeout`；请求体流中断保持 `400`，但记录明确 failure kind。
- 在捕获路径为“读体失败/超时/上游握手或连接失败”持久化 `source=proxy` 记录（不只日志）。
- 补充流式转发终态日志（`stream_completed` / `stream_error` / `downstream_closed`）并写入统一 failure kind。
- 补齐回归测试并通过最小验证。

### Out

- 不改动现有 API 路由与响应 JSON 基本结构。
- 不改动前端逻辑与页面展示。
- 不进行数据库 schema 变更。

## Acceptance Criteria

1. Given 捕获路径请求体读取超过超时时间，When 代理处理请求，Then 返回 `408` 且记录 `request_body_read_timeout`。
2. Given 客户端在上传请求体过程中断开，When 代理处理请求，Then 返回 `400` 且记录 `request_body_stream_error_client_closed`。
3. Given 上游连接失败或握手超时，When 代理处理请求，Then 分别记录 `failed_contact_upstream` 或 `upstream_handshake_timeout`，并可在持久化记录中检索。
4. Given 上游流式响应中断或下游提前关闭，When 捕获路径转发，Then 记录明确终态（`stream_error` / `downstream_closed` / `stream_completed`）。

## Testing

- `cargo fmt --check`
- `cargo test`
- 如遇流式时序不稳定，补跑：`cargo test proxy_openai_v1 -- --nocapture`

## Risks

- 更短的读体超时可能将极慢但合法的大请求误判为超时（需结合日志观察并按需调参）。
- 新增失败分类后，历史聚合口径可能与旧数据存在短期对比偏差。

## Milestones

- [ ] M1 配置与读体超时/分型落地
- [ ] M2 捕获路径失败持久化与流终态日志对齐
- [ ] M3 回归测试补齐并通过
- [ ] M4 RC 发布并替换测试线验证
