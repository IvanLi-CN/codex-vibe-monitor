# 代理流式稳定性修复（Codex 断流问题）

## Goal

修复 `/v1/*` 反向代理在流式转发阶段的误导性日志与断流处理，降低 Codex 侧出现 `stream disconnected before completion` / `error decoding response body` 的概率，并提升问题可观测性。

## In / Out

### In

- 调整代理日志语义：区分“响应头就绪”与“流式完成”。
- 在首包前上游流失败时直接返回 `502`，避免先返回 `200` 后中断。
- 优化代理 HTTP 传输配置（启用 HTTP/2 keepalive）。
- 增加对应回归测试（首包前失败、首包后失败）。
- 使用 Codex 客户端走代理链路做稳定性联调。

### Out

- 不改动前端页面逻辑。
- 不重构整体代理架构与存储模型。
- 不引入额外中间件或新外部依赖服务。

## Acceptance Criteria

1. Given 上游在首个响应块前失败，When 代理处理该请求，Then 返回 `502` 且错误语义明确。
2. Given 上游在首包后中断，When 代理转发流，Then 下游可感知流错误且日志可定位到同次代理请求。
3. Given 正常流式响应，When 请求完成，Then 仅记录“headers ready”与“stream completed”两类语义明确日志，不再用“response finished”误导完整性。
4. Given Codex 通过代理调用 `/v1/responses`，When 连续多次执行，Then 无 `stream disconnected before completion` 与 `error decoding response body` 签名。

## Testing

- 自动化测试：`cargo fmt`、`cargo test`（含新增回归用例）。
- 最终联调：`codex exec` 通过 `http://127.0.0.1:8080/v1` 连续执行稳定性验证并统计结果。

## Risks

- 上游服务限流/鉴权策略可能干扰稳定性判断。
- 日志增强会增加少量日志量；需关注生产日志采集成本。

## Milestones

- [ ] M1 日志语义修正与首包失败处理
- [ ] M2 HTTP 传输配置优化（HTTP/2 keepalive）
- [ ] M3 回归测试补齐并通过
- [ ] M4 Codex 端到端联调完成并留存结果
