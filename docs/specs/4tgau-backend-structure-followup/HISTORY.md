# 后端结构残余收口与浅层测试模块化 - History

## Migration

- Canonical spec: `docs/specs/4tgau-backend-structure-followup/SPEC.md`

## Key Decisions

- 2026-07-08: 将剩余后端结构债合并为一个 follow-up PR 处理，范围同时包含生产模块边界与浅层测试入口模块化。
- 2026-07-08: crate-root `include!()` / bridge re-export 与 `src/` 中剩余 `include!()` / `use crate::*;` 同轮收口，避免再拆出第二个同主题 PR。
- 2026-07-08: 大测试文件保持原有主体布局，仅把入口聚合器改成真实 `mod` 树；更深的测试切片治理留给后续独立主题。
- 2026-07-08: `proxy`、`forward_proxy`、`api/slices`、`upstream_accounts/routing` 最终统一收口为真实 `mod.rs` 入口，不保留旧 root bridge。
- 2026-07-08: 完整验证以 `cargo fmt`、`cargo check --locked --all-targets --all-features`、`cargo test` 与 `scripts/shared-testbox-api-read-smoke` 为准，结果全部通过。
