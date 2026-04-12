# 后端 archive 结构收敛（#g7n33）

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-12
- Last: 2026-04-12

## 背景 / 问题陈述

- `/Users/ivan/Projects/Ivan/codex-vibe-monitor/src/maintenance/archive.rs` 目前仍是 3k+ 行巨型实现文件，承担 archive retention、manifest rebuild、upstream activity backfill、hourly rollup materialization 等多类职责。
- 当前文件仍以单文件聚合方式暴露给 crate root，可读性、review 粒度和后续维护成本都偏高。
- 上一轮后端结构债已完成 `proxy/routing` 与 `api/router` 收敛，本轮继续处理 archive 这块最重的 maintenance 结构债。

## 目标 / 非目标

### Goals

- 把 `src/maintenance/archive.rs` 拆成真实子模块，并保持 crate 内部调用点兼容。
- 按职责收敛 archive 逻辑边界：cleanup/prune、manifest/backfill、archive batch writer、hourly rollup replay。
- 保持 retention/CLI/startup/stats/proxy 相关行为、数据库 schema 与文件布局兼容。
- 继续走 fast-flow，完成本地验证、shared-testbox 验证、PR、merge 与 cleanup。

### Non-goals

- 不重写 archive 算法或调整现有 retention 策略。
- 不改 HTTP/JSON/SSE、CLI 参数、env 语义或 SQLite schema。
- 不把 `AppState`、warning 清零、`proxy/dispatch.rs`、`prompt_cache_conversations.rs` 混入本轮。

## 范围（Scope）

### In scope

- `src/maintenance/archive.rs` 模块化拆分。
- 与 archive 拆分直接相关的最小可见性/re-export 调整。
- 与 archive 行为直接相关的现有测试随迁修复。
- spec/README 同步、PR 验证与收尾。

### Out of scope

- archive 行为增强、schema migration、archive 文件格式升级。
- 无关 maintenance/router/proxy/api 逻辑重构。
- 测试体系重写或 warning 专项治理。

## 需求（Requirements）

### MUST

- `src/maintenance/archive.rs` 不再保留当前巨石实现，改为薄入口 + 子模块装配。
- 至少拆分为：cleanup/prune、manifest/backfill、archive writers、hourly rollup replay/support 四类职责模块。
- 既有 crate 内部调用点与测试入口保持兼容，不新增外部接口。
- 本地 `cargo fmt --all -- --check`、`cargo check --locked --all-targets --all-features`、`cargo test --locked --all-features` 通过。
- shared-testbox 至少验证 `scripts/shared-testbox-raw-smoke --cleanup`；若 archive 影响 read-side，再补 `scripts/shared-testbox-api-read-smoke --cleanup`。

### SHOULD

- 减少 archive 相关跨职责 helper 的散落引用，优先让模块边界可直接读懂。
- 新模块尽量使用显式 `mod` / `pub(crate) use`，避免继续扩大 `include!()` 面。

### COULD

- 顺手清理与 archive 拆分直接相关的小范围 import/可见性噪音。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- retention/CLI/startup 调用 archive 相关能力时，入口函数名与行为保持兼容。
- archive batch 写入、manifest refresh、upstream activity backfill、hourly rollup replay 的执行顺序与结果保持不变。
- missing archive / stale temp / dry-run 行为与现有日志语义保持兼容。

### Edge cases / errors

- 缺失 archive 文件时仍按当前逻辑跳过并记录 warning，不因模块拆分改变失败策略。
- dry-run 路径仍只返回 summary，不做写入或删除。
- hourly rollup materialization 仍保持对已 materialized / missing archive / pending manifest 的既有保护逻辑。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- | --- | --- |
| archive maintenance internal API | Rust module API | internal | Modify | None | backend | retention/startup/stats/proxy/tests | 仅模块边界与 re-export 调整 |

### 契约文档（按 Kind 拆分）

None

## 验收标准（Acceptance Criteria）

- Given archive maintenance 相关 crate 内部调用点
  When 本轮结构收敛完成
  Then `src/maintenance/archive.rs` 仅保留薄入口与模块装配，主要实现落在拆分后的子模块中。

- Given 现有 retention/startup/CLI/archive 测试与 smoke
  When 运行本地与 shared-testbox 验证
  Then 行为与基线兼容，所有门禁通过。

- Given fast-flow merge+cleanup 终点
  When PR 收敛完成
  Then latest PR 已 merged，分支/worktree 已 cleanup，本地回到最新 `main`。

## 实现前置条件（Definition of Ready / Preconditions）

- archive 本轮目标与非目标已冻结。
- 验收标准覆盖本地验证、shared-testbox 验证与 merge+cleanup 终点。
- 本轮仅改 archive 结构边界，不扩展到其它债务面。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: 现有 archive / hourly rollup / retention 相关 Rust tests 随迁通过。
- Integration tests: `cargo test --locked --all-features`。
- E2E tests (if applicable): None。

### Quality checks

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo test --locked --all-features`
- `scripts/shared-testbox-raw-smoke --cleanup`
- `scripts/shared-testbox-api-read-smoke --cleanup`（若本轮 read-side 受影响则必跑）

## 文档更新（Docs to Update）

- `docs/specs/g7n33-backend-archive-structure-convergence/SPEC.md`: 跟踪实现与收尾状态。
- `docs/specs/README.md`: 新增条目并在合并前后同步状态。

## 计划资产（Plan assets）

- Directory: `docs/specs/g7n33-backend-archive-structure-convergence/assets/`
- In-plan references: `![...](./assets/<file>.png)`
- Visual evidence source: maintain `## Visual Evidence` in this spec when owner-facing or PR-facing screenshots are needed.

## Visual Evidence

本轮为后端结构债，不需要视觉证据。

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 建立 archive follow-up spec、冻结范围并从最新 `main` 起分支
- [x] M2: 完成 `archive.rs` -> 薄入口 + 子模块拆分，并保持 crate 内部调用兼容
- [x] M3: 完成本地 Rust 门禁与 shared-testbox archive 相关 smoke 验证
- [ ] M4: 完成 fast-flow PR、fresh review proof、merge 与 cleanup

## 方案概述（Approach, high-level）

- 以 `archive.rs` 为唯一主战场，使用子模块把职责边界按 archive lifecycle 拆开。
- 保留现有入口函数名，优先通过 `pub(crate) use` 维持 crate 内部兼容。
- 先完成物理拆分，再用 cargo/test/smoke 证明行为未回归。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：archive 与 retention/hourly rollup/stats/proxy 交叉较多，拆分时容易漏掉可见性边界。
- 需要决策的问题：None。
- 假设（需主人确认）：本轮继续按 fast-flow 的 merge+cleanup 终点推进。

## 变更记录（Change log）

- 2026-04-12: 创建 archive 结构收敛 follow-up spec，冻结 fast-flow / merge+cleanup / archive-only 范围。
- 2026-04-12: 完成 `archive.rs` 薄入口与子模块拆分；本地 `cargo fmt/check/test`、shared-testbox `raw-smoke` 与 `api-read-smoke` 全绿。

## 参考（References）

- `docs/specs/phb37-backend-structure-convergence-followup/SPEC.md`
- `docs/specs/9vau7-backend-structure-dual-pr-followup/SPEC.md`
