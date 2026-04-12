# Dashboard / stats 读链路 SQLite 锁冲突治理（#ay33j）

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-12
- Last: 2026-04-12

## 背景 / 问题陈述

- Dashboard 与共享 stats 读接口当前会在 GET 请求里同步执行 hourly rollup catch-up 与历史 summary rollup repair，导致“读中带写”。
- 当 SQLite 同时承受 proxy capture / runtime snapshot / 其他后台写事务时，请求线程里的 rollup 写事务会命中 `database is locked`，并把原本应是只读的 summary / timeseries / working-conversations 等接口直接打成 500。
- 前端 Dashboard 会在 records SSE 后以 1 秒节流静默补拉 summary，放大了偶发锁冲突的可见性；不治理根因，页面会持续出现红色错误横幅。

## 目标 / 非目标

### Goals

- 把 Dashboard 与共享 stats 相关 GET 改成真正只读，请求线程不再执行 live rollup sync 或 archive repair。
- 把 freshness 责任收敛到写侧 / 启动期 / 后台 coalesced maintenance，让接口在锁竞争下优先返回当前已 materialized 的数据而不是 500。
- 保持现有 `/api/**`、`/events`、JSON/SSE 字段、SQLite schema 与页面交互语义兼容。

### Non-goals

- 不替换 SQLite，不改 Dashboard 文案/布局，不重做前端轮询策略。
- 不新增对外接口，不改变 query 参数或 response 字段。

## 范围（Scope）

### In scope

- `summary / timeseries / prompt-cache working-conversations / forward-proxy / sticky-keys / error/failure/perf` 等共享 stats 读接口去写化。
- 后台 hourly rollup catch-up orchestration：在 invocation 写入与启动维护路径中异步 / 后台补齐 live rollups 与历史 summary repair。
- Rust 回归测试、浏览器 Dashboard smoke、PR 收敛到 merge-ready。

### Out of scope

- 任何 UI 视觉改版。
- release / deployment 流程改造。

## 需求（Requirements）

### MUST

- 请求处理线程不得再同步执行 `sync_hourly_rollups_from_live_tables`、`ensure_hourly_rollups_caught_up` 或 invocation summary archive repair。
- 在 SQLite 写锁竞争下，Dashboard 首屏依赖接口保持 2xx，最多返回当前已有 rollup / live tail 数据。
- 写侧或后台维护必须能在有界时间内把新 invocation 收敛进相关 rollup，避免长期陈旧。

### SHOULD

- 复用现有 coalesced worker / startup maintenance 机制，避免再引入无界后台 fan-out。
- 对 lock / missing archive 等非致命情况采用 best-effort 降级并记录日志，而不是升级成用户可见 500。

### COULD

- 顺手补齐 sticky-keys / shared stats surface 的同类只读化，避免留下第二条同源锁冲突入口。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- Dashboard / stats GET 只读取现有 rollup、当前小时 live tail、当前 snapshot，不做任何写事务。
- 新 proxy capture / runtime snapshot 写入后，由后台 coalesced follow-up 异步刷新 invocation/prompt-cache/sticky-key hourly rollups，并在需要时补跑 historical summary repair。
- 启动期 maintenance 负责兜底历史 summary repair/backfill，GET 不再承担 bootstrap 责任。

### Edge cases / errors

- 若后台 catch-up 命中 SQLite 锁竞争，应记录 warning 并等待下一次 coalesced 触发，而不是影响当前用户请求。
- 若历史 archive 文件缺失，继续沿用当前 rollup 读取并保留 pending repair 状态，不把缺失透传成 GET 失败。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

None

### 契约文档（按 Kind 拆分）

None

## 验收标准（Acceptance Criteria）

- Given SQLite 上存在并发写锁，When 请求 `/api/stats/summary?window=today`、`/api/stats/timeseries?range=today&bucket=1m`、`/api/stats/prompt-cache-conversations?...detail=compact`，Then 接口保持 2xx，不再返回 `database is locked` 500。
- Given 请求链路经过本轮改造，When 检查相关 handler / query helper，Then 不再存在同步 live rollup catch-up 或 archive repair 写事务。
- Given 新 invocation / runtime snapshot 写入，When 后台 follow-up 运行，Then invocation / prompt-cache / sticky-key 相关 rollup 会在有界时间内收敛，Dashboard 与共享 stats 不会永久陈旧。
- Given 现有前端与 API consumer，When 升级到本轮实现，Then `/api/**`、`/events`、JSON/SSE 字段与现有页面行为保持兼容。

## 实现前置条件（Definition of Ready / Preconditions）

- 根因已锁定为“GET 请求内同步 rollup 写库”
- 共享受影响面已锁定为 Dashboard + stats 共享读接口
- merge-ready 终点已锁定，不自动 merge / cleanup

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit / integration tests: `cargo test --locked --all-features`（至少覆盖本轮新增/修改的锁冲突与后台 catch-up 回归）

### UI / Storybook (if applicable)

- Not applicable（后端-only）

### Quality checks

- `cargo fmt --all -- --check` ✅
- `cargo check --locked --all-targets --all-features` ✅
- `cargo test --locked --all-features` ✅
- Dashboard 浏览器 smoke（无 `database is locked` 横幅，`summary / timeseries / prompt-cache-conversations` 维持 2xx）✅

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增本 spec 索引，并在实现/PR 收敛后更新状态与备注
- `docs/specs/ay33j-stats-read-path-lock-elimination/SPEC.md`: 记录实施结果与验证

## 计划资产（Plan assets）

- Directory: `docs/specs/ay33j-stats-read-path-lock-elimination/assets/`
- Visual evidence source: not applicable for this backend-only change

## Visual Evidence

- 不适用（本计划不涉及主人可见 UI 改动）

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 移除共享 stats 读链路中的同步 hourly rollup sync / summary repair 写事务
- [x] M2: 引入后台 catch-up orchestration，并把 startup / maintenance 迁移为 repair 兜底入口
- [ ] M3: 完成锁冲突回归、Dashboard smoke、PR/review/CI 收敛到 merge-ready

## 方案概述（Approach, high-level）

- 先把所有 GET handler 与 hourly-backed query helper 去写化，只保留“读已有 rollup + live tail”的路径。
- 再复用现有 coalesced follow-up / startup maintenance 机制，把 live rollup catch-up 与 historical summary repair 收敛到后台 best-effort 任务。
- 最后补锁竞争回归测试，证明接口 2xx 与后台收敛同时成立。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：去掉请求内同步 catch-up 后，若后台触发不完整，Dashboard 可能短暂显示旧 rollup。
- 风险：历史 summary repair 从请求链路剥离后，既有依赖“首次读取自动修复”的测试需要同步调整到后台语义。
- 假设：现有 summary/quota follow-up worker 与 startup maintenance 足以承载本轮 coalesced catch-up。

## 变更记录（Change log）

- 2026-04-12: 创建 stats read-path lock elimination spec，冻结根因、范围、验收与 merge-ready 终点。
- 2026-04-12: 完成共享 stats 读链路去写化、后台 catch-up orchestration 与锁竞争回归；本地 `cargo fmt/check/test` 全绿，Dashboard smoke 未再出现 `database is locked`。

## 参考（References）

- `docs/specs/xvdhm-dashboard-sse-refresh-optimization/SPEC.md`
- `docs/specs/r99mz-dashboard-today-activity-overview/SPEC.md`
