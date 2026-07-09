# 后端结构残余收口与浅层测试模块化（#4tgau）

## 背景 / 问题陈述

- 现有后端虽然已经完成多轮结构整理，但 `src/` 中仍残留大量 `include!()` 拼接入口、`use crate::*;` 全量导入和仅为兼容旧入口存在的 bridge re-export。
- 这些残余主要集中在 `main.rs`、`proxy`、`api/slices`、`upstream_accounts`、`forward_proxy`、`tests` 等热点，导致模块边界仍停留在“物理切片”层，而不是真实 Rust 模块层。
- 当前 active `docs/specs/**` 没有直接承接这批剩余结构债的 topic spec；历史 archived specs 已经完成并归档，不能继续充当当前真相源。

## 目标 / 非目标

### Goals

- 去除 `src/` 中剩余的 `include!()` 结构拼接入口，改成真实 `mod` 树与显式导出。
- 去除剩余 `use crate::*;`，把模块依赖收敛到 `use super::*;` 或明确导入。
- 收口 `main.rs` 的 crate-root bridge re-export / root include 结构，把 `config`、`app_state`、`share_links`、`runtime`、`schema`、`pricing`、`maintenance` 恢复为真实模块归属。
- 将 `src/tests/mod.rs` 与 `src/upstream_accounts/tests.rs` 改成真实测试模块树，同时保持现有大测试切片文件主体基本不动。
- 在不改变 HTTP/SSE/API/schema/env/CLI/SQLite/runtime 语义的前提下完成结构等价重构。

### Non-goals

- 不改任何用户可见行为、接口契约、数据库 schema、环境变量语义或运行时策略。
- 不深拆 `src/tests/slices/*.rs`、`src/upstream_accounts/tests_part_*.rs` 这类大测试文件。
- 不顺手修与本轮结构重构无关的功能缺陷、性能问题或前端结构问题。
- 不要求本轮执行 `proxy-parallel` 与 `raw` 两套 shared-testbox smoke。

## 范围（Scope）

### In scope

- `src/main.rs` 及其 crate-root 结构入口：`config`、`app_state`、`share_links`、`runtime`、`schema`、`pricing`、`maintenance`。
- `src/proxy.rs` 与 `src/proxy/*.rs`。
- `src/api/slices/mod.rs` 与 `src/api/slices/**`。
- `src/upstream_accounts/mod.rs`、`core.rs`、`sync.rs`、`routing/mod.rs`、`tests.rs` 及相关子文件。
- `src/forward_proxy/mod.rs` 与 `src/forward_proxy/slices/*.rs`。
- `src/stats/mod.rs`、`src/sqlite_batch_writer.rs`、`src/external_api.rs` 等剩余 `use crate::*;` 热点。
- `docs/specs/README.md` 与本 spec 目录。

### Out of scope

- 发布流程、版本标签、Release 资产或 CI 策略改动。
- shared-testbox 环境修复或额外 smoke 脚本扩容。
- 任何前端页面、Storybook、视觉证据相关工作。

## 设计约束

- 新模块边界必须使用真实 `mod` / 最小 `pub(crate)` / 显式 `use`；不得以新的 `include!()` 或新的 bridge re-export 模块替代旧结构。
- 测试入口只做浅层真模块化；允许保留大测试文件原有断言、fixture 和 helper 的布局。
- 允许为平滑迁移保留必要的 `pub(crate) use module::*;` 聚合导出，但不得再建立仅为模拟 crate-root 的桥接子模块。
- 结构整理必须优先服务于可导航性与可约束性，不能为了“零 diff”继续保留根级拼接形态。

## 验收标准（Acceptance Criteria）

- `rg 'include!\\(' src` 不再命中 Rust 源码中的结构拼接入口。
- `rg 'use crate::\\*;' src` 不再命中。
- `main.rs` 不再保留 crate-root `include!()` 或仅为兼容旧结构存在的 bridge re-export 模块。
- `src/tests/mod.rs` 与 `src/upstream_accounts/tests.rs` 改为真实 `mod` 树，且大测试切片文件内容仅做最小路径/导入修正。
- `cargo fmt`、`cargo check --locked --all-targets --all-features`、`cargo test` 通过。
- `scripts/shared-testbox-api-read-smoke` 通过，证明 `/health`、`/api/version`、`/api/stats/*` 装配未因结构迁移回归。

## 参考

- `docs/archive/specs/wt76b-backend-structure-convergence/SPEC.md`
- `docs/archive/specs/phb37-backend-structure-convergence-followup/SPEC.md`
- `docs/archive/specs/9vau7-backend-structure-dual-pr-followup/SPEC.md`
- `docs/archive/specs/krsd4-main-rs-structure-refactor/SPEC.md`
- `docs/solutions/performance/rust-backend-test-runtime-feedback-loop.md`
