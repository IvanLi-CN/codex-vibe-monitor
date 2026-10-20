# Responses `server_error` 过载特征与同账号重试

> 本文是该主题的长期需求合同；当前实现覆盖见 `./IMPLEMENTATION.md`，主题演进与兼容背景见 `./HISTORY.md`。

## Context and Scope

当前上游可能以 HTTP 200 返回 `response.failed`，并使用 `server_error` 搭配 `Our servers are currently overloaded. Please try again later.` 表达服务过载。现有 Responses early gate 只把精确的 `server_is_overloaded` 和明确的账号并发限制识别为可重试 overload，导致本主题的错误被直接记录为失败。

本主题只定义 pool Responses 请求对该新特征的窄范围识别和同账号重试策略；已有 `server_is_overloaded` 的账号/route 切换行为继续由既有合同拥有。

### In scope

- pool `POST /v1/responses` 与 `POST /v1/responses/compact` 的 pre-forward overload 识别。
- `server_error` 过载消息的稳定特征判定。
- 当前上游账号内的透明重试、预算、attempt 记录和终态语义。
- 普通 `server_error`、混合失败类型和既有 `server_is_overloaded` 行为的非回归边界。

### Out of scope

- generic proxy、非 Responses endpoint 或普通 HTTP 5xx 的新分类规则。
- 本次新增特征触发的其他账号、route 或 channel 切换。
- 已向下游发送业务事件或 body 后的透明回放。
- 新的配置项、HTTP API 字段、数据库字段或 UI 设置。
- 修改既有 `server_is_overloaded` 的账号/route 切换梯度。

## Terms and Interfaces

- **Responses family**：pool `POST /v1/responses` 与 `POST /v1/responses/compact`。
- **Same-account retry**：在当前已选上游账号上重新发起当前请求，不选择其他账号或 route/channel。
- **Upstream switch**：从当前上游账号或 route/channel 选择其他上游候选；本主题的新特征禁止该行为。
- **New overload signal**：HTTP 200 的 Responses 失败事件中，错误码为 `server_error` 且消息经过大小写、空白和标点归一化后包含 `our servers are currently overloaded`。
- **Legacy overload signal**：错误码为 `server_is_overloaded`，或错误码为 `rate_limit_exceeded` 且消息明确表达并发限制；该信号继续使用既有行为。

## Requirements

### Stable requirement identifiers

- REQ-RSO-001: 系统 MUST 仅将 Responses family 中 `code=server_error` 且消息包含规范化短语 `our servers are currently overloaded` 的 HTTP 200 失败事件识别为 New overload signal；普通 `server_error` 或仅包含模糊 `overloaded` 文本的消息不得匹配。 covers: [Responses error classifier](../../src/proxy/request_entry.rs)
- REQ-RSO-002: 系统 MUST 只在 pool Responses early gate 的安全窗口内应用 New overload signal；`/v1/responses` 允许在首个业务事件前检查 SSE，`/v1/responses/compact` 只允许检查首个尚未下发的 body chunk。 covers: [Responses gate](../../src/proxy/stream_gate.rs) and [pool failover entry](../../src/proxy/failover.rs)
- REQ-RSO-003: 命中 New overload signal 后，系统 MUST 只在当前上游账号内按既有 overload retry budget 和 backoff 透明重试，最多 4 次总尝试，且总请求超时预算继续累计；预算耗尽后 MUST 直接结束，不得切换其他账号、route 或 channel。 covers: [same-account budget](../../src/proxy/stream_gate.rs) and [retry loop](../../src/proxy/failover.rs)
- REQ-RSO-004: 系统 MUST 保持 Legacy overload signal 的既有行为，包括 `server_is_overloaded` 的同账号重试、同 route 账号切换和其他 route/channel 兜底；本主题不得把旧信号改为 same-account-only。 covers: [legacy overload classifier](../../src/proxy/request_entry.rs) and [overload ladder](../../archive/specs/bk2pt-responses-overload-early-route-retry/SPEC.md)
- REQ-RSO-005: 系统 MUST 保持失败类型边界：普通 `server_error`、timeout、普通 5xx 和其他临时失败继续使用各自既有规则，不得被反向改判为 New overload signal，不得因 New overload signal 重置其他失败家族的预算。 covers: [failure classifier](../../src/proxy/request_entry.rs) and [non-overload gate regression](../../src/tests/stateful_sqlite/routing_timeout_and_overload_failover.rs)
- REQ-RSO-006: 系统 MUST 只在下游尚未看到首个业务事件或 body 时透明重试；一旦业务内容已经可见，后续 overload MUST 保持 late-failure 语义，不得回放或重新执行请求。 covers: [early response gate](../../src/proxy/stream_gate.rs) and [late failure handling](../../src/proxy/failover.rs)
- REQ-RSO-007: New overload signal 的重试 attempt MUST 继续写入现有 attempt 观测，预算耗尽后的调用 MUST 保持 `upstream_response_failed` / `service_failure` 外部语义，并继续使用 retryable route failure、非立即账号 cooldown 的既有健康语义。 covers: [attempt finalization](../../src/proxy/failover.rs) and [failure constants](../../src/main.rs)
- REQ-RSO-008: 本主题 MUST NOT 新增 HTTP API、SQLite schema、配置开关或 UI 字段；修复只能扩展现有分类、early gate、failover attempt 记录和回归验证。 covers: [proxy classifier](../../src/proxy/request_entry.rs), [stream gate](../../src/proxy/stream_gate.rs), and [failover](../../src/proxy/failover.rs)

## Verification

### VER-RSO-001: Classifier boundaries

- Method: 使用窄范围分类测试验证 canonical `server_error` overload message 命中，普通 `server_error`、模糊 overload 文本和非 HTTP 200 不命中；现有 `server_is_overloaded` 与 concurrency `rate_limit_exceeded` 正向样例继续命中。
- covers: `REQ-RSO-001`, `REQ-RSO-004`, `REQ-RSO-005`
- Pass condition: 分类结果与错误码、消息和 HTTP 状态的组合完全一致，且不存在仅凭 `server_error` 或 `overloaded` 单词重试的路径。

### VER-RSO-002: Pre-forward gate

- Method: 用 metadata-prefixed `/v1/responses` SSE、compact 首 chunk、首个业务事件前失败和首个业务事件后失败 fixture 验证 gate outcome。
- covers: `REQ-RSO-002`, `REQ-RSO-006`
- Pass condition: 安全窗口内返回 `RetrySameAccount`，业务内容已发送后保持原流并记录 late failure。

### VER-RSO-003: Same-account retry integration

- Method: 用 pool upstream fixture 返回 New overload signal，随后返回成功或持续失败，并检查账号、route、attempt count、retry index、backoff 和终态记录。
- covers: `REQ-RSO-003`, `REQ-RSO-007`
- Pass condition: 最多 4 次尝试全部使用同一账号和 route，不发生上游切换；成功时下游只看到最终成功，耗尽时保留现有失败语义。

### VER-RSO-004: Mixed failure non-regression

- Method: 组合普通临时失败、普通 `server_error` 和 New overload signal，检查每类失败的原有分类和共享超时/attempt 预算。
- covers: `REQ-RSO-005`, `REQ-RSO-006`
- Pass condition: 前置失败不会被追溯改判为 overload，New overload 分支不会重置其他失败规则，且下游可见内容不会被回放。

### VER-RSO-005: Legacy overload compatibility

- Method: 复用现有 `server_is_overloaded` pool Responses integration fixture，验证旧信号的同账号重试、同 route 优先和其他 route/channel 兜底。
- covers: `REQ-RSO-004`
- Pass condition: 旧信号行为与既有 overload ladder 保持一致，新规则只影响 `server_error` canonical overload message。

### VER-RSO-006: Contract and rollout inspection

- Method: 检查 attempt detail 与 invocation detail 的线上记录，确认匹配样本的 attempt count 大于 1 或按预算耗尽记录多次 attempt，且未出现账号/route 切换。
- covers: `REQ-RSO-003`, `REQ-RSO-007`, `REQ-RSO-008`
- Pass condition: 线上记录能区分初次 HTTP 200 logical failure、同账号 retry 和最终终态，不需要新增外部字段。

## Related ADRs

None

## Visual Evidence

- None

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
- [`bk2pt` Responses overload early route retry](../../archive/specs/bk2pt-responses-overload-early-route-retry/SPEC.md)
- [`p6x4r` Responses per-account retry budget](../../archive/specs/p6x4r-pool-responses-per-account-retry-budget/SPEC.md)
- [`br38t` Responses overload preview-cap follow-up](../../archive/specs/br38t-responses-overload-preview-cap-followup/SPEC.md)
