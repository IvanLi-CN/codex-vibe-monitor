# 巨型源码结构收敛（#g8mfs）

## 状态

- Status: 已完成
- Created: 2026-04-09
- Last: 2026-04-09

## 背景 / 问题陈述

- 当前仓库仍存在多处超大源码文件，后端以 `/Users/ivan/.codex/worktrees/b368/codex-vibe-monitor/src/upstream_accounts/mod.rs`、`/Users/ivan/.codex/worktrees/b368/codex-vibe-monitor/src/proxy.rs`、`/Users/ivan/.codex/worktrees/b368/codex-vibe-monitor/src/tests/mod.rs` 为代表，前端以 `/Users/ivan/.codex/worktrees/b368/codex-vibe-monitor/web/src/lib/api.ts`、`/Users/ivan/.codex/worktrees/b368/codex-vibe-monitor/web/src/pages/account-pool/UpstreamAccountCreate.tsx`、`/Users/ivan/.codex/worktrees/b368/codex-vibe-monitor/web/src/pages/account-pool/UpstreamAccounts.tsx` 为代表。
- 这些文件同时承载多块职责，导致导航、review、定向测试和后续增量改动的成本持续升高。
- 仓库此前已完成 `main.rs` 等后端入口收敛，但剩余巨型文件仍集中在账号域、代理栈、前端 account-pool 页面和白盒测试聚合器，已成为当前开发阻力。

## 目标 / 非目标

### Goals

- 把本轮已识别的巨型文件拆到可 review 的职责粒度，并保持行为兼容。
- 保持 HTTP / JSON / SSE / SQLite schema / 环境变量 / Storybook 场景名称 / 页面默认导出不变。
- 为后续继续演进 account-pool、proxy 和测试域提供稳定文件边界。

### Non-goals

- 不新增 CI 行数门禁、ESLint 行数规则或 GitHub quality-gates 合同改动。
- 不主动扩展到当前未纳入范围的中型文件收敛。
- 不引入新的产品行为、数据库变更或接口协议变更。

## 范围（Scope）

### In scope

- 先在 `th/g8mfs-giant-source-structure-convergence` 上按 `$update-baseline` 完成 `origin/main` 的 rebase-only 基线同步。
- 后端：`src/upstream_accounts/mod.rs`、`src/proxy.rs`、`src/api/mod.rs`、`src/forward_proxy/mod.rs`、`src/tests/mod.rs` 及其新增切片文件。
- 前端：`web/src/lib/api.ts`、`web/src/pages/account-pool/UpstreamAccountCreate.tsx`、`web/src/pages/account-pool/UpstreamAccounts.tsx`、对应 page tests、`web/src/components/UpstreamAccountsPage.story-helpers.tsx` 及其新增切片文件。
- Spec/README 索引、PR 路径、review proof 与 merge+cleanup 收口。

### Out of scope

- `.github/**`、CI 规则、release 规则、quality-gates 契约。
- 无关页面与中型文件的顺手重构。
- 新增自动化 size guardrail。

## 功能与结构规格

- Rust 巨型模块优先使用稳定子模块或 `include!` 方式拆成薄入口 + 领域切片，保持现有公开 handler / helper / tests 语义不变。
- `web/src/lib/api` 改为顶层 barrel + 子模块，继续保留现有导出名，避免页面与 hooks 侧大面积改 import。
- `UpstreamAccountsPage` 与 `UpstreamAccountCreatePage` 继续保留默认导出；`SharedUpstreamAccountDetailDrawer` 可以迁入独立文件，但必须保持现有调用方兼容。
- 白盒测试与 Storybook helper 需要按领域拆薄，但保持现有测试入口和 stories 文件名稳定。

## 验收标准（Acceptance Criteria）

- 本轮目标文件全部完成物理拆分，原超大文件不再承担多领域职责。
- 本轮目标中的生产入口文件行数降到 `<=2500 LOC`，测试 / story / helper 文件降到 `<=3000 LOC`。
- 本地验证通过：
  - `cargo fmt --all -- --check`
  - `cargo check --locked --all-targets --all-features`
  - `cargo test`
  - `cd web && bun run lint`
  - `cd web && bun run test`
  - `cd web && bun run build`
  - `cd web && bun run build-storybook`
- 若改动影响 UI，可见视觉证据需先在对话中回传给主人，再推进 PR / merge 路径。

## 非功能性验收 / 质量门槛（Quality Gates）

### Quality checks

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo test`
- `cd web && bun run lint`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run build-storybook`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/g8mfs-giant-source-structure-convergence/SPEC.md`

## Visual Evidence

- 已基于既有 Storybook mock 场景完成本地视觉回归，并在对话中回传主人验收：
  - `account-pool-pages-upstream-accounts-list--operational`
  - `account-pool-pages-upstream-account-create-batch-oauth--ready`
- 本轮为结构收敛，无预期 UI 行为变更；截图资产未提交入仓，保留为本地 / 对话证据。

## 风险 / 假设

- 风险：超大文件间共享大量私有 helper，拆分时容易引入循环依赖或可见性回归。
- 风险：page tests 与 story helpers 对页面内部实现绑定较深，拆分后可能需要同步调整 fixtures / imports。
- 假设：本轮不新增 size guardrail，仅完成结构收敛与验证。
- 假设：快车道终点已锁定为 merge+cleanup。
