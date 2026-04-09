# OAuth `/v1/responses` 大包体直通与 distinct-account 记账修复（#gkser）

## 状态

- Status: 已完成
- Created: 2026-04-09
- Last: 2026-04-09
- Note: PR #317：OAuth `/v1/responses` file-backed body 已改为 large-body passthrough；small-body rewrite 保持原语义，distinct-account 预算记账已延后到真实 dispatch，local cargo fmt/check + targeted tests + review-loop clear。

## 背景 / 问题陈述

- 当前 OAuth `/v1/responses` 对 file-backed request body 仍保留本地 `8 MiB` rewrite gate；命中后不会真正发上游，却会在 pool failover 里占用 distinct-account 预算。
- 线上 `proxy-2126-1775745389929` 证明该限制会把真正发出的第二个账号降成 `distinct_account_count=2`，从而丢失 responses-family same-account retry 预算，并最终把一次超时放大成 synthetic `429`。
- 该限制不是上游公开契约，而是本地 bridge 实现为了把 `/v1/responses` body 先物化成 `Bytes` 后改写而加的保护分支；它同时把大包体改写成本留在热路径上。

## 目标 / 非目标

### Goals

- 删除 OAuth `/v1/responses` 的本地大包体 rewrite limit，不再因为 file-backed body 被本地 `413` / skip。
- 让 file-backed `/v1/responses` 改走 passthrough，不再把整包 request materialize 成新的 `Bytes` 副本。
- 保留 small-body `/v1/responses` 的现有 rewrite 语义：补 `instructions`、补 `store=false`、强制 `stream=true`、删除 `max_output_tokens`。
- 修正 pool failover 记账：只有真正进入 upstream dispatch 的账号才计入 `attempted_account_ids` / `poolDistinctAccountCount` / responses-family same-account retry budget。
- 增加可观测性，明确区分 `small_body_rewrite` 与 `large_body_passthrough`。

### Non-goals

- 不修改公共 HTTP API、前端契约、SQLite schema、全局 429/backoff 策略。
- 不改变非 OAuth 路径或 `/v1/responses/compact`、`/v1/chat/completions` 的既有语义。
- 不把 OAuth `/v1/responses` live-body first attempt 改成完全无 replay；本次只收口 file-backed body rewrite 热路径与 distinct-account 预算污染。

## 范围（Scope）

### In scope

- `src/oauth_bridge.rs`：允许 `/v1/responses` 接收 streamed OAuth body，并按 small-body rewrite / large-body passthrough 两条 lane 处理成功响应。
- `src/proxy/section_01.rs`、`src/proxy/section_02.rs`、`src/proxy/section_03.rs`：file-backed body 的 stream hint 提取、OAuth body 构造、distinct-account 记账顺序修正。
- `src/proxy/section_04.rs`、`src/proxy/section_06.rs`：把新的 OAuth body mode / snapshot kind 写进 invocation payload observability。
- `src/tests/slices/pool_failover_window_c.rs` 与 `src/tests/slices/timeseries_parallel_and_quota.rs`：更新大型 body 回归夹具与 distinct-account 预算回归测试。

### Out of scope

- 新增设置项、人工兜底开关或替代 hard cap。
- 对已有 spec 之外的 UI、Release、Retention 或 Dashboard 行为做顺手改动。

## 需求（Requirements）

### MUST

- file-backed OAuth `/v1/responses` 不得再触发本地 rewrite limit，也不得因为该分支跳过而偷吃 distinct-account 预算。
- file-backed OAuth `/v1/responses` 必须保留原始 request body，不新增整包 `Bytes` 副本；bridge 只允许读取 debug prefix 与线性解析 `stream` hint。
- small-body OAuth `/v1/responses` 必须继续保留现有 rewrite 语义与 debug 字段。
- 非 stream 请求在 OAuth upstream 返回 SSE 时，仍必须折叠为单个 JSON 响应；若 upstream 直接返回 JSON，则直接回传该 JSON 成功体。
- invocation payload 必须新增可观测字段，至少能区分 `oauthRequestBodySnapshotKind=file|memory|empty|...` 与 `oauthResponsesBodyMode=small_body_rewrite|large_body_passthrough`。
- pool failover 中，本地 preflight 失败（如 forward proxy 选择失败、OAuth body preflight 失败）不得写入 `attempted_account_ids`，也不得影响 responses-family same-account retry budget。

### SHOULD

- file-backed body 的 `stream` hint 提取应基于线性 JSON parse，而不是正则拼接或整包 materialize。
- 新回归测试应直接覆盖“preflight skip + 次账号 same-account retry 仍生效”的链路，避免只测计数字段。

## 验收标准（Acceptance Criteria）

- Given file-backed OAuth `/v1/responses` 且原始请求 `stream=false`
  When upstream 仍返回 `text/event-stream`
  Then 代理返回单个 JSON completed response，且 upstream 收到的 body 仍保留原始 `stream=false` / `max_output_tokens`。

- Given file-backed OAuth `/v1/responses` 且原始请求 `stream=false`
  When upstream 直接返回 JSON success body
  Then 代理直接回传该 JSON success body，不再误报 `streamed request bodies are not supported for /v1/responses`。

- Given file-backed OAuth `/v1/responses` 且原始请求 `stream=true`
  When upstream 返回 SSE
  Then 代理继续 passthrough SSE，且 invocation observability 标记 `large_body_passthrough`。

- Given sticky 首账号命中本地 preflight 失败、次账号首个 `/v1/responses` attempt 返回可重试 `5xx`
  When 次账号拥有同账号重试预算
  Then 次账号仍可继续 same-account retry 并成功，且最终 `poolDistinctAccountCount=1`。

## 验证

- `cargo fmt --check`
- `cargo check --tests`
- `cargo test pool_route_large_oauth_responses_file_backed_body_passthroughs_non_stream_sse -- --nocapture`
- `cargo test pool_route_large_oauth_responses_file_backed_body_passthroughs_non_stream_json -- --nocapture`
- `cargo test pool_route_large_oauth_responses_file_backed_body_passthroughs_stream_sse -- --nocapture`
- `cargo test pool_route_responses_preflight_failures_do_not_consume_distinct_account_budget -- --nocapture`

## 参考

- `docs/specs/q8h3n-proxy-hot-path-streaming-stability/SPEC.md`
- `docs/specs/uwke5-proxy-upstream-429-retry/SPEC.md`
- `docs/specs/pd77h-oauth-inline-adapter/SPEC.md`

## 变更记录

- 2026-04-09: 创建 spec，冻结 OAuth `/v1/responses` large-body passthrough、small-body rewrite 保留与 distinct-account 记账修复范围。
- 2026-04-09: 完成 file-backed passthrough、gzip stream hint 线性解析、buffered success hop-by-hop header 过滤与 targeted regressions。
