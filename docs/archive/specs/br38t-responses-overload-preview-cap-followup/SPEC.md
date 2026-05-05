# Responses overload early gate preview-cap follow-up（#br38t）

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-22
- Last: 2026-04-22

## 背景

- `#bk2pt` 已把 `/v1/responses` 的 early overload 重试窗扩展到 metadata-only 阶段，但当前实现仍把 raw preview 的 `16 KiB` 上限复用成 early gate 的缓冲上限。
- 当上游先发出超长 `response.created` / `response.in_progress` metadata-only 前缀，再发 `response.failed(server_is_overloaded)` 时，gate 会在失败事件到达前提前放行原流，导致本应发生的透明重试被错过。
- 主人明确锁定边界：**绝不允许先向下游暴露业务内容，再回放或重试**；修复只能发生在 pre-forward / metadata-only 阶段。

## 目标 / 非目标

### Goals

- 让 `/v1/responses` 的 metadata-only early gate 不再被 `RAW_RESPONSE_PREVIEW_LIMIT` 提前截断。
- 保持“只有在业务内容尚未对下游可见时，`response.failed(server_is_overloaded)` 才允许透明重试”的既有边界。
- 保持现有 overload ladder：同账号最多 4 次，然后同 route，再其他 route。

### Non-goals

- 不扩展 `/v1/responses/compact` 语义。
- 不修改 late failure after forward 的处理方式。
- 不修改非 overload 的 `response.failed`、429、transport、timeout、route cooldown 或外部接口。
- 不新增 env/config、数据库迁移或 UI 改动。

## 功能规格

- `RAW_RESPONSE_PREVIEW_LIMIT` 继续只服务于 raw preview / 持久化预览；不再决定 early retry gate 是否继续扫描。
- `/v1/responses` early gate 改用独立的内部缓冲上限，允许读取超过 `16 KiB` 的 metadata-only 前缀，只要仍未命中首个非 metadata 业务事件。
- 当超长 metadata-only 前缀后续出现 `response.failed(server_is_overloaded)` 时，仍必须返回 `RetrySameAccount`，并由现有 failover ladder 继续收口。
- 若 early gate 只是因为新的内部缓冲上限或 gate timeout 结束，则必须原样放行已缓冲字节；不得伪造 late failure，也不得触发透明重试。

## 验收标准

- Given `/v1/responses` 首段是超过 `16 KiB` 的 metadata-only `response.created`，When 下一段才出现 `response.failed(server_is_overloaded)`，Then early gate 仍返回 `RetrySameAccount`，而不是提前 `Forward`。
- Given 同样的超长 metadata-only overload 请求，When 号池首个账号重试后成功，Then 下游只看到最终成功响应，不泄露第一轮 overload 事件，且同账号 attempt 计数按透明重试递增。
- Given 已经向下游发出首个非 metadata 事件，When 后续出现 `server_is_overloaded`，Then 仍保持现有 late retryable failure 语义，不发生透明重试。
- Given `response.failed` 的错误码不是 `server_is_overloaded`，When 它落在 metadata-only 窗口内，Then 仍按原流转发。

## 质量门槛

- `cargo fmt`
- `cargo check`
- `cargo test gate_pool_initial_response_stream_retries_overload_after_metadata_prefix_exceeds_preview_limit -- --test-threads=1`
- `cargo test pool_openai_v1_responses_retries_after_metadata_prefix_exceeds_preview_limit -- --test-threads=1`
- `cargo test pool_openai_v1_responses_retries_same_account_on_server_overloaded_before_forwarding -- --test-threads=1`
- `cargo test gate_pool_initial_response_stream_keeps_non_overload_response_failed_on_original_stream -- --test-threads=1`
- `cargo test capture_target_pool_route_marks_server_overloaded_after_forward_as_retryable_without_cooldown -- --test-threads=1`

## Docs Disposition

- Solution检索: 未命中
- Solution引用: none
- solution_disposition: none
- project_doc_disposition: defer
- Project文档defer记录: reason=`内部代理热路径 bugfix，无新增人类使用说明`; target=`README.md`; follow_up=`若后续需要把 early gate / overload 透明重试边界公开为维护者当前真相，再并入 README 或专门的 proxy 维护文档`

## 变更记录

- 2026-04-22: 创建 `#bk2pt` follow-up spec，冻结“preview cap 不得提前关闭 metadata-only overload 透明重试窗，且永不对 downstream-visible output 做回放”的实现边界。
- 2026-04-22: 完成 early gate buffer cap 解耦，新增超长 metadata-only overload gate / integration 回归；本地 `cargo fmt`、`cargo check` 与 5 条 targeted cargo tests 已通过。
