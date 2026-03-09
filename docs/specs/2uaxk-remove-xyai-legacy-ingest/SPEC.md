# 移除 XYAI 采集，保留历史读取（#2uaxk）

## 状态

- Status: 部分完成（3/4）

## 背景 / 问题陈述

- 项目当前同时承载了历史读取能力与 legacy XYAI 采集链路，但产品方向已经调整，不再需要从 XYAI 上游拉取新数据。
- 继续保留这条链路会让启动配置、运行时调度、文档说明与测试噪音都围绕一套已经弃用的外部依赖打转，增加维护成本与误配风险。
- 本次要把服务收敛为“保留历史可读、移除 XYAI 新写入”，同时不影响 proxy / CRS 的现有能力。

## 目标 / 非目标

### Goals

- 移除所有 XYAI 专属的配置解析、CLI 覆盖、legacy poll 调度与新写入逻辑。
- 保留历史 `source='xy'` 调用记录与 `codex_quota_snapshots` 的只读查询能力。
- 保持 CRS stats 轮询、OpenAI `/v1/*` proxy capture、forward proxy / xray 维护与对应 API/UI 不回归。
- 更新 README 与规格索引，明确服务已不再支持 XYAI 接入。

### Non-goals

- 不删除历史数据库表，也不清洗现有 `xy` 记录或 quota 快照。
- 不重命名仍然通用的 `XY_*` 配置键（如数据库、HTTP bind、retention）。
- 不调整 proxy、CRS、forward proxy 的功能边界或交互口径。

## 范围（Scope）

### In scope

- `src/main.rs` 中 XYAI-only CLI / env / AppConfig 字段移除。
- legacy XYAI fetch / persist / snapshot 写入链路删除。
- scheduler 启动条件收敛为仅服务 CRS stats 轮询。
- 后端测试更新：移除 XYAI 写入路径测试，补强历史只读兼容与空 quota fallback。
- `README.md`、`docs/specs/README.md` 更新为新口径。

### Out of scope

- `source='xy'` 历史数据的迁移、重算、脱敏或归档策略变更。
- `/api/quota/latest` 的响应结构调整。
- 新增任何替代 XYAI 的采集入口。

## 对外接口与兼容口径

### 删除的配置 / CLI

- 环境变量：
  - `XY_BASE_URL`
  - `XY_VIBE_QUOTA_ENDPOINT`
  - `XY_SESSION_COOKIE_NAME`
  - `XY_SESSION_COOKIE_VALUE`
  - `XY_LEGACY_POLL_ENABLED`
  - `XY_SNAPSHOT_MIN_INTERVAL_SECS`
- CLI 参数：
  - `--base-url`
  - `--quota-endpoint`
  - `--session-cookie-name`
  - `--session-cookie-value`
  - `--snapshot-min-interval-secs`

### 保持不变的接口

- `GET /api/invocations`
- `GET /api/stats`
- `GET /api/stats/summary`
- `GET /api/stats/timeseries`
- `GET /api/quota/latest`
- `GET /events`
- `ANY /v1/*path`

### 数据兼容性

- 历史 `codex_invocations.source='xy'` 记录继续参与读取与统计聚合。
- 历史 `codex_quota_snapshots` 继续通过 `/api/quota/latest` 暴露最新快照；空库时仍返回 degraded default。
- 清理完成后，运行时不再新增 XYAI 调用记录与 XYAI quota snapshot。

## 验收标准（Acceptance Criteria）

- Given 未配置任何 XYAI 上游 env，When 启动服务或执行 `cargo run --help`，Then 服务可正常启动，帮助输出不再包含已删除的 XYAI CLI 参数。
- Given 数据库中已有历史 `source='xy'` 调用记录，When 请求 `/api/invocations`、`/api/stats`、`/api/stats/summary`、`/api/stats/timeseries`，Then 这些历史记录仍可被读取并计入聚合。
- Given 数据库中已有 `codex_quota_snapshots` 历史快照，When 请求 `/api/quota/latest`，Then 返回最新快照；Given 快照表为空，Then 继续返回 degraded default。
- Given 服务正常运行，When 产生 CRS tick 或 proxy 请求，Then 不会触发任何 XYAI 上游抓取，也不会写入新的 `source='xy'` 记录或新的 quota snapshot。
- Given proxy / CRS 既有测试场景，When 执行相关测试，Then 行为维持现状且测试继续通过。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test`
- `cd web && npm run test`
- `cargo run --help`

### Quality checks

- `cargo fmt`

## 文档更新（Docs to Update）

- `README.md`: 移除 XYAI 接入说明与示例，明确 quota 为历史只读。
- `docs/specs/README.md`: 收录本 spec，并在交付后更新状态与 PR 备注。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 删除 XYAI-only 配置解析、legacy scheduler 分支与新写入逻辑。
- [x] M2: 更新后端测试，覆盖历史 `xy` 读取兼容与 quota latest 只读行为。
- [x] M3: 清理 README/规格索引并完成本地验证。
- [ ] M4: 创建 PR、等待 checks 明确并完成 review 收敛。

## 方案概述（Approach, high-level）

- 保留 `SOURCE_XY` 与 quota snapshot 表读取路径，只移除“从 XYAI 上游拉取并写入”的入口。
- 将 scheduler 继续作为 CRS stats 的后台轮询器使用，避免误伤现有 CRS 聚合链路。
- 通过最小代码删除与定向测试替换，确保历史兼容而不引入新的迁移成本。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：仓库外部署脚本或环境变量模板可能仍注入已移除的 XYAI env，需要在交付说明中提醒。
- 需要决策的问题：None。
- 假设（需主人确认）：仅移除 XYAI，proxy 与 CRS 保持不变。

## 变更记录（Change log）

- 2026-03-09: 新建规格，冻结“移除 XYAI 采集、保留历史读取”的范围与验收口径。
- 2026-03-09: 完成本地代码清理与验证（`cargo fmt`、`cargo test`、`cd web && npm run test`、`cargo run -- --help`）。

## 参考（References）

- `README.md`
- `src/main.rs`
