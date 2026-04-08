# Responses-family `server_is_overloaded` 早期重试与分层换路由收口（#bk2pt）

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-08
- Last: 2026-04-08

## 背景 / 问题陈述

- `responses` family 里存在一类特殊上游失败：HTTP 仍为 `200`，但在响应体里提前给出 `upstream_response_failed + server_is_overloaded`。
- 现有逻辑只会在 `/v1/responses` 的**首个 SSE event** 直接命中 overload 时重试；如果先收到 `response.created` / `response.in_progress`，再收到 overload，透明重试窗口会过早关闭。
- 现有 failover 更偏向“同账号 / 换账号”，缺少 overload 专用的“原账号重试 → 同 route 其他账号 → 其他 route/channel”收口顺序。
- `/v1/responses/compact` 也会遇到同类偶发 overload，但当前没有对应的 pre-forward 处理。

## 目标 / 非目标

### Goals

- 仅针对 `upstream_response_failed + server_is_overloaded` 增加专门的早期透明重试流程，不改变其他 5xx / transport / timeout / 429 语义。
- `/v1/responses` 把 early gate 从“首个 SSE event”扩展为“metadata-only 起始窗口”，允许在 `response.created` / `response.in_progress` 之后、首个非 metadata 事件之前继续透明重试。
- overload 专用顺序固定为：**原账号重试 3 次（总计 4 次尝试）→ 同 route 其他账号 → 其他 route/channel**。
- `/v1/responses/compact` 只在当前 pre-forward 安全阶段接入同一 overload ladder，不引入 full-body buffering 或 body 已下发后的回放。
- 一旦任意有效业务事件 / 响应体已对下游可见，后续 overload 保持现有 late-failure 语义：不透明重放，只记录 route state 与失败详情。

### Non-goals

- 不修改非 overload 的 `response.failed`、普通 HTTP 5xx、429、transport、timeout、compact unsupported 等既有失败策略。
- 不新增 env 配置、数据库迁移、外部 HTTP API 参数或管理面开关。
- 不重写 pool resolver 的通用排序逻辑；route 约束仅作为 overload failover 的局部策略。
- 不为 `responses/compact` 做整包缓冲或通用“先读完整 body 再决定是否重放”。

## 功能与行为规格（Functional / Behavior Spec）

- `/v1/responses` 的起始 gate 必须持续缓冲 metadata-only SSE 事件；`response.created`、`response.in_progress` 与空 keepalive 不应提前结束透明重试窗口。
- 若在 metadata-only 窗口内检测到 `response.failed(server_is_overloaded)`，服务端必须先重试**原账号 3 次**，退避固定为 `500ms -> 1s -> 2s`。
- 原账号 overload 重试预算耗尽后，后续 fresh account 解析必须先限定在相同 `upstream_route_key`；仅当相同 route 无可用候选时，才把当前 route 视为已穷尽并切换到其他 route/channel。
- `/v1/responses/compact` 只允许基于**首个已拉取但尚未下发的 body chunk** 检查 retryable overload；若该安全窗口内命中 overload，则走同一 overload ladder。
- 若 `/v1/responses` 已经向下游放出首个非 metadata 事件，或 `/v1/responses/compact` 已经开始下发 body，后续 overload 仍走现有 late capture / route state 记录流程，不做透明重放。
- route 级健康语义保持现状：retryable overload 继续记为 `route_retryable_failure`，不立刻触发 cooldown。

## 接口契约（Interfaces & Contracts）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- |
| `/v1/responses` early SSE gate | runtime behavior | internal | Modify | pool routing / proxy stream forwarder | metadata-only 起始事件继续缓冲；首个非 metadata 事件才结束透明重试窗口 |
| overload failover ladder | routing behavior | internal | Add | pool failover loop | 原账号 3 次重试后，先同 route 再其他 route |
| `resolve_pool_account_for_request_with_route_requirement()` | Rust helper | internal | Add | overload failover | 仅为局部 route pin 服务；原有 resolver 包装器保持可用 |
| `/v1/responses/compact` pre-forward overload gate | runtime behavior | internal | Add | compact proxy path | 只检查首个未下发 chunk；不做 full-body buffering |

## 验收标准（Acceptance Criteria）

- Given `/v1/responses` 起始阶段先收到 `response.created` / `response.in_progress`，When 随后收到 `response.failed(server_is_overloaded)`，Then 请求必须在未向下游暴露业务事件前执行 overload ladder，而不是立即失败或立即跳去其他 route。
- Given 原账号命中 early overload，When 服务端继续重试，Then 原账号必须总计尝试 4 次，并按 `500ms -> 1s -> 2s` 退避；在这 3 次重试结束前不得提前切其他账号。
- Given 原账号 overload 预算已耗尽且同 route 仍有健康账号，When failover 继续，Then 必须先命中同 route 账号，而不是直接跳去其他 route/channel。
- Given 同 route 候选也全部耗尽，When 仍存在其他 route/channel，Then 请求必须可以切到其他 route/channel 继续完成。
- Given `/v1/responses` 已向下游放出首个非 metadata 事件，When 后续出现 `server_is_overloaded`，Then 不得透明重试，且 route 仍保持 retryable / 无 cooldown 的 late-failure 语义。
- Given `/v1/responses/compact` 在首个未下发 chunk 内返回 `server_is_overloaded`，When 请求仍处于 pre-forward 阶段，Then 必须按同一 overload ladder 重试；但不得引入整包缓冲或 body 已下发后的回放。
- Given `response.failed` 的错误码不是 `server_is_overloaded`，When 它落在 metadata-only 窗口内，Then 仍按原流转发，而不是被误判为透明重试。

## 非功能性验收 / 质量门槛（Quality Gates）

- `cargo fmt`
- `cargo check`
- `cargo test pool_openai_v1_responses_overload -- --test-threads=1`
- `cargo test pool_openai_v1_responses_retries_same_account_on_server_overloaded_before_forwarding -- --test-threads=1`
- `cargo test pool_openai_v1_compact_overload_falls_back_to_alternate_route_before_body_forward -- --test-threads=1`
- `cargo test gate_pool_initial_response_stream_keeps_non_overload_response_failed_on_original_stream -- --test-threads=1`
- `cargo test capture_target_pool_route_marks_server_overloaded_after_forward_as_retryable_without_cooldown -- --test-threads=1`

## 文档更新（Docs to Update）

- `/Users/ivan/.codex/worktrees/1175/codex-vibe-monitor/docs/specs/README.md`

## 方案概述（Approach, high-level）

- 在 proxy failover loop 中引入 overload 专用 route requirement：当 early overload 用尽原账号重试预算后，先把 fresh assignment 限定到相同 route；同 route 无候选后再显式排除该 route，恢复全局路由选择。
- 为 `/v1/responses` 增加单事件级分类，把 `response.created` / `response.in_progress` 留在 metadata-only 窗口里继续缓冲，直到出现 overload、首个非 metadata 事件或 gate 超时/预览上限。
- 为 `/v1/responses/compact` 复用相同的 overload retry outcome，但只检查首个 pre-forward chunk 是否为完整 overload error JSON。
- 通过回归测试覆盖：metadata-prefixed overload、原账号 4 次尝试、same-route 优先、alternate-route 兜底、compact pre-forward overload，以及非 overload `response.failed` 保护用例。
