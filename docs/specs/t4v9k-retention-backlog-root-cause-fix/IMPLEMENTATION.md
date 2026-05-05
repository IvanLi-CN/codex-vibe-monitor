# Retention backlog root-cause fix - Implementation

## Current State

- Canonical spec: `docs/specs/t4v9k-retention-backlog-root-cause-fix/SPEC.md`
- Implementation summary: 已实现，待 PR / CI 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI 收敛
- Created: 2026-03-24
- Last: 2026-03-24

## Validation

- schema migration / idempotency
- raw compression catch-up budget
- manifest rebuild + manifest-only archive backfill
- invocation archive TTL cleanup
- maintenance CLI dry-run/live
- `StatsResponse.maintenance` 前后端兼容
