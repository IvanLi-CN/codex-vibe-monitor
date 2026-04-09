# 代理热路径并发稳定性与传输背压收口（#q8h3n）

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Note: `/v1/*` 代理热路径收口已完成：请求级并发背压、request/response raw 异步旁路、大 body sticky/rewrite 降级、records 列表轻量分页与 summary/quota follow-up debounce 已落地；本地 `cargo check --tests` 与 targeted cargo tests 已通过，等待 PR 阶段收敛。

## 背景 / 问题陈述

- 线上高峰窗口已出现稳定的 `waiting for first upstream chunk` 超时、raw payload 磁盘爆涨和 records 慢查询。
- 现有实现把 response raw append 放在 chunk 转发之前，把 request raw 写盘放在请求路径上，并且在部分 sticky/rewrite 场景下默认物化整包 body。
- `/api/invocations` 首屏列表查询会在分页主查询里重复计算大段 `json_extract + CASE` 表达式，summary/quota follow-up 也会在高频写入时持续触发。

## 目标 / 非目标

### Goals

- 让 `/v1/*` 代理拥有真正的请求级并发背压，超过上限时只做短等待后失败。
- 让 request raw 与 response raw 从业务转发热路径旁路出去；资源紧张时优先丢 raw，不阻塞代理流。
- 让大 body sticky/rewrite 不再默认整包内存物化；大体积 body 只做前缀探测，必要时回落到 file-backed replay。
- 让 `/api/invocations` 分页主查询只对当前页记录执行重型投影，并让 summary/quota follow-up 在 burst 写入时自动合并。

### Non-goals

- 不改变外部 API 语义、响应格式或 SSE 事件顺序。
- 不做数据库引擎迁移，不引入新的持久化后端。
- 不把问题收口为单纯的配置调参或运营规避。

## 范围（Scope）

### In scope

- `src/proxy.rs` 中 `/v1/*` 请求入口、pool 路由 body 处理、request/response raw capture 与 summary follow-up 调度。
- `src/config.rs` / `src/app_state.rs` / `src/runtime.rs` 中新增的代理级并发/异步采集运行态。
- `src/api/mod.rs` 中 invocation 列表分页查询的轻量页 id 预选 + 当前页重型投影。
- `src/tests/mod.rs` 中与 raw 异步、并发背压、分页查询和 summary debounce 相关的回归测试。

### Out of scope

- retention / archive 离线链路改造。
- 详情页 raw 文件读取语义调整。
- 前端 UI 结构改造。

## 设计约束

- 代理请求级背压只作用于 `/v1/*` 上游代理链路，不影响普通 UI/API 读请求。
- response raw 必须“先转发 chunk，再异步落盘”；request raw 必须“先发上游/继续流程，再异步收尾写盘”。
- raw 异步旁路必须有明确截断原因，至少覆盖 `max_bytes_exceeded`、`write_failed:*`、`async_backpressure_dropped`。
- 大 body sticky 探测只允许读取固定前缀窗口；超过窗口仍未识别时，直接回落到“无 body sticky 优化”的 replay 路径。
- summary/quota follow-up 必须具备 burst coalesce，避免每条新记录都立即跑完整汇总。

## Task Orchestration

- wave: 1
  - main-agent => 为代理入口补上请求级并发背压与日志指标，并把 permit 生命周期延长到响应流结束 (skill: $normal-flow)
- wave: 2
  - main-agent => 将 request raw 改为异步收尾写盘、将 response raw 改为有界异步 writer，保证业务转发优先于 raw 落盘 (skill: $normal-flow)
- wave: 3
  - main-agent => 收紧 pool body sticky/rewrite 路径：大 body 仅前缀探测 sticky，rewrite 只保留小体积 materialize (skill: $normal-flow)
- wave: 4
  - main-agent => 将 `/api/invocations` 改成“页 id 预选 + 当前页重型投影”，并给 summary/quota follow-up 增加 debounce/coalesce (skill: $normal-flow)
- wave: 5
  - main-agent => 补齐 targeted tests、完成本地验证与 spec 状态同步，收口到普通流程的 local PR-ready (skill: $normal-flow)

## 验收标准（Acceptance Criteria）

- `/v1/*` 代理在达到并发上限时只做短等待后返回明确可重试错误，并记录在途数/排队等待/拒绝统计。
- response raw append 不再位于 chunk 转发之前；request raw 写盘不再阻塞上游发送。
- 大于小体积阈值的 pool request body 不再默认整包内存物化；sticky 探测仅依赖前缀窗口或 replay snapshot 前缀。
- `/api/invocations` 的分页主查询只先选出当前页 id，再对当前页记录执行完整投影。
- summary/quota follow-up 在 burst 写入时能够合并，不再对每条记录立即触发一次完整汇总。

## 验证

- `cargo fmt --check`
- `cargo check`
- `cargo test raw_payload -- --nocapture`
- `cargo test proxy_request_concurrency -- --nocapture`
- `cargo test list_invocations -- --nocapture`

## 参考

- `/Users/ivan/.codex/worktrees/4032/codex-vibe-monitor/src/proxy.rs`
- `/Users/ivan/.codex/worktrees/4032/codex-vibe-monitor/src/api/mod.rs`
- `/Users/ivan/.codex/worktrees/4032/codex-vibe-monitor/docs/specs/n7c2r-proxy-hot-path-no-raw-reread/SPEC.md`
