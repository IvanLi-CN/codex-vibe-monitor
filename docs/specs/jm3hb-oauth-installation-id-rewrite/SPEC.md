# OAuth 上游 `x-codex-installation-id` 代理侧稳定改写（#jm3hb）

## 状态

- Status: 已完成
- Created: 2026-04-11
- Last: 2026-04-11

## 背景 / 问题陈述

- 当前 OAuth `/v1/responses` 转发会保留下游请求体里的 `client_metadata.x-codex-installation-id`。
- 这个值来自下游终端安装实例，不属于代理自身身份，继续透传会让上游看到混杂的终端 installation id。
- 代理当前也没有自己的 deployment 级稳定 installation id 机制，导致无法把“代理实例 + 上游账号”稳定映射为可复现的上送身份。

## 目标 / 非目标

### Goals

- 代理不再透传下游真实 `client_metadata.x-codex-installation-id`。
- 为每个部署实例持久化一个稳定 seed，并基于 `internal account_id` 派生上游 installation id。
- 相同部署实例 + 相同上游账号必须稳定，相同部署实例 + 不同上游账号必须不同。
- 若当前请求没有可用的内部 `account_id`，则从上游 body 中移除该键，避免回传下游原值。
- 保持现有 `/v1/responses` rewrite 语义不回归。

### Non-goals

- 不改 `x-codex-installation-id` header 转发策略。
- 不改非 OAuth 路由、`/v1/chat/completions`、`/v1/responses/compact` 的 body 规范化。
- 不新增对外 API、UI 配置项或运维面板设置项。

## 范围（Scope）

### In scope

- OAuth bridge 的 deployment seed 持久化与读取。
- `/v1/responses` body 中 `client_metadata.x-codex-installation-id` 的覆盖/移除。
- 基于 `account_id` 的确定性 installation id 派生 helper。
- Rust 单元/集成测试与相关 spec/README 索引同步。

### Out of scope

- 其它 metadata 字段的脱敏策略。
- 上游账号 display name / ChatGPT account id 参与派生。
- 任何 UI、Storybook 或视觉证据变更。

## 需求（Requirements）

### MUST

- 持久化一个 deployment 级 seed，数据库克隆前后行为可复现。
- 派生算法对同一 seed + 同一 `account_id` 输出稳定，对不同 `account_id` 输出不同。
- 上游 body 中若存在 `client_metadata.x-codex-installation-id`，必须被代理派生值覆盖。
- 上游 body 中若不存在可用 `account_id`，必须删除该键，不得保留下游原值。
- 若 `client_metadata` 已存在，只允许改动 `x-codex-installation-id` 一个键。
- 不得把 seed 或派生前原文写入日志、指标或调试输出。

### SHOULD

- deployment seed 应只初始化一次，并在运行时缓存，避免每次请求重复读库。
- 派生值格式应为稳定的小写 UUID-like 字符串，便于观测与排查。

### COULD

- 为后续扩展其它 OAuth 请求体 metadata 归一化预留内部 helper 结构。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 当 OAuth `/v1/responses` 请求带 `account_id` 且 body 中存在 `client_metadata.x-codex-installation-id` 时，代理使用 deployment seed + `account_id` 派生新值并覆盖后转发上游。
- 当 OAuth `/v1/responses` 请求带 `account_id` 但 body 中不存在 `client_metadata` 时，代理不新增 `client_metadata` 容器，只保留现有 rewrite 逻辑。
- 当 OAuth `/v1/responses` 请求没有 `account_id` 时，若 body 中存在该 installation id，则删除该键后再转发。
- deployment seed 首次读取缺失时，代理在本地 SQLite 事务中生成并持久化，再缓存到运行时供后续请求复用。

### Edge cases / errors

- seed 表已存在且有值时，不得重复生成或覆盖。
- `client_metadata` 不是 object 时，按最小兼容策略忽略 installation id 改写，不额外造结构，也不破坏其它字段。
- body 非 JSON object 仍沿用现有错误返回。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- | --- | --- |
| OAuth `/v1/responses` rewrite | internal | internal | Modify | None | backend | oauth bridge | 仅改 body 里的 installation id 归一化 |
| deployment installation seed storage | internal | internal | New | None | backend | oauth bridge runtime | SQLite 单行持久化 |

### 契约文档（按 Kind 拆分）

None

## 验收标准（Acceptance Criteria）

- Given 同一 deployment seed 与同一 `account_id`
  When 多次转发 OAuth `/v1/responses`
  Then 上游看到的 `client_metadata.x-codex-installation-id` 完全一致。

- Given 同一 deployment seed 与两个不同 `account_id`
  When 分别转发 OAuth `/v1/responses`
  Then 上游看到的 installation id 不同。

- Given 下游 body 中带有真实 `client_metadata.x-codex-installation-id`
  When 代理转发且有可用 `account_id`
  Then 上游看到的是代理派生值而非下游原值。

- Given 下游 body 中带有真实 `client_metadata.x-codex-installation-id`
  When 代理转发但没有可用 `account_id`
  Then 上游 body 中不再包含该键。

- Given 现有 `/v1/responses` rewrite 逻辑
  When 引入 installation id 改写后
  Then `instructions`、`store=false`、`stream=true`、移除 `max_output_tokens` 的既有行为不回归。

## 实现前置条件（Definition of Ready / Preconditions）

- deployment seed 使用 SQLite 持久化已锁定
- 派生输入键锁定为内部 `account_id`
- 非 OAuth 路由与 header 策略明确不在本次范围内
- 验收标准已覆盖稳定性、覆盖、strip 与回归场景

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: OAuth body rewrite / seed helper / installation id 派生稳定性
- Integration tests: mock upstream 验证转发后的 body 观测值
- E2E tests (if applicable): None

### UI / Storybook (if applicable)

- 不适用

### Quality checks

- `cargo fmt`
- `cargo test`（至少覆盖 oauth bridge 相关测试与新增场景）

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增索引并在收尾时同步状态

## 计划资产（Plan assets）

- Directory: `docs/specs/jm3hb-oauth-installation-id-rewrite/assets/`
- In-plan references: `![...](./assets/<file>.png)`
- Visual evidence source: maintain `## Visual Evidence` in this spec when owner-facing or PR-facing screenshots are needed.

## Visual Evidence

- 不适用（后端行为改动，无 UI 交付面）

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 落地 deployment seed 持久化与运行时读取缓存
- [x] M2: 完成 OAuth `/v1/responses` installation id 改写与派生 helper
- [x] M3: 补齐测试并通过相关后端验证

## 方案概述（Approach, high-level）

- 在现有 OAuth bridge 路径中扩展最小内部状态：由 SQLite 持久化一个 deployment seed，并在 AppState/runtime 中缓存。
- 用 seed + `account_id` 的 HMAC-SHA256 前 16 bytes 派生 UUID-like 值，确保稳定、不可逆、与 display name 解耦。
- 仅在现有 `/v1/responses` JSON rewrite 处触碰 installation id 字段，不额外扩大 metadata 改写面。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：数据库克隆会继承同一 deployment seed，克隆环境会共享代理身份。
- 需要决策的问题：None
- 假设（需主人确认）：当前 OAuth 上游发送路径始终可提供内部 `account_id` 给 rewrite 逻辑。

## 变更记录（Change log）

- 2026-04-11: 初始化规格，冻结 deployment seed + `account_id` 派生方案。
- 2026-04-11: 完成 OAuth `/v1/responses` installation id 稳定改写、SQLite seed 持久化与回归测试。

## 参考（References）

- `/Users/ivan/.codex/sessions/2026/04/11/rollout-2026-04-11T11-28-58-019d7a95-eadc-79e3-923d-97d0ad6132be.jsonl`
- `/Users/ivan/.codex/worktrees/adbd/codex-vibe-monitor/src/oauth_bridge.rs`
