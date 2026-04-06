# `main.rs` 结构化拆分与基线同步重构（#krsd4）

## 状态

- Status: 已完成
- Created: 2026-04-06
- Last: 2026-04-06

## 背景 / 问题陈述

- [src/main.rs](/Users/ivan/.codex/worktrees/5311/codex-vibe-monitor/src/main.rs) 已超过 24k 行，同时承载入口装配、配置解析、运行时生命周期、maintenance 任务、archive/rollup 逻辑与 share-link 解析。
- 其他模块通过 `use super::*` 透传依赖 `main.rs` 中的共享类型与常量，导致边界不清、可见性失控，任何局部改动都容易形成大面积连锁影响。
- 如果继续在现状上叠加需求，review、测试定位与后续模块化迁移成本都会持续放大。

## 目标 / 非目标

### Goals

- 把 `main.rs` 收缩为薄入口与装配层，只保留模块声明、最小导入和 `#[tokio::main]` handoff。
- 为共享配置/状态/生命周期/maintenance/share-link 逻辑建立稳定的模块归属和显式 `crate::...` 导入。
- 在不改变 HTTP、CLI、schema、环境变量语义和运行时行为的前提下完成结构重组。
- 为后续继续拆分 `upstream_accounts`、`tests` 或局部 maintenance 逻辑提供更稳的边界。

### Non-goals

- 不修改接口行为、数据库 schema、环境变量名或默认值。
- 不顺手处理与本次结构重构无关的 warning、测试债务或性能优化。
- 不要求同步拆小 [src/upstream_accounts/mod.rs](/Users/ivan/.codex/worktrees/5311/codex-vibe-monitor/src/upstream_accounts/mod.rs) 或 [src/tests/mod.rs](/Users/ivan/.codex/worktrees/5311/codex-vibe-monitor/src/tests/mod.rs)。

## 范围（Scope）

### In scope

- 先按 `$update-baseline` 在 `th/refactor-main-rs-structure` 上完成 `origin/main` rebase-only 基线同步。
- 新增并落地以下模块树：
  - `src/config.rs`
  - `src/app_state.rs`
  - `src/runtime.rs`
  - `src/share_links.rs`
  - `src/maintenance/mod.rs`
  - `src/maintenance/cli.rs`
  - `src/maintenance/startup_prep.rs`
  - `src/maintenance/startup_backfill.rs`
  - `src/maintenance/retention.rs`
  - `src/maintenance/archive.rs`
  - `src/maintenance/hourly_rollups.rs`
- 把共享接口稳定为：
  - `crate::config::{CliArgs, AppConfig, CrsStatsConfig, UpstreamAccountsMoeMailConfig, ForwardProxyAlgo, RawCompressionCodec, ArchiveBatchLayout, ArchiveSegmentGranularity, ArchiveFileCodec}`
  - `crate::app_state::{AppState, HttpClients}`
  - `crate::runtime::{run, init_tracing, run_runtime_until_shutdown}`
  - `crate::share_links::{parse_vmess_share_link, parse_shadowsocks_share_link, canonical_*}`
  - `crate::maintenance::*`
- 更新 `api`、`forward_proxy`、`stats`、`upstream_accounts` 与测试代码对共享类型的导入方式，去除对 `main.rs` 的 reach-through 依赖。

### Out of scope

- HTTP 路由语义和 handler 行为变更。
- CLI flag/subcommand 口径变化。
- schema/迁移/数据库数据格式变化。
- 远端发布、merge 后 cleanup。

## 需求（Requirements）

### MUST

- 结构迁移后 `cargo check` 和完整 `cargo test` 通过。
- 不新增高于当前基线的新 warning。
- 所有共享类型必须从明确模块导出，不再依赖 `use super::*` 把 `main.rs` 当作隐式 prelude。
- maintenance / archive / startup-backfill / share-link 的函数名、调用顺序和行为语义保持兼容。

### SHOULD

- 常量跟着所属领域迁移，避免新建泛化 `constants.rs` 或 `utils.rs` 杂物模块。
- 模块之间使用最小可见性（优先 `pub(crate)`），避免扩大 API 面。
- 提交与 PR 描述清楚说明“结构重组，无行为改动”。

### COULD

- 在不改变行为的前提下，顺手改善少量导入组织与局部文件注释可读性。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 启动流仍由 `CliArgs::parse()` + `AppConfig::from_sources()` 驱动；入口只负责把解析后的配置和 CLI 交给 `runtime::run(...)`。
- maintenance CLI、startup prep、startup backfill、retention、archive、hourly rollup 逻辑移动到 `maintenance/` 下后，行为、调用顺序和日志口径保持一致。
- share-link 解析与 canonicalization 从 `main.rs` 迁出到 `share_links.rs`，但解析结果与调用点行为不变。
- `api`、`forward_proxy`、`stats`、`upstream_accounts`、测试模块从显式模块导入共享类型/函数，不再依赖 `super::*` 间接拿到 `main.rs` 内容。

### Edge cases / errors

- 若基线同步出现 rebase 冲突，必须先解决冲突再继续任何代码迁移。
- 若拆分过程中暴露真实行为缺陷，不在本 spec 中顺手修，另开后续 spec/PR。
- 若完整 `cargo test` 暴露与本次改动无关的历史失败，需要在最终 PR 中明确区分“已有问题”与“本次回归”。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Rust shared module exports | internal | internal | Modify | None | backend | `api` / `forward_proxy` / `stats` / `upstream_accounts` / `tests` | 仅调整模块归属与导入路径，不改行为 |

### 契约文档（按 Kind 拆分）

None

## 验收标准（Acceptance Criteria）

- Given 当前分支已同步到 `origin/main`
  When 本次结构重构完成
  Then `main.rs` 只保留入口/装配职责，不再内嵌配置定义、startup/retention/archive/backfill 实现或 share-link 解析实现。

- Given `api`、`forward_proxy`、`stats`、`upstream_accounts` 与测试代码依赖共享类型
  When 它们完成导入更新
  Then 这些依赖通过显式 `crate::config` / `crate::app_state` / `crate::runtime` / `crate::share_links` / `crate::maintenance` 导入解析，不再依赖 `use super::*` reach-through `main.rs`。

- Given 结构迁移完成
  When 运行 `cargo check` 与 `cargo test`
  Then 结果通过，且没有引入新的 warning 类别。

## 实现前置条件（Definition of Ready / Preconditions）

- 目标模块树与非目标已冻结
- 共享接口归属已确认
- 验证门槛固定为 `cargo check` + 完整 `cargo test`
- 当前分支已完成基线同步并保留 rebase 前备份引用

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: 现有 Rust 单测与模块内测试需保持通过。
- Integration tests: 继续依赖现有 `cargo test` 覆盖 archive / retention / proxy / share-link / startup flows。
- E2E tests (if applicable): None

### UI / Storybook (if applicable)

- None

### Quality checks

- `cargo check`
- `cargo test`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 追加本 spec 索引并记录状态/日期/说明

## 计划资产（Plan assets）

- Directory: `docs/specs/krsd4-main-rs-structure-refactor/assets/`
- In-plan references: `![...](./assets/<file>.png)`
- Visual evidence source: maintain `## Visual Evidence` in this spec when owner-facing or PR-facing screenshots are needed.
- If an asset must be used in impl (runtime/test/official docs), list it in `资产晋升（Asset promotion）` and promote it to a stable project path during implementation.

## Visual Evidence

无。本次为后端结构重构，不涉及主人可见 UI 变化。

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 在 `th/refactor-main-rs-structure` 上完成 `origin/main` 基线同步并保留 backup ref
- [x] M2: 提取 `config.rs`、`share_links.rs`、`app_state.rs` 并更新所有共享依赖导入
- [x] M3: 提取 `runtime.rs` 与 `maintenance/` 模块群，收缩 `main.rs`
- [x] M4: 通过 `cargo check` 与完整 `cargo test`
- [x] M5: 完成 review-loop、自审收敛，并准备/更新 PR 说明

## 方案概述（Approach, high-level）

- 先稳住基线，再按低风险共享模块优先的顺序迁移：`config` / `share_links` / `app_state` -> `runtime` -> `maintenance/*`。
- 迁移时优先保持符号名与函数签名稳定，先改变归属，再处理导入，不同时改行为。
- 通过多轮编译/测试缩小破坏面，确保每一组抽离都可独立验证。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：`main.rs` 现有共享常量和 helper 被多个子模块隐式依赖，移动时容易遗漏导入或可见性。
- 风险：`tests/mod.rs` 与 `upstream_accounts/mod.rs` 本身也很大，导入修复可能比较分散。
- 需要决策的问题：None
- 假设（需主人确认）：None

## 变更记录（Change log）

- 2026-04-06: 新建 spec，冻结 `main.rs` 结构化拆分范围、模块树与验收口径。
- 2026-04-06: 完成模块拆分、显式导入迁移、文档索引同步与本地验证收口；`main.rs` 收缩为薄入口。

## 参考（References）

- `$update-baseline`
- `$fast-flow`
- `$codex-review-loop`
