# Immutable invocation archive segments（#s6d1q）

## 状态

- Status: 已实现
- Created: 2026-03-26
- Updated: 2026-03-26

## Summary

- `codex_invocations` archive 从“可变月归档 `*.sqlite.gz`”切到“不可变上海自然日日分片 `part-<seq>.sqlite.gz`”。
- retention 正常路径只追加新 segment，不再对既有归档做 `inflate -> attach -> deflate` 覆写。
- `archive_batches` 扩展为统一 manifest，新增 `day_key`、`part_key`、`layout`、`codec`、`writer_version`、`cleanup_state`、`superseded_by`，兼容 `legacy_month` 与 `segment_v1` 两种布局。
- 启动与 retention 增加 archive temp janitor；新增 `maintenance verify-archive-storage` 与 `maintenance prune-archive-batches`。

## Implementation

- 新增配置：
  - `CODEX_INVOCATION_ARCHIVE_LAYOUT=segment_v1`
  - `CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY=day`
  - `INVOCATION_ARCHIVE_CODEC=gzip`
- `codex_invocations` 的 detail prune / max-age archive 改为按日分组，并通过 `archive_rows_into_segment_batch` 写入 `ARCHIVE_DIR/codex_invocations/YYYY/MM/DD/part-<seq>.sqlite.gz`。
- 旧 `archive_rows_into_month_batch` 保留给 legacy month archive 兼容追加，其失败路径补齐 temp file cleanup。
- `cleanup_expired_archive_batches` 继续按 manifest TTL 删除；`prune_archive_batches` 在此基础上补删已经 backup-only 的 legacy archive。
- `verify_archive_storage` 扫描 manifest 缺失文件、未登记 orphan 文件和 stale temp residue；janitor 自动清理超龄 `*.tmp` / `*.partial` / old inflated sqlite residue。

## Validation

- retention 归档老 `codex_invocations` 时，跨日样本会生成多个 `segment_v1` batch，且 `archive_batches_touched` 与实际分片数一致。
- startup manifest rebuild、historical rollup materialization 对 missing archive file 继续 fail-soft，不因缺失 archive 让在线统计查询报错。
- janitor 只删除超龄 temp residue，不误删正式 segment；verify 可以正确报告 `missing_files`、`orphan_files`、`stale_temp_files`。
- `prune-archive-batches` 能同时清理 expired segment 和 legacy backup-only archive metadata/file。

## Assumptions

- 当前 codec 默认保持 `gzip`，后续如需切换 `zstd`，只扩展 codec 抽象，不回退到 mutable month archive。
- 只对 `codex_invocations` 启用 `segment_v1`；其他 dataset 继续沿用现有 month archive。
- legacy month archive 不做在线重写或自动拆分，仅作为 backup-only 兼容产物逐步受控清理。
