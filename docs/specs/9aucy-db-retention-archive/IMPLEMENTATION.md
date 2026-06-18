# 数据分层保留、离线归档与长周期汇总 - Implementation

## Current State

- Canonical spec: `docs/specs/9aucy-db-retention-archive/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Note: 本 spec 保留 retention/archive 基线；在线长期统计主来源与 archive 在线读取边界后续已由 `#h9r2m` 接管。
- Note: 2026-06-18 继续收口 mixed archive/live summary 边界；summary 读取不再因为 `window.start` 落在“当前 retention cutoff 之后”就退回 live-only，而是固定与 hourly timeseries 共用 rollup-backed 读路径，避免 `previous7d` 这类自然日窗口漏掉先前已 materialize 的历史天数。

## Verification

- `cargo test previous7d_summary_matches_daily_timeseries_when_window_spans_archived_and_live_days -- --nocapture`
- `cargo test archived_range_reads_skip_archive_fallback_rows_already_counted_in_live_tail -- --nocapture`
- `cargo fmt`

## Migrated Task-Ticket Sections

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

## Migrated Implementation Sections

## 101 Rollout Gate

- 首次上线前先执行 `--retention-run-once --retention-dry-run`，确认预计归档行数、archive 文件数与磁盘变化。
- 首次真实清理后，需要保留四组证据：dry-run 计数、archive batch 文件清单、数据库体积前后对比、`/api/stats/summary?window=all` 与 `/api/invocations?limit=200` 核验结果。
- backlog cleanup 完成后，在维护窗口人工执行一次 `VACUUM`，不把它放进常驻任务。
