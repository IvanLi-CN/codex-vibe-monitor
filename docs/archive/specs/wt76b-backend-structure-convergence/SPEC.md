# 后端优先源码结构收敛（#wt76b）

## 状态

- Status: 已完成

## 背景 / 问题陈述

- `/Users/ivan/Projects/Ivan/codex-vibe-monitor/src/main.rs` 当前约 `26396` 行，已经同时承载启动装配、HTTP 路由、forward proxy/Xray、统计查询、SSE、settings、价格目录与大量白盒测试，导航与回归成本过高。
- 当前文件内含约 `209` 个测试，`#[cfg(test)] mod tests` 约 `10716` 行，并且测试块后仍有生产代码继续追加，导致定位生产逻辑与测试夹层非常困难。
- 前端也存在超长文件，但本轮交付明确只做后端优先收敛，前端拆分仅登记下一阶段优先级，不在本轮改动范围内。

## 目标 / 非目标

### Goals

- 将 `src/main.rs` 收窄为启动装配、router 注册与少量 glue，不再内联测试、forward proxy 子系统实现与统计读侧实现。
- 把白盒测试迁入 `src/tests/`，继续保留对 crate 内私有实现的访问能力，不改成 integration tests。
- 把 forward proxy / Xray / subscription / validation 相关实现迁入 `src/forward_proxy/`。
- 把 invocations/stats/timeseries/errors/failures/perf/quota/SSE 的读侧处理迁入 `src/api/`，并把共享的统计/时间 helper 迁入 `src/stats/`。
- 保持 HTTP 路由、JSON 字段、SSE payload、SQLite schema、环境变量与 `occurred_at` 的 Asia/Shanghai naive-string 语义完全不变。

### Non-goals

- 不调整任何前端页面或 hook 的结构。
- 不修改数据库 schema、归档策略、计费算法或业务语义。
- 不把白盒测试改造为独立集成测试工程。
- 不顺手处理与本次结构收敛无关的功能增强。

## 范围（Scope）

### In scope

- `src/main.rs` 的测试拆分、模块声明与装配回收。
- `src/tests/`、`src/forward_proxy/`、`src/api/`、`src/stats/` 的新增模块文件。
- 仅为模块化所需的最小 `pub(super)` / `use` 调整。
- 对应 spec 索引、实施记录与后续前端优先级备注。

### Out of scope

- `/Users/ivan/Projects/Ivan/codex-vibe-monitor/web/src/pages/Settings.tsx`
- `/Users/ivan/Projects/Ivan/codex-vibe-monitor/web/src/hooks/useTimeseries.ts`
- `/Users/ivan/Projects/Ivan/codex-vibe-monitor/web/src/lib/api.ts`

## 模块边界与落位

### 后端模块收口

- `src/tests/`：承载原 `#[cfg(test)] mod tests` 内容，按现有白盒访问方式继续运行。
- `src/forward_proxy/`：承载 forward proxy settings/runtime 读取、live stats response builder、subscription refresh、candidate validation、bootstrap probe、manager、Xray supervisor 与相关 response/type。
- `src/api/`：承载 invocations/stats/summary/timeseries/errors/failures/perf/quota/events/prompt-cache 的读侧 handler 与响应类型。
- `src/stats/`：承载范围解析、时区转换、ISO serializer、统计窗口/过滤辅助等共享 helper。

### 后续前端优先级（本轮不实现）

- P1: `web/src/pages/Settings.tsx`
- P2: `web/src/hooks/useTimeseries.ts`
- P3: `web/src/lib/api.ts`

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                                                   | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）                |
| -------------------------------------------------------------- | ------------ | ------------- | -------------- | ------------------------ | --------------- | ------------------- | ---------------------------- |
| HTTP routes under `/health`, `/api/**`, `/events`, `/v1/*path` | http         | external      | Modify         | None                     | backend         | web SPA / clients   | 仅模块内搬迁，实现行为不变   |
| Internal crate modules                                         | rust-module  | internal      | New            | None                     | backend         | backend crate       | 仅内部重构，不新增运行时依赖 |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- `src/main.rs` 行数降到 `10000` 以下，且不再包含内联测试、forward proxy 子系统实现体、统计读侧 handler 实现体。
- 现有路由路径、query 参数、JSON 字段、SSE payload 类型与 `cargo test` 行为保持兼容。
- `cargo fmt`、`cargo check`、`cargo test` 全部通过。
- 新增或迁移后的模块文件名能够按职责快速定位：测试、forward proxy、api 读侧、stats helper 不再混在同一文件。
- PR 说明中明确记录前端后续优先级，但本轮实现中这些文件 `不存在` 结构性修改。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: 现有 Rust 白盒测试继续通过。
- Integration tests: None。
- E2E tests: None。

### Quality checks

- `cargo fmt`
- `cargo check`
- `cargo check --tests`
- `cargo clippy -- -D warnings`
- `cargo test`
- `cd web && npm run lint -- --max-warnings=0`
- `cd web && npx tsc -b`
- `codex --sandbox read-only -a never review --base origin/main`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增 spec 索引并在完成后更新状态/PR 记录。
- `docs/specs/wt76b-backend-structure-convergence/SPEC.md`: 记录实施进度、PR 与收敛状态。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 建立 spec、登记基线与前端后续优先级。
- [x] M2: 抽离 `src/tests/` 并保持白盒测试可运行。
- [x] M3: 抽离 `src/forward_proxy/` 与 `src/api/` / `src/stats/`，回收 `src/main.rs`。
- [x] M4: 完成格式化、验证、PR 与 review-loop 收敛。

## 方案概述（Approach, high-level）

- 采用“子模块承接实现 + crate root 只保留装配”的最小重构路径，优先保持现有函数签名与测试覆盖，不做行为改写。
- 利用 crate 子模块对父模块私有项的可见性，减少无意义的 `pub(crate)` 扩散，仅在主入口重新绑定必要导出。
- 先拆测试，再拆 forward proxy，再拆 API/stat helpers，确保每一步都可用 `cargo test` 及时回归。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：模块迁移后若遗漏 `use` 或可见性，会先表现为编译错误；必须以 `cargo check/test` 收敛。
- 风险：部分白盒测试若直接依赖 sibling module 私有项，需要通过 crate root 重新绑定保持可访问。
- 需要决策的问题：None。
- 假设（需主人确认）：None。

## 变更记录（Change log）

- 2026-03-09: 创建 spec，冻结首波“后端优先源码结构收敛”范围与验收口径。
- 2026-03-09: 完成 `src/tests/`、`src/forward_proxy/`、`src/api/`、`src/stats/` 首波拆分，`src/main.rs` 收窄到 9990 行。
- 2026-03-09: PR #104 已创建并打上 `type:skip` / `channel:stable`；本地验证、CI checks 与 `codex review` 均已收敛为通过/无阻塞。

## 参考（References）

- `/Users/ivan/Projects/Ivan/codex-vibe-monitor/src/main.rs`
- `/Users/ivan/Projects/Ivan/codex-vibe-monitor/docs/specs/README.md`
