# 号池暂时无号时的 10 秒有界等待与 503 终态（#667ae）

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-04
- Last: 2026-04-04

## 背景 / 问题陈述

- 线上 `pool_no_available_account` 故障会在本地选路层快速失败，典型延迟只有 `100-200ms`，但实际恢复窗口常落在几秒到十几秒内。
- 当前 generic no-account 终态被包装成 `502`，把短时资源空窗误导成上游网关故障，也没有给调用方明确的重试提示。
- `resolve_pool_account_for_request()` 的 `Unavailable / NoCandidate` 本身只表达“当前拿不到 fresh candidate”，并不代表永久无解；更合理的策略是服务端先吸收短时抖动。

## 目标 / 非目标

### Goals

- 当 pool 进入 generic `Unavailable / NoCandidate` 时，服务端先内部等待最多 `10s`，有账号恢复即可继续同一请求。
- 若等待窗口结束仍无账号，则对调用方返回 `503 Service Unavailable`，并带 `Retry-After: 10`；但如果当前请求已经拿到具体 upstream failure 且之后只是 `NoCandidate` 耗尽，则继续保留那个具体 upstream 错误。
- 保持 `RateLimited -> 429`、`DegradedOnly -> 503`、`pool_no_alternate_upstream_after_timeout -> 502` 的现有语义不变。
- `BlockedByPolicy` 改为即时 `503`，保留具体错误 message，但不进入等待。
- 纯等待阶段不得写入伪造的 `pool_upstream_request_attempts`。

### Non-goals

- 不做 stickyKey 折叠、singleflight、队列化等待或同 key 请求合并。
- 不改 `resolve_pool_account_for_request()` 的枚举设计、账号排序、并发判定或 tag / sticky 规则。
- 不引入新的 env 变量、API 参数或管理面开关。
- 不处理账号池扩容、账号修复或前端界面改造。

## 功能与行为规格（Functional / Behavior Spec）

- 新增 caller-side bounded wait helper，默认内部参数固定为 `timeout=10s`、`poll_interval=250ms`、`retry_after=10s`。
- `proxy_openai_v1_via_pool()` 的“header sticky 先解析初始账号”入口必须复用该 helper，避免绕过等待逻辑。
- `send_pool_request_with_failover()` 在 generic no-account 分支也必须复用该 helper；但当终态已经是 `429 exhaustion` 或 `no alternate after timeout` 时，不进入该等待。
- 等待期间每轮只重新调用既有 resolver；在真正选中账号前，不得创建 upstream attempt row。
- generic no-account 最终 message 保持 `no healthy pool account is available`，仅状态码从 `502` 改为 `503`。
- `Unavailable` 在等待耗尽后必须落到新的 generic `503`；`NoCandidate` 若没有更具体的 `last_error` 才落到 generic `503`，否则保留最后一次真实 upstream failure。
- `BlockedByPolicy` 对外状态码改为 `503`，但不附带 `Retry-After`。

## 接口契约（Interfaces & Contracts）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- |
| pool generic no-account terminal | HTTP behavior | external | Modify | pool callers | 从立即 `502` 改为 bounded wait 后 `503 + Retry-After: 10` |
| `ProxyErrorResponse` | Rust struct | internal | Modify | proxy HTTP adapter | 增加 `retry_after_secs`，用于注入响应头 |
| `PoolNoAvailableWaitSettings` | Rust runtime struct | internal | Add | pool callers / tests | 内部默认固定值；测试 helper 可缩短 timeout 与 poll interval |
| `resolve_pool_account_for_request_with_wait()` | Rust helper | internal | Add | pool routing callers | caller-side bounded wait，不改 resolver 契约 |

## 验收标准（Acceptance Criteria）

- Given pool 当前只有 generic `Unavailable / NoCandidate`，When 新请求到达，Then 服务端最多等待 `10s`，而不是立即返回 `502`。
- Given 等待窗口内账号恢复可选，When 同一请求继续执行，Then 请求成功命中上游，且等待阶段没有伪造 attempt row。
- Given 等待窗口结束仍无账号且不存在更具体的 upstream failure，When 请求返回，Then 调用方收到 `503`、body 为现有 error JSON 壳，且包含 `Retry-After: 10`。
- Given 当前请求已经拿到具体 upstream failure，When 后续 fresh candidate 在等待后仍是 `NoCandidate`，Then 对外继续保留该 upstream failure，而不是改写成 generic `503`。
- Given 终态属于 `BlockedByPolicy`，When 请求返回，Then 状态码为 `503` 且 message 保持具体原因，不等待、不附 `Retry-After`。
- Given 终态属于 `RateLimited`、`DegradedOnly` 或 `pool_no_alternate_upstream_after_timeout`，When 请求返回，Then 它们保持原有状态码与 message，不被这次修复改变。

## 非功能性验收 / 质量门槛（Quality Gates）

- `cargo fmt --check`
- `cargo check`
- `cargo test pool_route_waits_for_header_sticky_account_before_first_attempt -- --test-threads=1`
- `cargo test pool_route_body_sticky_returns_503_after_wait_timeout -- --test-threads=1`
- `cargo test pool_route_keeps_generic_no_candidate_when_other_accounts_are_unavailable_for_other_reasons -- --test-threads=1`
- `cargo test pool_route_returns_specific_ungrouped_error_when_all_candidates_are_ungrouped -- --test-threads=1`
- `cargo test pool_route_returns_ungrouped_error_for_sticky_account_when_cut_out_is_forbidden -- --test-threads=1`

## 文档更新（Docs to Update）

- `docs/specs/README.md`

## 方案概述（Approach, high-level）

- 在 pool caller 层增加 bounded wait helper，把 `Unavailable / NoCandidate` 从“立即失败”改成“重复 resolve 直到 deadline 或成功”。
- 通过 `ProxyErrorResponse.retry_after_secs` 在最外层 HTTP 响应统一注入 `Retry-After`，避免大范围改写内部错误结构。
- 测试通过内部 runtime wait settings 缩短 timeout / poll interval，确保回归仍是毫秒级。
