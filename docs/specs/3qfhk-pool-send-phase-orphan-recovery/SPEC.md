# 修复 pool send-phase 孤儿请求长期挂起（#3qfhk）

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-09
- Last: 2026-04-09

## 背景 / 问题陈述

- 线上 101 出现 pool `/v1/responses` 请求在 `sending_request` 阶段长时间停留为 `running/pending` 的事故，直到服务重启后才被 startup recovery 收尾。
- 现有实现只在进程启动时恢复 orphaned invocations / pool attempts；当请求在首字节前被 drop、abort 或遗留为孤儿时，运行中没有对应的 cleanup 路径。
- 这会让 Records、Dashboard 和排障接口持续显示“还在运行”，掩盖真实终态，并放大同账号同路由上的排障成本。

## 目标 / 非目标

### Goals

- 让 pool 早期 pending phase（`connecting` / `sending_request` / `waiting_first_byte`）在运行时也能被安全收尾，不再依赖重启。
- 复用统一的 recovery helper，同时支撑 startup recovery、request-drop cleanup 与 runtime stale sweeper。
- 为 `/v1/responses` oauth send-phase 补齐最小必要观测日志，能区分 send 返回、send timeout 与 handler 首字节前取消。
- 保持现有 API、SQLite schema、UI 契约与 late-stream 终态语义不变。

### Non-goals

- 不修改 overload failover ladder、上游路由策略或账号池挑选逻辑。
- 不新增外部 HTTP endpoint、管理页配置项、env 开关或 DB migration。
- 不改变 `streaming_response` 之后的 success / downstream_closed / late stream error 语义。

## 范围（Scope）

### In scope

- `src/proxy.rs`：共享 recovery helper、early-phase cleanup guard、定向/批量 orphan recovery。
- `src/runtime.rs`：新增 shutdown-aware runtime stale sweeper，并接入现有 maintenance 生命周期。
- `src/oauth_bridge.rs`：补齐 send-phase telemetry。
- `src/tests/mod.rs`：新增 early-phase abort / stale sweeper / startup compatibility 回归测试。
- `docs/specs/README.md` 与本 spec。

### Out of scope

- `web/**` 与 Storybook/视觉证据。
- 上游账号池策略、routing settings 存储结构、SSE payload 形状。
- 非 early-phase 的流式请求中断/补偿模型重构。

## 需求（Requirements）

### MUST

- pool `/v1/responses` 请求若在进入 `streaming_response` 前被取消或遗留为孤儿，必须在 request cleanup 或 runtime stale sweeper 中收口为：
  - attempt: `transport_failure / pool_attempt_interrupted`
  - invocation: `interrupted / proxy_interrupted`
- cleanup guard 只能影响 `connecting` / `sending_request` / `waiting_first_byte`；一旦进入 `streaming_response` 或显式终态，必须立刻解除，且不得覆盖已存在终态。
- startup recovery 必须继续可用，并与 runtime stale sweeper 复用同一 failure taxonomy 与核心收尾逻辑。
- runtime stale sweeper 只能扫描 stale 的 early-phase rows；不得误伤新鲜请求、`streaming_response` 记录或 late-stream `downstream_closed`。
- `/v1/responses` send-phase 日志必须能区分：send started、send returned ok、send returned err、send timeout、guard recovery、sweeper recovery。

### SHOULD

- stale 判定应直接复用现有首字节超时上限并加固定 grace，而不是引入新的配置面。
- runtime stale sweeper 应复用现有 shutdown-aware maintenance 生命周期，停机时可安全退出。
- recovery helper 应尽量支持“按 invoke/attempt 定向收尾”和“按 stale 条件批量收尾”两种模式，避免重复 SQL/rollup 逻辑。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 请求进入 pool oauth 上游阶段后，系统先创建 pending attempt，并在 early phase 持有 cleanup guard。
- 若请求正常推进到 `streaming_response` 或显式终态，guard 立即失效，后续仍沿用现有 finalize 路径。
- 若 handler 在首字节前被取消，guard 会异步触发 best-effort recovery，把 attempt / invocation 收敛为 interrupted。
- 若 guard 未触发或 cleanup 失败，runtime stale sweeper 会周期性扫描 stale early-phase rows 并补收尾。
- 服务重启时，startup recovery 仍会兜底回收残留 running/pending 记录。

### Edge cases / errors

- 同一条记录被 guard、sweeper、startup recovery 重复命中时，只允许第一次成功收尾；后续路径必须保持幂等，不覆盖已终态记录。
- 已进入 `streaming_response` 的请求即使随后 downstream 关闭，也必须保持既有 `downstream_closed` 分类，不得被回写成 interrupted。
- 若 send-phase 在 `request.send()` 内返回超时或错误，日志必须保留对应 phase 与耗时信息，便于与“future 被取消”区分。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- | --- | --- |
| None | None | internal | Modify | None | backend | backend | 无新增/删除对外接口；仅内部恢复与日志行为调整 |

### 契约文档（按 Kind 拆分）

None

## 验收标准（Acceptance Criteria）

- Given 一条 pool `/v1/responses` 请求停在 `sending_request` 且 handler 在首字节前被取消，When cleanup guard 生效，Then 对应 attempt 会在运行时被收口为 `transport_failure / pool_attempt_interrupted`，父 invocation 会被收口为 `interrupted / proxy_interrupted`。
- Given 一条 stale early-phase pending attempt 超过“首字节超时上限 + grace”，When runtime stale sweeper 运行，Then 它会被收尾，且同一父 invocation 也同步收尾。
- Given 一条新鲜 early-phase pending attempt 尚未 stale，When runtime stale sweeper 运行，Then 该记录保持不变。
- Given 一条请求已经进入 `streaming_response` 并最终因为 downstream 提前关闭而结束，When guard 或 sweeper 运行，Then 其终态仍保持 `downstream_closed`，不会被误判为 interrupted。
- Given 服务重启，When startup recovery 扫描到残留 running/pending rows，Then 其 failure taxonomy 与 list/detail 查询语义继续与 runtime cleanup 保持一致。

## 实现前置条件（Definition of Ready / Preconditions）

- 事故路径、目标边界与验收口径已在 101 线上现象与源码比对中锁定。
- 本次修复明确限定为 backend/runtime hotfix，不扩展到上游 routing 策略重构。
- 不新增接口、schema 或配置开关；实现与测试可以直接按现有仓库约定落地。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit / integration tests:
  - `cargo test recover_orphaned_pool_upstream_request_attempts_marks_pending_rows_terminal`
  - `cargo test recover_orphaned_proxy_invocations_marks_running_rows_interrupted`
  - 本次新增 early-phase abort / stale sweeper / streaming guard 相关 Rust 测试

### Quality checks

- `cargo fmt --check`
- `cargo check`
- `cargo test`

## 文档更新（Docs to Update）

- `docs/specs/README.md`：登记 follow-up spec 与状态。
- `docs/specs/3qfhk-pool-send-phase-orphan-recovery/SPEC.md`：记录范围、验收口径与实现状态。

## 计划资产（Plan assets）

- Directory: `docs/specs/3qfhk-pool-send-phase-orphan-recovery/assets/`
- In-plan references: `![...](./assets/<file>.png)`
- Visual evidence source: 本次为 backend/runtime hotfix，默认无需视觉证据。

## Visual Evidence

None

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 follow-up spec 并登记 `docs/specs/README.md`
- [x] M2: 共享 orphan recovery helper + early-phase cleanup guard 落地
- [x] M3: runtime stale sweeper 接入 maintenance 生命周期
- [x] M4: send-phase telemetry 与 Rust 回归测试补齐并验证通过

## 方案概述（Approach, high-level）

- 先把 startup-only recovery 抽成共享 helper，允许 guard、sweeper、startup 三条路径用同一套收尾口径写回 DB 与 rollup。
- 在 pool oauth early-phase 建立轻量 cleanup guard，专门处理“future 在首字节前消失”的情况；进入 `streaming_response` 后立即 disarm。
- 用 shutdown-aware 的定时 sweeper 扫描 stale early-phase rows，作为 guard 失败或遗漏时的运行时兜底。
- send-phase 仅补内部日志与诊断字段，不修改现有外部返回形状。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：若 stale 判定过于激进，可能误伤真实仍在尝试连接的请求；因此必须仅覆盖 early-phase，并绑定现有 timeout ceiling + grace。
- 风险：guard / sweeper / startup recovery 可能并发命中同一记录；实现必须保证幂等更新。
- 假设：现有 routing settings 中的首字节超时已足以作为 stale ceiling，不需要额外配置项。

## 变更记录（Change log）

- 2026-04-09: 创建 follow-up spec，冻结 pool send-phase orphan recovery 的范围与验收口径。
- 2026-04-09: 完成 runtime orphan cleanup guard、stale sweeper、send-phase telemetry 与回归测试；本地 `cargo fmt --check`、`cargo check`、`cargo test` 通过。

## 参考（References）

- `docs/specs/yf3s3-running-invocation-durable-persistence/SPEC.md`
- `docs/specs/bk2pt-responses-overload-early-route-retry/SPEC.md`
- `src/proxy.rs`
- `src/runtime.rs`
- `src/oauth_bridge.rs`
