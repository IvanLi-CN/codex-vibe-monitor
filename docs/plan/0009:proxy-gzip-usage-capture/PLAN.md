# 代理 gzip 流式 usage 采集修复

## Goal

修复 `/v1/responses` 等流式代理响应在 `Content-Encoding: gzip` 场景下的 usage 解析缺失，确保 `inputTokens` / `outputTokens` / `cacheInputTokens` / `totalTokens` 可稳定入库并在统计与仪表盘展示。

## In / Out

### In

- 解析链路在落库前按上游 `Content-Encoding` 进行解码（优先支持 `gzip` / `x-gzip`）。
- 在代理请求头透传策略中屏蔽 `accept-encoding`，降低未来压缩流解析风险。
- 保持下游响应透传行为不变（不改变对客户端的响应字节流）。
- 新增单元与集成回归测试，覆盖 gzip 成功解析、解压失败可观测、header 过滤。

### Out

- 不改动 HTTP API 路径与响应字段。
- 不改动数据库 schema。
- 不扩展 `br` / `deflate`（待后续出现真实需求时再扩展）。

## Acceptance Criteria

1. Given 上游返回 `Content-Encoding: gzip` 的 SSE，When 代理解析并落库，Then 成功记录含非空 `inputTokens`、`outputTokens`、`cacheInputTokens`、`totalTokens`。
2. Given 解析阶段遇到压缩解码失败，When 请求仍完成转发，Then 请求链路不被中断且 payload 中有可观测的 decode failure reason。
3. Given 未压缩响应，When 执行解析，Then 与现有行为一致且无回归。
4. Given 代理转发请求头，When 上游收到请求，Then 不包含 `Accept-Encoding` 透传。

## Testing

- `cargo fmt -- --check`
- `cargo test`
- 共享测试环境端到端验证：模拟上游 gzip SSE，校验 `/api/stats` 与 `/api/invocations` 的 token 字段恢复。

## Risks

- 当前仅处理 gzip，若上游后续改为 `br` / `deflate` 仍需补充解码支持。
- 解码失败时回退原始字节解析，usage 可能为空，但不会影响代理转发可用性。

## Milestones

- [x] M1 响应解析链路增加按 `Content-Encoding` 解码能力
- [x] M2 请求头透传策略屏蔽 `accept-encoding`
- [x] M3 单元/集成测试补齐并通过
- [x] M4 共享测试环境端到端验证通过

## Execution Notes

- PR: #48
- Automated: `cargo fmt -- --check`, `cargo test`（62 passed, 0 failed）
- Shared testbox: 在 `codex-testbox` 复现 gzip SSE 上游，验证 `/api/stats` 的 `totalTokens` 增长，且 `/api/invocations` 成功记录含非空 `inputTokens`/`outputTokens`/`cacheInputTokens`/`totalTokens`。
