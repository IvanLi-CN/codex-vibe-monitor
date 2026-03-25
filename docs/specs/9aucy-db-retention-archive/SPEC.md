# 数据分层保留、离线归档与长周期汇总（#9aucy）

## 状态

- Status: 已完成

## 背景 / 问题陈述

- 线上数据库的主要压力来自调用明细里的原始 payload / raw response / raw file 引用，以及持续增长的代理尝试与统计快照表。
- 这些数据主要用于短期排障，但当前主库长期保留了过多不再常用的原始细节，导致 SQLite 主文件膨胀、维护成本上升、首次冷数据清理风险变高。
- 长期趋势统计仍然有价值，因此方案需要在“主库减压”与“全局 totals 不缩水”之间做分层：短期保留可排障明细，长期在线只保留聚合，完整旧明细转入离线归档。

## 目标 / 非目标

### Goals

- 为 `codex_invocations`、`forward_proxy_attempts`、`stats_source_snapshots`、`codex_quota_snapshots` 建立按上海自然日 / 自然月切分的冷热分层策略。
- 让 `/api/invocations` 与 `InvocationTable` 在展开详情中明确告知记录当前是 `Full` 还是 `Structured only`，避免误判细节完整性。
- 通过 `invocation_rollup_daily` 承接被归档删除的调用总量，确保 `/api/stats` 与 `summary?window=all` 在清理前后 totals 一致。
- 固化离线归档格式、运维开关、执行顺序与 101 首次 rollout 验证口径，保证维护任务可重试、可核查、可回滚。

### Non-goals

- 不切换到非 SQLite 存储。
- 不为 archived 明细增加在线查询 UI。
- 不让现有排障接口回读离线归档文件。
- 不在本轮实现异机归档传输或外部归档编目系统。

## 范围（Scope）

### In scope

- SQLite schema 扩展：`codex_invocations.detail_level/detail_pruned_at/detail_prune_reason`、`archive_batches`、`invocation_rollup_daily`。
- retention/archive 运维配置与 CLI：`XY_RETENTION_*` / `XY_ARCHIVE_DIR` / `--retention-run-once` / `--retention-dry-run`。
- 调用明细 30/90 天分层、月度 `sqlite.gz` 归档、manifest 校验、主库 purge、raw file 删除与 orphan sweep。
- `forward_proxy_attempts`、`stats_source_snapshots`、`codex_quota_snapshots` 的在线保留、离线归档与压缩策略。
- `summary?window=all` / 总量统计对 `invocation_rollup_daily` 的承接。
- `README.md`、`docs/deployment.md`、`docs/specs/README.md` 与前端 `InvocationTable` 的契约更新。

### Out of scope

- archived 明细在线搜索、筛选、回放。
- `stats_source_deltas` 的清理策略调整。
- 任何依赖 archived 明细的新增 API 或页面。

## 数据生命周期与保留策略

### `codex_invocations`

- 成功记录超过 30 个上海自然日后，先把该月完整记录写入离线 archive，再让主库仅保留结构化统计字段；原始 payload、raw response、request/response raw file 引用清空，并写入：
  - `detail_level='structured_only'`
  - `detail_pruned_at=<maintenance timestamp>`
  - `detail_prune_reason='success_over_30d'`
- 任意记录超过 90 个上海自然日时，先归档到 `archives/<table>/<yyyy>/<table>-<yyyy-mm>.sqlite.gz`，校验 `row_count` 与 `sha256` 成功后写入 `archive_batches`，再从主库删除。
- 离线归档前必须先将待删调用折叠进 `invocation_rollup_daily`，确保长期 totals 不缩水。
- 运行时不再维护 `raw_expires_at`；历史 archive `sqlite.gz` 中若仍带该列，不作为新版本的在线契约，也不执行离线回写重做。

### `forward_proxy_attempts` / `stats_source_snapshots`

- 主库只保留最近 30 个上海自然日的在线排障明细。
- 超过窗口的数据走与调用明细一致的“按表、按月、先归档后删除”流程，并登记到 `archive_batches`。

### `codex_quota_snapshots`

- 最近 30 个上海自然日保留全量。
- 更老数据在主库内压缩为“每个上海自然日只保留最后一条”；被折叠掉的重复快照先写入离线归档，再从主库删除。
- 压缩后的日级配额快照长期在线保留，`/api/quota/latest` 行为不变。

### `stats_source_deltas`

- 长期在线保留，不参与本轮清理。
- 继续作为 CRS 聚合历史的长期来源。

## 对外接口与契约

### HTTP / SSE / UI

- `/api/invocations` 新增字段：
  - `detailLevel`: `full | structured_only`
  - `detailPrunedAt?: string`
  - `detailPruneReason?: string`
- `/api/invocations` 不再返回 `rawExpiresAt`；这是一次显式 breaking change，调用方应改用 `detailLevel` / `detailPrunedAt` 理解在线细节保留状态。
- `InvocationTable` 仅在展开详情中显示 `Full` / `Structured only` 徽标；若记录已精简，还要在详情中显示精简时间，并提示“离线 archive 保留归档行，超窗 raw file 不保证继续可用”。列表摘要不展示 detail level。orphan sweep 只清理超过宽限期的未引用文件，避免误删进行中的请求落盘文件。
- 旧记录缺少新字段时按 `detailLevel=full` 兼容渲染。

### 查询边界

- `/api/invocations`、`/api/stats/errors`、`/api/stats/failures/summary`、`/api/stats/prompt-cache-conversations`、`/api/stats/forward-proxy` 只查询在线 retention window，不接 archived 明细。
- `/api/stats` 与 `/api/stats/summary?window=all` 读取“主库在线明细 + invocation_rollup_daily”，归档前后总请求数、成功/失败数、tokens、cost 必须一致。
- `build_raw_response_preview` 的 16KiB 上限保持不变；`raw_response` 明确只承载 preview，完整代理响应原文继续以 `response_raw_path` 为准。长期减压由分层保留与离线归档承担，而不是缩短 preview。

### 运维配置

- 新增环境变量：
  - `XY_RETENTION_ENABLED`
  - `XY_RETENTION_DRY_RUN`
  - `XY_RETENTION_INTERVAL_SECS`
  - `XY_RETENTION_BATCH_ROWS`
  - `XY_ARCHIVE_DIR`
  - `XY_INVOCATION_SUCCESS_FULL_DAYS`
  - `XY_INVOCATION_MAX_DAYS`
  - `XY_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS`
  - `XY_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS`
  - `XY_QUOTA_SNAPSHOT_FULL_DAYS`
- `PROXY_RAW_RETENTION_DAYS` 不再作为公开运行配置；raw file 生命周期由 invocation retention 窗口间接驱动。
- 新增 CLI：
  - `--retention-run-once`
  - `--retention-dry-run`

## 归档与维护约束

- 所有删除动作都必须遵守 `导出成功 -> manifest 成功 -> 删除源数据`。
- `archive_batches` 至少记录：`dataset`、`month_key`、`file_path`、`sha256`、`row_count`、`created_at`、`status`。
- 维护任务按 `XY_RETENTION_BATCH_ROWS` 分批执行，避免一次长事务锁住整个 SQLite。
- 被精简或归档的记录，其关联 raw 文件要立即删除；另外执行 orphan sweep，按文件名反查主库引用并清理无引用文件。缺失文件视为可接受且必须幂等。
- live DB 与新创建 archive DB 均不再包含 `raw_expires_at`；历史 archive 文件保持只读兼容，不在本轮做离线 schema 重写。
- 常驻任务只执行 `PRAGMA wal_checkpoint(PASSIVE)` 与 `PRAGMA optimize`；`VACUUM` 不放进周期任务，由 101 首次 backlog cleanup 完成后的维护窗口人工执行一次。

## Task Orchestration

- wave: 1
  - main-agent => 新建 `docs/specs/9aucy-db-retention-archive/SPEC.md` 与 `docs/specs/README.md` 索引项，锁定 retention tier、archive batch 命名、主库边界、101 rollout gate 与验证口径 (skill: $fast-flow + $docs-no-revision-markers)
- wave: 2
  - main-agent => 扩展后端 schema 与配置：为 `codex_invocations`、`archive_batches`、`invocation_rollup_daily` 增加迁移与默认值，并接入新的 env/CLI retention 开关 (skill: $fast-flow)
  - main-agent => 新增 retention 维护入口与生命周期接线：常驻 maintenance loop、`--retention-run-once`、`--retention-dry-run`、batch-size 控制与 cancel/shutdown 行为 (skill: $fast-flow)
- wave: 3
  - main-agent => 实现调用明细的 30/90 天分层策略、月度 archive sqlite.gz 导出、manifest 校验、daily rollup 回填与主库 purge 流程 (skill: $fast-flow)
  - main-agent => 实现 `forward_proxy_attempts`、`stats_source_snapshots`、`codex_quota_snapshots` 的归档/清理/压缩策略，以及 raw file 删除与 orphan sweep (skill: $fast-flow)
- wave: 4
  - main-agent => 改造查询层：`summary all` 与总量统计读取 live detail + `invocation_rollup_daily`，其他排障接口保持 live-window only，并补齐告警/日志 (skill: $fast-flow)
  - main-agent => 扩展 `/api/invocations` 返回字段、`web/src/lib/api.ts` 类型与 `InvocationTable` 细节状态展示，同时更新 `README.md`、`docs/deployment.md`、101 部署说明 (skill: $fast-flow + $docs-no-revision-markers)
- wave: 5
  - main-agent => 补齐 Rust 单测/集成测试与前端组件测试，覆盖迁移、dry-run、archive manifest、purge 后 totals 不变、quota compaction、orphan sweep、UI badge 呈现 (skill: $fast-flow)
  - main-agent => 在 101 上执行 dry-run、记录预计归档行数/文件/磁盘变化，真实执行首次 cleanup、跑 `VACUUM`、收集 before/after 体积与 API 响应证据 (skill: $fast-flow)
- wave: 6
  - main-agent => push 分支、创建 PR、附上 101 rollout 证据与回滚说明、收敛 checks 与 review 反馈直到状态清晰且可合并 (skill: $codex-review-loop + $fast-flow)

## 验收标准（Acceptance Criteria）

- 成功调用超过 30 天后，主库在线记录仍可用于结构化排障，但 `detailLevel` 变为 `structured_only`，并明确标出精简时间与原因。
- 超过 90 天的调用明细、超过 30 天的代理尝试与统计快照，在归档文件与 `archive_batches` 清单成功生成后，才能从主库删除。
- `summary?window=all` 与总量统计在归档前后完全一致；长期 totals 依赖 `invocation_rollup_daily` 与 `stats_source_deltas`，而不是 archived 明细在线回查。
- 最近 30 天的 `codex_quota_snapshots` 逐条保留，更老日期只保留每天最后一条在线记录。
- 前端旧 payload 缺失新字段时仍能稳定渲染，并在展开详情中默认按 `Full` 展示。

## 101 Rollout Gate

- 首次上线前先执行 `--retention-run-once --retention-dry-run`，确认预计归档行数、archive 文件数与磁盘变化。
- 首次真实清理后，需要保留四组证据：dry-run 计数、archive batch 文件清单、数据库体积前后对比、`/api/stats/summary?window=all` 与 `/api/invocations?limit=200` 核验结果。
- backlog cleanup 完成后，在维护窗口人工执行一次 `VACUUM`，不把它放进常驻任务。

## 参考

- `README.md`
- `docs/deployment.md`
- `web/src/lib/api.ts`
- `web/src/components/InvocationTable.tsx`
