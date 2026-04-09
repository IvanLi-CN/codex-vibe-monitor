# 代理热路径并发稳定性与传输背压收口（#q8h3n）

## 状态

- Status: 已完成
- Note: 已移除错误的 whole-proxy admission gate，`PROXY_REQUEST_CONCURRENCY_*` 已进入 deprecated/ignored 兼容态；共享测试机 `codex-testbox` 100 并行压测通过，确认 `/v1/*` 不再因本地 admission gate 返回 `503`。

## 背景 / 问题陈述

- 已落地的 raw 异步旁路、records 查询收敛与 summary/quota debounce 确实缓解了 `180s first-chunk timeout`、`database is locked` 与热路径抖动。
- 但新增的 `/v1/*` whole-proxy admission gate 把本地默认并行上限锁到了 `12`，并在高峰窗口内稳定产生日志 `proxy request concurrency wait timed out` 与本地 `503 proxy concurrency limit reached; retry later`。
- 产品硬性要求是“至少 100 个并行调用在工作”，因此该 gate 不是保护，而是新的功能性缺陷；必须回退。

## 目标 / 非目标

### Goals

- 移除 `/v1/*` whole-proxy admission gate，确保至少 100 个同时在途请求不会被本地入口限流直接拒绝。
- 让 request raw 与 response raw 从业务转发热路径旁路出去；资源紧张时优先丢 raw，不阻塞代理流。
- 让大 body sticky/rewrite 不再默认整包内存物化；大体积 body 只做前缀探测，必要时回落到 file-backed replay。
- 让 `/api/invocations` 分页主查询只对当前页记录执行重型投影，并让 summary/quota follow-up 在 burst 写入时自动合并。
- 在共享测试机 `codex-testbox` 上完成 100 并行压测，验证本地 admission reject 已消失，且已有效的 hot-path 修复没有回退。

### Non-goals

- 不改变外部 API 语义、响应格式或 SSE 事件顺序。
- 不做数据库引擎迁移，不引入新的持久化后端。
- 不把问题收口为单纯的配置调参或运营规避。

## 范围（Scope）

### In scope

- `src/proxy.rs` 中 `/v1/*` 请求入口、pool 路由 body 处理、request/response raw capture 与 summary follow-up 调度。
- `src/config.rs` / `src/app_state.rs` / `src/runtime.rs` 中 whole-proxy admission gate 的移除、纯观测型 in-flight 指标与 deprecated 配置告警。
- `src/api/mod.rs` 中 invocation 列表分页查询的轻量页 id 预选 + 当前页重型投影（保留并复验，不回退）。
- `src/tests/mod.rs` 中与 100 并行、不再本地 admission reject、raw 异步和长流 in-flight tracking 相关的回归测试。
- `scripts/shared-testbox-proxy-parallel-smoke` 共享测试机压测脚本与对应的 mock upstream / loadgen harness。

### Out of scope

- retention / archive 离线链路改造。
- 详情页 raw 文件读取语义调整。
- 前端 UI 结构改造。

## 设计约束

- `/v1/*` 不允许再存在任何整机级 whole-proxy admission gate；观测可以保留，准入拒绝不允许保留。
- response raw 必须“先转发 chunk，再异步落盘”；request raw 必须“先发上游/继续流程，再异步收尾写盘”。
- raw 异步旁路必须有明确截断原因，至少覆盖 `max_bytes_exceeded`、`write_failed:*`、`async_backpressure_dropped`。
- 大 body sticky 探测只允许读取固定前缀窗口；超过窗口仍未识别时，直接回落到“无 body sticky 优化”的 replay 路径。
- summary/quota follow-up 必须具备 burst coalesce，避免每条新记录都立即跑完整汇总。
- `PROXY_REQUEST_CONCURRENCY_LIMIT` / `PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS` 只能作为弃用兼容项继续被读取与告警，不得再影响 `/v1/*` 准入。

## Task Orchestration

- wave: 1
  - main-agent => 从 `/v1/*` 入口移除 whole-proxy admission gate，把 permit 改成纯观测型 in-flight tracking，并对 `PROXY_REQUEST_CONCURRENCY_*` 输出 deprecated/ignored 告警 (skill: $fast-flow)
- wave: 2
  - main-agent => 保留并复验 request/response raw 异步旁路与 summary/quota debounce，确保移除 gate 后业务流优先级不回退 (skill: $fast-flow)
- wave: 3
  - main-agent => 补齐本地回归：100 并行不再本地 503、长流期间 in-flight tracking 到 body 结束、raw 饱和只丢 capture、不影响业务流 (skill: $fast-flow)
- wave: 4
  - main-agent => 新增 `scripts/shared-testbox-proxy-parallel-smoke`，在 `codex-testbox` 上用隔离 run dir、唯一 Compose project、LXC caps 兼容 override 跑 100 并行压测 (skill: $shared-testbox-runner)
- wave: 5
  - main-agent => 完成本地验证、共享测试机压测、fast-track review/PR/merge/release，并在 101 与浏览器侧复验不再出现本地 admission reject (skill: $fast-flow)

## 验收标准（Acceptance Criteria）

- `codex-testbox` 上 100 个同时发起的 `/v1/*` 代理请求不会出现任何本地 `503 proxy concurrency limit reached; retry later`。
- response raw append 不再位于 chunk 转发之前；request raw 写盘不再阻塞上游发送。
- 大于小体积阈值的 pool request body 不再默认整包内存物化；sticky 探测仅依赖前缀窗口或 replay snapshot 前缀。
- `/api/invocations` 的分页主查询只先选出当前页 id，再对当前页记录执行完整投影。
- summary/quota follow-up 在 burst 写入时能够合并，不再对每条记录立即触发一次完整汇总。
- 即使线上环境仍设置 `PROXY_REQUEST_CONCURRENCY_LIMIT` / `PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS`，它们也只会产生日志告警，不会改变 `/v1/*` 准入行为。

## 验证

- `cargo fmt --check`
- `cargo check --tests`
- `cargo test proxy_request_tracking_can_reach_100_in_flight_without_local_rejection -- --nocapture`
- `cargo test proxy_openai_v1_via_pool_reads_request_body_without_local_admission_gate -- --nocapture`
- `cargo test list_invocations_ -- --nocapture`
- `scripts/shared-testbox-proxy-parallel-smoke`

## 参考

- `/Users/ivan/.codex/worktrees/4032/codex-vibe-monitor/src/proxy.rs`
- `/Users/ivan/.codex/worktrees/4032/codex-vibe-monitor/src/api/mod.rs`
- `/Users/ivan/.codex/worktrees/4032/codex-vibe-monitor/docs/specs/n7c2r-proxy-hot-path-no-raw-reread/SPEC.md`
