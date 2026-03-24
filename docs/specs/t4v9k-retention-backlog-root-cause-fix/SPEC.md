# Retention backlog root-cause fix（#t4v9k）

## Summary

- 将 raw 冷压缩从“靠 `*.gz` 后缀猜状态”改为 `request_raw_codec` / `response_raw_codec` 显式状态，修掉候选查询无法稳定命中索引的问题。
- retention 在 raw backlog 存在时，按 `RETENTION_CATCHUP_BUDGET_SECS` 连续执行多个压缩 batch，优先追平最近堆积，而不是每小时只跑一轮。
- `codex_invocations` archive batch 现在带 `INVOCATION_ARCHIVE_TTL_DAYS` 与 `archive_batch_upstream_activity` manifest；startup archive backfill 只读 manifest，不再临时解压 archive SQLite。
- 新增运维入口：
  - `codex-vibe-monitor maintenance raw-compression [--dry-run]`
  - `codex-vibe-monitor maintenance archive-upstream-activity-manifest [--dry-run]`
- `/api/stats` 与 `/api/stats/summary` 可选返回 `maintenance` 字段，暴露 raw backlog 和 startup archive backfill 观测值。

## Scope

- 后端 schema:
  - live/archive `codex_invocations` 增加 `request_raw_codec` / `response_raw_codec`
  - `archive_batches` 增加 `upstream_activity_manifest_refreshed_at`
  - 新增 `archive_batch_upstream_activity(archive_batch_id, account_id, last_activity_at)`
- retention 行为:
  - raw 冷压缩按 request/response 双 lane 扫描
  - 候选 SQL 不再使用 `NOT LIKE '%.gz'`
  - 存在 backlog 时在单轮 budget 内连续 catch up
- archive 行为:
  - 新 `codex_invocations` archive batch 写 manifest
  - 历史 batch 可由 maintenance 重建 manifest
  - invocation archive 默认再保留 30 天，并在 TTL cleanup 时连同 manifest 一起删除
- 可观测性:
  - `maintenance.rawCompressionBacklog`
  - `maintenance.startupBackfill`

## Data Model

- `request_raw_codec` / `response_raw_codec`
  - 取值：`identity | gzip`
  - 旧数据迁移时按 path suffix 一次性映射
- raw pending partial indexes:
  - `idx_codex_invocations_request_raw_pending (occurred_at, id) WHERE request_raw_path IS NOT NULL AND request_raw_codec = 'identity'`
  - `idx_codex_invocations_response_raw_pending (occurred_at, id) WHERE response_raw_path IS NOT NULL AND response_raw_codec = 'identity'`
- `archive_batch_upstream_activity`
  - 每个 invocation archive batch 存储 account 级 `MAX(occurred_at)`
  - startup archive backfill 仅从该表恢复 `pool_upstream_accounts.last_activity_at`

## Operations

- 101 上线后的推荐顺序：
  1. `maintenance archive-upstream-activity-manifest`
  2. `maintenance raw-compression`
  3. 常规 `--retention-run-once`
- raw backlog 告警阈值：
  - `warn`: `oldestUncompressedAgeSecs >= 24h` 或 `uncompressedBytes >= 10 GiB`
  - `critical`: `oldestUncompressedAgeSecs >= 48h` 或 `uncompressedBytes >= 20 GiB`
- startup archive backfill 若发现仍有未补 manifest 的历史 batch：
  - 记录 `waiting_for_manifest_backfill`
  - 进入 idle backoff
  - 不再临时解压 archive 文件热循环

## Validation

- schema migration / idempotency
- raw compression catch-up budget
- manifest rebuild + manifest-only archive backfill
- invocation archive TTL cleanup
- maintenance CLI dry-run/live
- `StatsResponse.maintenance` 前后端兼容
