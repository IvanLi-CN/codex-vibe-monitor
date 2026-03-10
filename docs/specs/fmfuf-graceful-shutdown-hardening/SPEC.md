# 优雅停机补强（#fmfuf）

## 状态

- Status: 部分完成（4/5）
- Created: 2026-03-10
- Last: 2026-03-10

## 背景 / 问题陈述

- 当前服务已具备基础停机能力：主进程会监听 `Ctrl+C` / `SIGTERM`，HTTP 服务接入了 `axum` 的 graceful shutdown，scheduler / retention / forward proxy maintenance 等常驻任务也会监听共享取消信号。
- 现有停机编排主要内联在 `main`，缺少可注入 shutdown trigger 的测试入口，难以对“收到停机信号后是否按预期 drain”进行稳定回归。
- forward proxy 的 Xray 子进程在清理路径中仍直接执行 `kill()`；若子进程能响应温和退出信号，当前实现会跳过这一步，且启动失败与正常下线各自维护一套终止逻辑。
- 代码中仍有少量 detached 后台 worker（如 forward proxy bootstrap probe、penalized probe、summary/quota broadcast worker）未显式感知 shutdown，可能在停机过程中继续派生工作。

## 目标 / 非目标

### Goals

- 提炼可测试的运行时停机编排 helper，使生产路径继续监听 OS signal，测试路径可注入人工 shutdown trigger。
- 保持 HTTP graceful shutdown 行为：收到停机信号后停止接收新连接，并等待当前请求与后台任务在合理时间内收尾。
- 为 Xray 子进程引入统一的两阶段终止 helper：Unix 先尝试 `SIGTERM` 温和退出，超时后再强杀；启动失败与常规清理复用同一路径。
- 让非请求型 detached worker 在 shutdown 期间尽快短路退出，避免停机时继续追加 probe / broadcast 工作。
- 补齐回归测试与关键日志，确保停机链路可验证、可审查。

### Non-goals

- 不修改任何对外 HTTP API、前端交互、数据库 schema 或用户配置项。
- 不引入“第二次信号立即强退”或对外暴露 shutdown timeout 配置。
- 不重做请求生命周期内的流式代理链路；仅在最小必要范围内适配新的 shutdown 抽象。

## 需求（Requirements）

### MUST

- `main` 中的运行时启动/停机编排必须下沉到可测试 helper，helper 需支持注入 shutdown future / trigger。
- 生产入口仍须监听 `Ctrl+C`，并在 Unix 上额外监听 `SIGTERM`。
- HTTP 服务必须继续使用 `with_graceful_shutdown`，且停机后新连接不可再被接受。
- scheduler 在 shutdown 后必须停止发起新一轮调度，并等待当前 in-flight poll 完成后再退出。
- Xray 子进程终止必须统一走同一个 helper；Unix 平台优先发送 `SIGTERM` 并等待短暂 grace period，超时后回退到 force kill。
- Xray 启动失败后的清理路径不得再手写独立 `kill + wait` 逻辑。
- summary/quota broadcast worker、forward proxy bootstrap probe、penalized forward proxy probe 至少要在 shutdown 后停止继续派生新工作。
- 新增测试必须覆盖：HTTP graceful shutdown、runtime helper 停机编排、Xray 两阶段终止、detached worker shutdown 感知。

### SHOULD

- 停机阶段日志至少覆盖：shutdown signal received、HTTP drain started/finished、scheduler drained、xray soft terminate/fallback kill、shutdown complete。
- detached worker 的 shutdown 短路逻辑应保持最小侵入，优先使用现有 `CancellationToken`。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 服务完成 runtime 初始化后，统一由运行时 helper 拉起 HTTP server、scheduler、retention、startup backfill、forward proxy maintenance 等后台任务。
- 生产模式下，helper 接收来自 OS signal listener 的 shutdown future；测试模式下，helper 接收人工注入的 shutdown trigger。
- shutdown future resolve 后：
  - 立即记录停机开始日志并取消共享 token；
  - HTTP server 进入 graceful drain；
  - scheduler / retention / startup backfill / forward proxy maintenance 响应取消并退出；
  - scheduler 若已有 in-flight poll，需等待该 poll 完成；
  - Xray supervisor 统一执行子进程清理；
  - 所有 join handle 完成后记录 shutdown complete。
- detached worker 在 shutdown token 已取消时，应直接返回或停止后续循环，不再继续 probe / broadcast。

### Edge cases / errors

- 若某个后台任务在 shutdown 前已异常退出，主流程仍要继续等待其它任务并记录错误日志。
- 若 Unix soft terminate 发送失败或 grace period 内未退出，必须记录 fallback kill 日志，再执行强杀。
- 非 Unix 平台不要求 soft terminate 语义，但必须保持终止行为安全、可等待、不会遗留配置文件清理回归。
- 若 shutdown 时无 broadcaster receiver 或无待 probe 节点，detached worker 应安静退出，不额外报错。

## 验收标准（Acceptance Criteria）

- Given 服务已启动，When 注入 shutdown trigger，Then HTTP server 结束 accept，新请求连接失败或被拒绝，且运行时 helper 可正常返回。
- Given scheduler 已有 in-flight poll，When shutdown 发生，Then scheduler 不再发起新 poll，并等待当前 poll 结束后再退出。
- Given Xray 子进程可响应 `SIGTERM`，When 触发终止 helper，Then 子进程在 grace period 内退出且不会走 force kill。
- Given Xray 子进程忽略 `SIGTERM`，When 触发终止 helper，Then helper 会回退到 force kill，并最终完成清理。
- Given summary/quota broadcast worker 或 forward proxy probe worker 已准备启动，When shutdown 已经开始，Then worker 不再继续执行新的 broadcast/probe 循环。
- Given 全部后台任务完成退出，When shutdown 流程结束，Then 日志中可观察到停机开始、关键 drain/terminate 事件与停机完成事件。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests：覆盖运行时停机编排、HTTP graceful shutdown、Xray soft terminate / fallback kill、detached worker shutdown 短路。
- 回归测试需保持现有 proxy / forward proxy 行为不变，不新增对外接口断言变更。

### Quality checks

- `cargo fmt --check`
- `cargo test`

## 文档更新（Docs to Update）

- `docs/specs/README.md`：新增 spec 索引，并在交付完成后同步状态、PR 与 checks 结果。
- `docs/specs/fmfuf-graceful-shutdown-hardening/SPEC.md`：记录验收、测试结果与最终交付状态。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 创建 graceful shutdown spec，冻结目标、边界与验收标准。
- [x] M2: 运行时停机编排下沉为可测试 helper，生产路径保留 OS signal 监听。
- [x] M3: Xray 子进程终止统一为两阶段 helper，并在清理与启动失败路径复用。
- [x] M4: detached worker 接入 shutdown 感知，停止 shutdown 期间继续 probe / broadcast。
- [ ] M5: 新增并通过停机相关 Rust 测试；fast-track 交付完成（提交、push、PR、checks、review-loop、spec 同步）。

## 风险 / 假设

- 风险：Unix 信号测试在 CI 环境可能出现偶发波动；若出现，需要通过条件编译或 helper 级测试控制不稳定面。
- 风险：部分 detached worker 由请求路径触发，若 shutdown 时仍有 in-flight 请求，可能出现少量已开始工作在短时间内自然收尾；本次目标仅保证 shutdown 后不继续派生新工作。
- 假设：现有 `CancellationToken` 适合作为共享 shutdown 原语，无需引入新的运行时框架。

## 变更记录（Change log）

- 2026-03-10: 创建 spec，冻结 graceful shutdown 标准补强范围、验收标准与快车道交付要求。
- 2026-03-10: 完成运行时停机编排重构、Xray 两阶段终止、detached worker shutdown 短路与本地测试收敛。

## 参考（References）

- `src/main.rs`
- `src/forward_proxy/mod.rs`
- `docs/specs/9aucy-db-retention-archive/SPEC.md`
