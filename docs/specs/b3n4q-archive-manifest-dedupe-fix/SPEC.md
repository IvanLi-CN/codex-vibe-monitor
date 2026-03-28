# 修复 archive manifest 重复账号唯一键冲突（#b3n4q）

## 状态

- Status: 已实现，待 PR / CI 收敛
- Created: 2026-03-27
- Last: 2026-03-27

## 背景 / 问题陈述

- 101 当前 `ai-codex-vibe-monitor` 在 `2026-03-27T11:08:12Z` 报出 `UNIQUE constraint failed: archive_batch_upstream_activity.archive_batch_id, archive_batch_upstream_activity.account_id`，整轮 retention maintenance 被中断。
- 当前线上 live DB 仍有 `75,004` 条带 raw 路径记录，其中 `4,334` 条已超过 7 天；raw volume 仍约 `125.0G / 147,161 files`，说明 archive/prune 没有稳定清掉老数据。
- 根因不是 `t4v9k` 未上线，而是 archive writer 仍会把跨 `BACKFILL_ACCOUNT_BIND_BATCH_SIZE=400` chunk 的同一 `upstreamAccountId` 重复送入 manifest 写入路径，最终撞上 `(archive_batch_id, account_id)` 唯一键。

## 目标 / 非目标

### Goals

- `archive_batch_upstream_activity` 写入前必须按 `account_id` 去重，并保留该 batch 内最大的 `last_activity_at`。
- `upsert_archive_batch_manifest()`、archive writer、`maintenance archive-upstream-activity-manifest` 统一复用同一套去重逻辑。
- manifest 写入改成幂等 upsert，未来即使调用方再次传入重复账号，也不能再把 retention 卡死。

### Non-goals

- 不改公开 HTTP/API 契约，不新增前端字段。
- 不改 retention 窗口、archive layout、raw codec 或 stats maintenance 字段。
- 不重做 archive/backfill 调度，只修 manifest rows 的去重与幂等写入。

## 功能与行为规格（Functional / Behavior Spec）

- archive writer 生成 `upstream_last_activity` 时，必须先对 `(account_id, last_activity_at)` 聚合：
  - 每个账号只保留一行；
  - `last_activity_at` 取该账号在当前 archive batch 内的最大时间戳；
  - 输出顺序稳定，避免 manifest diff 抖动。
- `write_archive_batch_upstream_activity()` 在写入前必须再次做去重保护，并使用 `ON CONFLICT(archive_batch_id, account_id) DO UPDATE`：
  - 冲突时保留更大的 `last_activity_at`；
  - 重复输入不能再触发唯一键错误。
- `upsert_archived_upstream_last_activity()` 只消费去重后的 values，避免对同一账号做重复回填。
- `refresh_archive_upstream_activity_manifest()` 的 `account_rows_written` 必须反映实际写入的去重后账号数。

## 验收标准（Acceptance Criteria）

- Given 单个 archive batch 中同一 `upstreamAccountId` 跨越多个 `400` 行 chunk，When `run_data_retention_maintenance()` 执行 archive，Then 不再报唯一键错误，archive 成功落盘，live rows 被删除，对应 raw 文件被清空。
- Given archive 文件内同一账号有多条历史 invocation，When `maintenance archive-upstream-activity-manifest` 重建 manifest，Then manifest 中每个账号只落一行，且 `last_activity_at` 为该账号最大时间戳。
- Given manifest 写入收到重复 `(account_id, last_activity_at)` 输入，When 执行写入，Then 同一 `(archive_batch_id, account_id)` 最终只有一行，并能幂等重跑。

## 非功能性验收 / 质量门槛（Quality Gates）

- `cargo fmt --check`
- `cargo check`
- `cargo test retention_archives_duplicate_upstream_activity_across_chunks -- --test-threads=1`
- `cargo test archive_manifest_refresh_dedupes_duplicate_account_rows_from_archive_file -- --test-threads=1`
- `cargo test archive_backfill_waits_for_manifest_until_rebuilt -- --test-threads=1`
- `cargo test archive_manifest_refresh_leaves_missing_batches_pending_for_retry -- --test-threads=1`
- `cargo test retention_archives_rows_with_compressed_raw_payload_files -- --test-threads=1`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/b3n4q-archive-manifest-dedupe-fix/SPEC.md`

## 方案概述（Approach, high-level）

- 在后端新增共享 helper，把 archive manifest 的 `(account_id, last_activity_at)` 输入先收敛成唯一账号列表，再复用到 archive writer、manifest rebuild 和账号 last-activity 回填。
- 用幂等 upsert 兜住 manifest 写入路径，保证重复账号最多更新 `last_activity_at`，不再抛唯一键错误。
- 补两条针对性回归：一条锁定 live archive 的 chunk 重复账号缺陷，一条覆盖 maintenance manifest rebuild 的最终对账语义。

## 实现与验证记录

- 已在 `src/main.rs` 落地共享去重 helper，并让 archive writer、manifest rebuild、batch manifest upsert 与 archived upstream last-activity 回填统一走去重后的账号列表。
- `write_archive_batch_upstream_activity()` 已改为幂等 upsert；同一 `(archive_batch_id, account_id)` 重复写入时保留更大的 `last_activity_at`。
- 已新增回归：
  - `retention_archives_duplicate_upstream_activity_across_chunks`
  - `archive_manifest_refresh_dedupes_duplicate_account_rows_from_archive_file`
- 已完成本地验证：
  - `cargo fmt`
  - `cargo check`
  - `cargo test retention_archives_duplicate_upstream_activity_across_chunks -- --test-threads=1`
  - `cargo test archive_manifest_refresh_ -- --test-threads=1`
  - `cargo test archive_backfill_waits_for_manifest_until_rebuilt -- --test-threads=1`
  - `cargo test retention_archives_rows_with_compressed_raw_payload_files -- --test-threads=1`
