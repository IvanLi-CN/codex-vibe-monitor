# 后端结构债双 PR 快车道（#9vau7）

## 状态

- Status: 待实现
- Created: 2026-04-12
- Last: 2026-04-12

## 背景 / 问题陈述

- `src/proxy.rs`、`src/api/mod.rs` 与 `src/upstream_accounts/mod.rs` 仍把大量生产控制流停留在 `include!()` 级别，真实模块边界不清晰。
- `src/upstream_accounts/routing.rs`、`src/api/slices/prompt_cache_and_timeseries.rs` 与 `src/maintenance/hourly_rollups.rs` 仍是 review 半径很大的热点文件。
- 上一轮 `phb37` 已收敛 `maintenance` 支撑 helper，但核心 runtime / api/router 根节点仍未回收到真实 `mod` 树。

## 目标 / 非目标

### Goals

- 用两条顺序 PR 把高风险伪模块化根节点收回到真实 Rust `mod` 边界。
- PR1 聚焦 `proxy + upstream_accounts/routing`；PR2 聚焦 `api + hourly_rollups router` 与 shared-testbox API read smoke。
- 保持 `/health`、`/api/**`、`/events`、`/v1/*`、JSON/SSE 字段、SQLite schema、CLI/env 语义完全兼容。

### Non-goals

- 不深拆 `src/maintenance/archive.rs`。
- 不做 `AppState` 全量字段分组重构。
- 不以 `cargo check` warning 清零为本轮交付目标。

## 范围（Scope）

### In scope

- `src/proxy.rs` 与 `src/proxy/**`：把根级 `include!()` 收敛到真实子模块。
- `src/upstream_accounts/mod.rs` 与 `src/upstream_accounts/routing.rs`：把根级与 routing 热点拆成真实 `mod` 树。
- `src/api/mod.rs` 与 `src/api/slices/prompt_cache_and_timeseries.rs`：把根级与超大读侧热点拆成真实 `mod` 树。
- `src/maintenance/hourly_rollups.rs`：把 `.route(...)` 装配抽成 domain router builders。
- `scripts/shared-testbox-api-read-smoke`：新增后端 read-side shared-testbox smoke。

### Out of scope

- 数据库 schema 变更。
- 新增产品功能。
- 前端页面、Storybook 或视觉交付面。

## 验收标准（Acceptance Criteria）

- PR1：`src/proxy.rs` 与 `src/upstream_accounts/mod.rs` 不再使用根级 `include!()`；`routing` 热点拆回真实子模块；本地 `cargo fmt/check/test` 与 shared-testbox `proxy-parallel/raw` smoke 通过。
- PR2：`src/api/mod.rs` 不再使用根级 `include!()`；`prompt_cache_and_timeseries` 拆回真实子模块；`hourly_rollups` route 装配收敛成 domain router builders；新增 API read smoke 并通过。
- 两条 PR 合并前都必须完成 `$codex-review-loop` 收敛，标签固定为 `type:skip` + `channel:stable`。
- 本轮不得把 warning 基线从当前 `cargo check --locked --all-targets --all-features` 的 40 条继续抬高，也不得新增新的 warning 家族。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo test --locked --all-features`
- `scripts/shared-testbox-proxy-parallel-smoke --cleanup`
- `scripts/shared-testbox-raw-smoke --cleanup`
- `scripts/shared-testbox-api-read-smoke --cleanup`（PR2）

### UI / Storybook (if applicable)

- Not applicable

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/9vau7-backend-structure-dual-pr-followup/SPEC.md`

## Visual Evidence

- 不适用（纯后端结构债收敛）

## 实现里程碑（Milestones / Delivery checklist）

- [ ] M1: 建立双 PR follow-up spec 并确认基线对齐 `main`
- [ ] M2: 完成 PR1 (`proxy + upstream_accounts/routing`) 的结构收敛、验证、PR、合并与 cleanup
- [ ] M3: 完成 PR2 (`api + router builders + api-read smoke`) 的结构收敛、验证、PR、合并与 cleanup

## 方案概述（Approach, high-level）

- 先用最小行为面改动把 `include!()` 根节点换成真实子模块，再对超大热点做职责拆分。
- 所有拆分都以现有函数签名、查询顺序、route/path、数据库读写与响应形状保持不变为前提。
- shared-testbox 继续使用隔离 run directory + compose project；新增 API smoke 只覆盖 read-side 端点与响应形状。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：`include!()` 迁移成真实模块后，隐藏的可见性依赖会在编译期暴露，需要通过最小 `use super::*;` / `pub(crate)` 收口。
- 风险：`hourly_rollups` router builder 抽离若遗漏 handler 绑定，容易引入启动时路由缺失；必须靠 shared-testbox API smoke 回归。
- 假设：当前 GitHub main head = `c8afc4a95433e1ea14d4343023565ceab01c904c`，本地 `origin/main` 与之对齐。

## 变更记录（Change log）

- 2026-04-12: 创建双 PR 后端结构债 fast-track spec，冻结 PR1/PR2 范围与 merge+cleanup 终点。

## 参考（References）

- `docs/specs/wt76b-backend-structure-convergence/SPEC.md`
- `docs/specs/phb37-backend-structure-convergence-followup/SPEC.md`
