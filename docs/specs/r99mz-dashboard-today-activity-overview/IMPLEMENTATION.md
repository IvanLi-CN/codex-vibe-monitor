# Dashboard：把“今日”并入“活动总览”，并为今日新增分钟级柱状 / 累计面积图 - Implementation

## Current State

- Canonical spec: `docs/specs/r99mz-dashboard-today-activity-overview/SPEC.md`
- Implementation summary: 已实现

## Migrated Implementation Notes

## 状态

- Status: 已实现
- Created: 2026-04-08
- Last: 2026-04-11

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Frontend targeted tests:
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor/web && bunx vitest run src/components/DashboardTodayActivityChart.test.tsx src/components/PromptCacheConversationTable.test.tsx src/lib/promptCacheLive.test.ts src/hooks/useTimeseries.test.ts src/lib/api.test.ts`
- Backend targeted tests:
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test timeseries_and_summary_do_not_treat_running_rows_with_failure_metadata_as_failures -- --nocapture`
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test timeseries_and_summary_count_completed_rows_as_success -- --nocapture`
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test timeseries_hourly_backed_ignores_missing_exact_archive_batch -- --nocapture`
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test all_time_summary_missing_archive_does_not_mark_repair_complete -- --nocapture`
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test prompt_cache_last24h_requests_keep_null_status_rows_neutral -- --nocapture`
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test prompt_cache_last24h_requests_treat_running_rows_with_error_text_as_failures -- --nocapture`
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test prompt_cache_last24h_requests_treat_pending_rows_with_failure_kind_as_failures -- --nocapture`
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test combined_totals_count_legacy_null_status_failures_when_only_downstream_error_exists -- --nocapture`
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test combined_totals_count_legacy_http_200_failures_when_only_downstream_error_exists -- --nocapture`
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test timeseries_and_summary_count_http_200_rows_with_downstream_only_failure_metadata -- --nocapture`
- Storybook build:
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor/web && bun run build-storybook`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/r99mz-dashboard-today-activity-overview/SPEC.md`
