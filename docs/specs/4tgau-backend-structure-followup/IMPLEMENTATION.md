# 后端结构残余收口与浅层测试模块化 - Implementation

## Current State

- Canonical spec: `docs/specs/4tgau-backend-structure-followup/SPEC.md`
- Implementation summary: 已完成
- 本轮完成：清除 `src/` 中剩余 `include!()` / `use crate::*;` / bridge re-export 结构债，并把测试入口收口到真实模块树。

## 状态

- Status: 完成
- Note: 本轮限定为结构等价重构，不改变 HTTP/SSE/API/schema/env/CLI/SQLite/runtime 语义。
- Note: `src/tests/mod.rs` 与 `src/upstream_accounts/tests.rs` 只做浅层入口真模块化，不深拆大测试文件。
- Note: 验证门槛固定为 `cargo fmt`、`cargo check --locked --all-targets --all-features`、`cargo test` 与 `scripts/shared-testbox-api-read-smoke`。

## 结构收口结果

- `main.rs` 不再依赖 crate-root `include!()` 或仅为桥接旧入口存在的 re-export 模块。
- `proxy`、`forward_proxy`、`api/slices`、`upstream_accounts/routing` 恢复为真实 `mod.rs` 入口，剩余生产热点移除 `use crate::*;`。
- `src/tests/mod.rs` 改为真实测试入口，`src/tests/slices/mod.rs` 承接原测试切片；`src/upstream_accounts/tests.rs` 改为真实模块树并下沉到 `src/upstream_accounts/tests/parts.rs`。
- `rg 'include!\\(' src` 与 `rg 'use crate::\\*;' src` 已清零。

## 验证

- 已通过：`cargo fmt`
- 已通过：`cargo check --locked --all-targets --all-features`
- 已通过：`cargo test`
- 已通过：`scripts/shared-testbox-api-read-smoke`
