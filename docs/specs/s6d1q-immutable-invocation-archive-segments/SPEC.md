# Immutable invocation archive segments（#s6d1q）

## Summary

- `codex_invocations` archive 从“可变月归档 `*.sqlite.gz`”切到“不可变上海自然日日分片 `part-<seq>.sqlite.gz`”。
- retention 正常路径只追加新 segment，不再对既有归档做 `inflate -> attach -> deflate` 覆写。
- `archive_batches` 扩展为统一 manifest，新增 `day_key`、`part_key`、`layout`、`codec`、`writer_version`、`cleanup_state`、`superseded_by`，兼容 `legacy_month` 与 `segment_v1` 两种布局。
- 启动与 retention 增加 archive temp janitor；新增 `maintenance verify-archive-storage` 与 `maintenance prune-archive-batches`。

## Assumptions

- 当前 codec 默认保持 `gzip`，后续如需切换 `zstd`，只扩展 codec 抽象，不回退到 mutable month archive。
- 只对 `codex_invocations` 启用 `segment_v1`；其他 dataset 继续沿用现有 month archive。
- legacy month archive 不做在线重写或自动拆分，仅作为 backup-only 兼容产物逐步受控清理。
