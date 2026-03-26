# 代理热路径停止 response raw 二次回读（#n7c2r）

## 状态

- Status: 已实现，待 PR / CI 收敛
- Note: `/v1/responses` 与 `/v1/responses/compact` 的 capture 热路径已改为只依赖 live stream parser 与 bounded preview；完整 raw 文件仍照常落盘，但请求处理阶段不再为判型或 metadata 补全回读 `response_raw_path`。

## 背景 / 问题陈述

- 代理已经把响应原文写入 `response_raw_path`，同时 `raw_response` 只保留 preview。
- 现有成功路径在某些流式或大响应场景下，仍会为了 SSE 判型或 metadata 补全再次读取刚刚写出的 raw 文件。
- 这类二次回读会引入额外磁盘读取与额外热路径复制，而且并不会改变客户端可见的 HTTP / SSE 行为。
- 主人已经明确约束：不能降低 raw capture 保真度，也不能为了降内存引入新的磁盘 IO。

## 目标 / 非目标

### Goals

- 让 `/v1/responses` 与 `/v1/responses/compact` 的成功热路径不再回读 `response_raw_path`。
- 让 `response_is_stream_hint` 仅基于 live 证据判定：响应头、live stream parser、preview 内可识别的 SSE 特征。
- 保持 `request_raw_path`、`response_raw_path`、`raw_response` preview、raw size / truncate 语义不变。
- 在极少数超大或异常响应场景下，允许 metadata 退化为“仅保留 live 已提取字段 + `usageMissingReason`”，而不是回退到 raw 文件补全。

### Non-goals

- 不修改 request body 读取与 replay 路径。
- 不修改 raw capture 的保真度、落盘策略、路径命名或截断规则。
- 不新增 file-backed replay、临时文件或其他磁盘写入。
- 不承诺单靠这次改动显著降低整体 RSS；收益重点是去掉热路径二次磁盘读取和一部分额外复制。

## 范围（Scope）

### In scope

- `src/main.rs` 中 proxy capture 收尾阶段的 stream hint / response metadata 判型逻辑。
- preview-only 的宽松解码与 bounded parse 路径。
- `src/tests/mod.rs` 中大流 / 大 JSON / 压缩流回归测试，外加热路径不再 raw 回读的显式断言。
- `docs/specs/README.md` 与本 spec 的状态同步。

### Out of scope

- request body 流式化或 bounded replay。
- retention、archive、WAL、页缓存或 allocator 调优。
- `/api/...` 返回字段结构调整。

## 对外行为与内部契约

- 客户端可见的 HTTP 状态码、响应体、SSE 事件顺序与结束语义保持不变。
- `response_raw_path` 继续保存完整 raw 原文；`raw_response` 继续只保存 preview。
- 代理请求处理热路径不得再调用基于 raw 文件的 SSE hint fallback 或 response parse fallback。
- 离线/详情读取路径允许保留 raw 文件 helper，但它们不再属于在线 capture 成功路径。

## 设计约束

- `response_is_stream_hint` 只允许使用三类 live 信号：
  - `Content-Type: text/event-stream`
  - `StreamResponsePayloadParser::saw_stream_fields`
  - preview 经过 bounded / lossy decode 后仍可识别的 SSE 头行
- 对流式响应：
  - 优先使用 live `StreamResponsePayloadParser` 产物
  - 若因 oversized line、preview 截断或压缩预览不完整导致 metadata 缺失，只保留 live 已得字段，并通过 `usageMissingReason` 明确记录原因
- 对非流式响应：
  - 继续使用 bounded prefix parse
  - 不再额外回读 raw 文件补全 metadata

## Task Orchestration

- wave: 1
  - main-agent => 在 `src/main.rs` 中移除成功热路径对 `response_raw_path` 的 SSE hint / parse 回读，改为只使用 live parser 与 preview decode (skill: $fast-flow)
- wave: 2
  - main-agent => 保留 raw-file helper 作为非热路径能力，并新增测试计数器证明 proxy capture 热路径不再触发 raw fallback (skill: $fast-flow)
- wave: 3
  - main-agent => 更新 `src/tests/mod.rs`，覆盖 gzip 大流、超大终态 SSE、大非流 JSON、raw 截断等场景，并断言热路径 raw reread 次数为零 (skill: $fast-flow)
- wave: 4
  - main-agent => 执行本地验证、同步 spec 状态、创建 PR 并收敛到 merge-ready (skill: $plan-sync + $codex-review-loop + $fast-flow)

## 验收标准（Acceptance Criteria）

- `/v1/responses` 与 `/v1/responses/compact` 的 capture 成功路径不再调用 raw-file SSE hint fallback 或 raw-file response parse fallback。
- 客户端可见的代理行为不变；raw capture 路径、大小、截断标记与 preview 上限不变。
- 标准 SSE、大 gzip SSE、超大终态 SSE、大非流 JSON 等回归全部通过。
- 测试能够显式证明热路径 raw reread 计数为零。
- 当 metadata 无法仅凭 live parser / preview 补全时，记录必须稳定带出 `usageMissingReason`，而不是静默回退到 raw reread。

## 验证

- `cargo fmt --check`
- `cargo check`
- `cargo test proxy_capture_target_ -- --nocapture`
- `cargo test proxy_capture_target_large_stream_soak_keeps_rss_within_stable_window -- --ignored --nocapture --test-threads=1`

## Change log

- 将 proxy capture 成功热路径中的 raw-file SSE hint / response parse fallback 从在线请求链路移除，保留 raw helper 作为非热路径能力，并补上“热路径 raw reread 为零”的回归断言。

## 参考

- `src/main.rs`
- `src/tests/mod.rs`
- `docs/specs/9aucy-db-retention-archive/SPEC.md`
