# 修复 archive manifest 重复账号唯一键冲突 - Implementation

## Current State

- Canonical spec: `docs/specs/b3n4q-archive-manifest-dedupe-fix/SPEC.md`
- Implementation summary: 已实现，待 PR / CI 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI 收敛
- Created: 2026-03-27
- Last: 2026-03-27

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
