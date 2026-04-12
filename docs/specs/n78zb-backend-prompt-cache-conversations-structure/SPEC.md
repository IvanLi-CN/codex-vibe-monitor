# 后端 prompt-cache conversations 结构收敛（#n78zb）

## 状态

- Status: 已完成
- Created: 2026-04-12
- Last: 2026-04-12

## 背景 / 问题陈述

- `/Users/ivan/Projects/Ivan/codex-vibe-monitor/src/api/slices/prompt_cache_and_timeseries/prompt_cache_conversations.rs` 目前仍是 2k+ 行巨型实现文件，混合了请求解析、缓存并发控制、分页/游标、hydrate 组装、聚合查询、事件预览与上游账号汇总等多类职责。
- 当前 prompt-cache conversations 读侧已经是 Dashboard、stats read API 与 shared-testbox smoke 的核心路径，单文件继续膨胀会让 review 粒度、可见性边界和后续 follow-up 成本持续升高。
- 上一轮已完成 archive 模块化，本轮继续处理后端剩余最高价值的读侧巨石文件之一。

## 目标 / 非目标

### Goals

- 把 `src/api/slices/prompt_cache_and_timeseries/prompt_cache_conversations.rs` 拆成真实子模块，并保持 crate 内部调用点兼容。
- 按职责收敛为：request/cursor、cache gate、hydrate/adapter、response builders、aggregate queries、detail queries 六类模块。
- 保持 `/api/stats/prompt-cache-conversations`、summary/timeseries 关联读侧行为、JSON 字段与 SQLite 访问语义兼容。
- 继续走 fast-flow，完成本地验证、shared-testbox 验证、PR、merge 与 cleanup。

### Non-goals

- 不改 prompt-cache conversations 的公开 JSON 契约、分页语义、cursor 编码含义或 `snapshotAt` 规则。
- 不把 `AppState` 重组、warning 清零、`proxy/dispatch.rs`、`maintenance/hourly_rollups.rs` 混入本轮。
- 不重写 prompt-cache hourly rollup 或新增 API 端点。

## 范围（Scope）

### In scope

- `src/api/slices/prompt_cache_and_timeseries/prompt_cache_conversations.rs` -> 真实子模块拆分。
- 与拆分直接相关的最小 `mod` / `pub(crate) use` / 可见性调整。
- 与 prompt-cache conversations 读侧直接相关的现有测试随迁修复。
- spec/README 同步、PR 验证与收尾。

### Out of scope

- 新增接口、schema migration、cursor 协议升级。
- 无关 stats/proxy/archive/runtime 逻辑改造。
- warning 专项治理或测试体系重写。

## 需求（Requirements）

### MUST

- `prompt_cache_conversations` 不再保留单文件巨石实现，改为薄入口 + 真正的子模块装配。
- 至少拆分为：request/cursor、cache gate、hydrate/adapter、response builders、aggregate queries、detail queries。
- 既有 crate 内部调用点与测试入口保持兼容，不新增外部接口。
- 本地 `cargo fmt --all -- --check`、`cargo check --locked --all-targets --all-features`、`cargo test --locked --all-features` 通过。
- shared-testbox 至少验证 `scripts/shared-testbox-api-read-smoke --cleanup`。

### SHOULD

- prompt-cache conversations 共享 helper 尽量收敛到读起来直接能辨认的职责边界。
- 使用显式 `mod` / `pub(crate) use`，不扩大新的 `include!()` 面。

### COULD

- 顺手清理拆分直接带出的 import/可见性噪音，但不改行为。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- `/api/stats/prompt-cache-conversations` 的 count/activityHours/activityMinutes、compact/full、cursor/snapshotAt 行为保持兼容。
- cached conversations、paginated working conversations、events、recent invocations、upstream account summaries 的组装结果保持兼容。
- prompt-cache conversations 与 summary/timeseries 共用的 hourly-rollup / source-scope 判定行为不变。

### Edge cases / errors

- 非法 `detail`、`pageSize`、`cursor`、`snapshotAt` 仍保持现有 bad request 语义。
- 仅 `activityMinutes` working conversations 支持分页/游标的限制不变。
- snapshot 过滤、row-id ceiling 与 cursor 恢复逻辑保持兼容。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- | --- | --- |
| prompt cache conversations internal API | Rust module API | internal | Modify | None | backend | stats/dashboard/tests/shared-testbox smoke | 仅模块边界与 re-export 调整 |

### 契约文档（按 Kind 拆分）

None

## 验收标准（Acceptance Criteria）

- Given prompt-cache conversations 相关 crate 内部调用点
  When 本轮结构收敛完成
  Then `prompt_cache_conversations` 仅保留薄入口与模块装配，主要实现落在拆分后的子模块中。

- Given 现有 Rust tests 与 api-read smoke
  When 运行本地与 shared-testbox 验证
  Then `/api/stats/prompt-cache-conversations` 与关联 read-side 行为保持兼容，所有门禁通过。

- Given fast-flow merge+cleanup 终点
  When PR 收敛完成
  Then latest PR 已 merged，分支/worktree 已 cleanup，本地回到最新 `main`。

## 实现前置条件（Definition of Ready / Preconditions）

- prompt-cache conversations 本轮目标与非目标已冻结。
- 验收标准覆盖本地验证、shared-testbox 验证与 merge+cleanup 终点。
- 本轮仅改 prompt-cache conversations 结构边界，不扩展到其它债务面。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: 现有 prompt-cache conversations / stats 相关 Rust tests 随迁通过。
- Integration tests: `cargo test --locked --all-features`。
- E2E tests (if applicable): None。

### Quality checks

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo test --locked --all-features`
- `scripts/shared-testbox-api-read-smoke --cleanup`

## 文档更新（Docs to Update）

- `docs/specs/n78zb-backend-prompt-cache-conversations-structure/SPEC.md`: 跟踪实现与收尾状态。
- `docs/specs/README.md`: 新增条目并在合并前后同步状态。

## 计划资产（Plan assets）

- Directory: `docs/specs/n78zb-backend-prompt-cache-conversations-structure/assets/`
- In-plan references: `![...](./assets/<file>.png)`
- Visual evidence source: maintain `## Visual Evidence` in this spec when owner-facing or PR-facing screenshots are needed.

## Visual Evidence

本轮为后端结构债，不需要视觉证据。

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 建立 prompt-cache conversations follow-up spec、冻结范围并从最新 `main` 起分支
- [x] M2: 完成 `prompt_cache_conversations` -> 薄入口 + 子模块拆分，并保持 crate 内部调用兼容
- [x] M3: 完成本地 Rust 门禁与 shared-testbox api-read smoke 验证
- [x] M4: 完成 fast-flow PR、fresh review proof、merge 与 cleanup

## 方案概述（Approach, high-level）

- 以 `prompt_cache_conversations` 为唯一主战场，先按职责切出六类子模块，再用 `pub(crate) use` 维持 crate 内兼容。
- 保留现有入口函数名与查询行为，优先做物理拆分，不顺手改契约。
- 先完成模块化，再用 cargo/test/smoke 证明行为未回归。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：cursor/snapshot/filter 与 hydrate/detail queries 交叉较多，拆分时容易漏掉共享 helper。
- 需要决策的问题：None。
- 假设（需主人确认）：本轮继续按 fast-flow 的 merge+cleanup 终点推进。

## 变更记录（Change log）

- 2026-04-12: 创建 prompt-cache conversations 结构收敛 follow-up spec，冻结 fast-flow / merge+cleanup / prompt-cache-only 范围。
- 2026-04-12: 完成 `prompt_cache_conversations` 真模块拆分；本地 `cargo fmt/check/test` 通过，期间命中过一次既有代理热路径时间敏感单测 `proxy_openai_v1_chunked_json_without_header_sticky_uses_live_first_attempt`，单测复跑与整套复跑均通过；shared-testbox `api-read-smoke` 全绿。
- 2026-04-12: PR #339 review proof clear，GitHub checks 全绿，merge + cleanup 完成，本地回到最新 `main`。

## 参考（References）

- `docs/specs/9vau7-backend-structure-dual-pr-followup/SPEC.md`
- `docs/specs/g7n33-backend-archive-structure-convergence/SPEC.md`
