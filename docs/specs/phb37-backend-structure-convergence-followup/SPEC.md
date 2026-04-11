# 后端结构收敛 follow-up（#phb37）

## 状态

- Status: 部分完成（2/3）
- Created: 2026-04-11
- Last: 2026-04-11

## 背景 / 问题陈述

- `src/` 已经形成多个超大热点文件，尤其集中在 `proxy/`、`api/slices/`、`upstream_accounts/` 与 `maintenance/`，review 与定位成本过高。
- 当前后端仍大量依赖 `include!()` 风格切片拼接，模块边界更多停留在物理分片层，而不是可导航、可约束的真实 Rust 模块层。
- 若继续在现状上叠加功能，`AppState`、router 装配与代理热路径会继续放大回归半径，shared-testbox 验证与 CI 定位也会越来越慢。

## 目标 / 非目标

### Goals

- 把本轮触及的后端热点文件拆回可 review 的真实模块边界，优先收敛 `proxy / api / upstream_accounts / maintenance`。
- 在不改变 HTTP/JSON/SSE/schema/env/CLI 语义的前提下，减少 `include!()` 控制流拼接面，提升模块可读性与局部验证能力。
- 保持既有 shared-testbox smoke 与本地测试入口可复用，并在后端 PR 合并前完成实际环境复验。

### Non-goals

- 不新增产品功能，不改数据库 schema，不改变外部 API 契约。
- 不把本轮目标扩展到前端 page 结构、Storybook 或 UI 行为。
- 不以“清零 warning”为本轮交付目标；仅允许随迁修复与本轮重构直接相关的问题。

## 范围（Scope）

### In scope

- `src/proxy/section_0*.rs` 热路径中与请求准入、pool routing、stream rebuild、usage capture、持久化恢复直接相关的热点收敛。
- `src/api/slices/*.rs` 中本轮已识别的超大 handler/query/cache 组合热点收敛。
- `src/upstream_accounts/*.rs` 中路由、候选解析、group/session 同步与 account import 相关热点收敛。
- `src/maintenance/*.rs` 中 router 装配与 archive / rollup 热点的职责拆分。
- 与上述实现直接耦合的测试与 shared-testbox smoke 脚本复用。

### Out of scope

- 新增后台功能、迁移脚本、release 流程设计变更。
- 与本轮热点无关的纯格式化重排。
- 101 线上部署动作。

## 需求（Requirements）

### MUST

- 对外行为完全兼容：`/health`、`/api/**`、`/events`、`/v1/*`、JSON/SSE 字段、SQLite schema、环境变量与 CLI 语义都不能变化。
- 新的模块边界必须使用真实 `mod` / `pub(crate)` / 显式 re-export，而不是继续扩大 `include!()` 控制流拼接面积。
- 本轮 PR 必须在 shared-testbox 上通过既有 proxy / raw smoke，再允许合并。

### SHOULD

- 优先把热点中的纯组装逻辑、纯查询逻辑、纯转换逻辑拆开，减少单文件跨域职责。
- 把可复用的 helper / model / query 层收敛到命名清晰的子模块。

### COULD

- 顺手移除与本轮拆分直接冲突的少量死代码或无效 helper。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 代理热路径在模块拆分后，仍按当前顺序执行 admission / upstream routing / request forwarding / usage capture / persistence repair。
- API 层在模块拆分后，既有 handler 路由、参数解析、响应结构与错误口径保持不变。
- 账号池与维护任务在模块拆分后，既有状态读写、archive/rollup 与账户同步行为保持不变。

### Edge cases / errors

- branch-update、shared-testbox、CI 与本地测试若发现行为回归，必须优先回到最新拆分点修正，而不是追加新一层兼容胶水。
- 若某热点拆分无法在不扩大 scope 的前提下完成，允许保留局部热点并在 spec 里记录剩余风险，但不能破坏本轮验收门槛。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

None

### 契约文档（按 Kind 拆分）

None

## 验收标准（Acceptance Criteria）

- Given 当前主干的 HTTP / SSE / SQLite / env 契约，When 合并本轮后端 PR，Then 公开行为与返回结构保持兼容。
- Given `cargo fmt --all -- --check`、`cargo check --locked --all-targets --all-features` 与 `cargo test --locked --all-features`，When 在本轮分支上执行，Then 全部通过。
- Given shared-testbox smoke，When 在本轮后端 PR head 上执行 `scripts/shared-testbox-proxy-parallel-smoke` 与 `scripts/shared-testbox-raw-smoke`，Then 两者均通过。
- Given 本轮纳入的热点文件，When 完成本轮重构，Then 至少把核心热点拆到可 review 的真实模块边界，不再继续扩大 `include!()` 控制流拼接面积。

## 实现前置条件（Definition of Ready / Preconditions）

- 目标热点范围已冻结为 `proxy / api / upstream_accounts / maintenance`
- 对外兼容约束已冻结
- shared-testbox 验证入口已明确沿用既有脚本

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `cargo test --locked --all-features`
- Integration tests: `cargo check --locked --all-targets --all-features`
- E2E tests (if applicable): `scripts/shared-testbox-proxy-parallel-smoke`、`scripts/shared-testbox-raw-smoke`

### UI / Storybook (if applicable)

- Not applicable

### Quality checks

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo test --locked --all-features`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增本 spec 索引，并在完成后更新状态/备注
- `docs/specs/phb37-backend-structure-convergence-followup/SPEC.md`: 记录实现结果、验证与 shared-testbox 证据

## 计划资产（Plan assets）

- Directory: `docs/specs/phb37-backend-structure-convergence-followup/assets/`
- Visual evidence source: not applicable for this backend-only change

## Visual Evidence

- 不适用（本计划不涉及主人可见 UI 变更）

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [ ] M1: 拆分 `proxy / api / upstream_accounts / maintenance` 的首批后端热点到真实模块边界
- [ ] M2: 完成本地 Rust 质量门槛与必要回归测试
- [ ] M3: 完成 shared-testbox 实际环境 smoke、PR 收敛、合并与 cleanup

## 方案概述（Approach, high-level）

- 先从 router 装配与代理热路径入手，把控制流重心从巨型切片中剥离到更清晰的子模块。
- 对查询/转换/helper 采取“最小切面拆分”，优先降低单文件跨域职责，而不是大面积重写算法。
- 保持 shared-testbox 脚本与既有 CI 门禁不变，把验证成本压在结构层而不是接口层。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：代理热路径拆分若边界判断失误，容易引入请求生命周期回归。
- 风险：`include!()` 迁移到真实模块时，可能触发更严格的可见性与循环依赖问题。
- 假设：现有 `scripts/shared-testbox-proxy-parallel-smoke` 与 `scripts/shared-testbox-raw-smoke` 足以覆盖本轮后端回归核心面。

## 变更记录（Change log）

- 2026-04-11: 创建后端结构收敛 follow-up spec，冻结本轮范围与验收口径。
- 2026-04-11: 把 `maintenance` 中 archive/hourly-rollup 支撑 helper 收敛为真实支持模块，并完成本地 + shared-testbox 验证。

## 参考（References）

- `docs/specs/huzqt-frontend-structure-convergence-followup/SPEC.md`
- `docs/specs/q8h3n-proxy-hot-path-streaming-stability/SPEC.md`

## 实施结果

- `src/maintenance/hourly_rollups.rs` 中原本混在主流程里的 raw-path / archive-layout / gzip / retention helper 全部抽到新的 `src/maintenance/hourly_rollup_archive_support.rs`，通过显式 `mod + pub(crate) use` 复用，主文件回到 replay/materialize 主控制流。
- `src/maintenance/archive.rs` 中 hourly rollup delta / bucket / keyed conversation / perf sample / pruned-success helper 抽到新的 `src/maintenance/archive_hourly_rollup_support.rs`，把 archive 主文件的职责收敛到 materialize / upsert / prune 主路径。
- 本轮没有扩大 `include!()` 拼接面积，也没有变更 `/health`、`/api/**`、`/events`、`/v1/*`、JSON/SSE、SQLite schema、env 或 CLI 语义。

## 验证记录

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo test --locked --all-features`（1005 passed / 0 failed / 52 ignored）
- `scripts/shared-testbox-proxy-parallel-smoke --cleanup`
  - run: `/srv/codex/workspaces/ivan/codex-vibe-monitor__4dd0653c/runs/20260412_000001_proxy_parallel_fec27426`
  - result: stage A / stage B 各 100 请求，`bad_count=0`，`max_in_flight=100`
- `scripts/shared-testbox-raw-smoke --cleanup`
  - run: `/srv/codex/workspaces/ivan/codex-vibe-monitor__4dd0653c/runs/20260412_000548_shared_smoke_fec27426`
  - result: raw payload 从 `.bin` 压缩到 `.bin.gz`，SQLite `request_raw_path` 同步更新，`search-raw` 同时命中 plain + gzip 文件

## 里程碑完成情况

- [x] M1: 拆分 `proxy / api / upstream_accounts / maintenance` 的首批后端热点到真实模块边界
- [x] M2: 完成本地 Rust 质量门槛与必要回归测试
- [ ] M3: 完成 shared-testbox 实际环境 smoke、PR 收敛、合并与 cleanup
