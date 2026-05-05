# Permanent online hourly stats retention - Implementation

## Current State

- Canonical spec: `docs/specs/h9r2m-permanent-online-hourly-rollups/SPEC.md`
- Implementation summary: 已实现

## Migrated Implementation Notes

## 状态

- Status: 已实现
- Created: 2026-03-21
- Updated: 2026-03-25

## 验证

- `cargo check`
- Rust targeted tests covering:
  - invocation hourly continuity across archive boundary
  - forward proxy historical hourly continuity after retention
  - prompt cache / sticky key aggregate continuity
- Rust targeted tests covering:
  - legacy archive materialization + prune
  - missing archive file no longer breaks historical stats
  - pool attempt route is live-only
- `cd web && bun run test -- api.test.ts`
