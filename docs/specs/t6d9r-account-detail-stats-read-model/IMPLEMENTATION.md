# 账号详情统计 read-model 与 3 秒准确展示 SLA - Implementation

## Current State

- Canonical spec: `docs/specs/t6d9r-account-detail-stats-read-model/SPEC.md`
- Implementation summary: 已完成

## 状态

- Status: 已完成
- Note: 账号详情统计已从在线重算主路径切到账号 read-model 主路径；live raw 只保留 boundary 与 cursor 之后的有界精确补尾。
- Note: 前端已收紧为“仅当前选中账号”的 `window-usage` hydrate，不再因 roster 刷新批量触发详情重型统计。
- Note: 账号详情抽屉默认不再额外预取 roster / sticky conversation 统计；只有 `edit` / `routing` 这类真正依赖上下文的 tab 才会触发对应重查询。

## 落地内容

- 后端新增 `upstream_account_stats_hourly` 与 `upstream_account_stats_minute` 两层账号统计 read-model。
- 账号 summary / timeseries 改为 minute/hourly read-model 优先，边界补齐使用冻结 cursor 的精确 raw tail。
- `/api/pool/upstream-accounts/window-usage` 改为 minute read-model 优先，再合并缺失 hourly usage rows 与 cursor 之后的 live tail。
- schema ensure 顺序已修正：先确保 `hourly_rollup_live_progress` 存在，再执行账号统计 rebuild，避免旧库迁移时 cursor 保存失败。
- 账号统计 rebuild 完成后会把 invocation live cursor 写回 `hourly_rollup_live_progress`，避免冷启动后重复回放或尾部缺口。
- 账号详情抽屉内嵌 `useUpstreamAccounts(...)` 改为按 tab 懒启用：`overview` / `records` 首开不再重复拉 roster，上下文相关 tab 才补拉。
- `useUpstreamStickyConversations(...)` 改为仅在 `routing` tab 启用，避免详情首开时误触发 `sticky-keys` 预览重查询。
- upstream roster 最新 usage 样本读取已改为索引友好的“每账号最新样本 + 最新非空 plan type”组合查询，移除 `pool_upstream_account_limit_samples` 上按账号 `ROW_NUMBER()` 排序的热点慢 SQL。
- summary repair 对已完成 marker 但落后的 repair cursor 改成“只追平 cursor”，避免旧库在 readiness 通过后继续带着陈旧 summary live cursor 读详情。
- archive materialization / bootstrap 现在会修复账号 usage、账号 stats hourly、账号 stats minute 三类 replay marker；materialized archive 不再被账号 summary / timeseries 误判成未物化历史缺口。
- startup proxy usage backfill snapshot 改为共享 invocation cursor + `MAX(id)`，并为 proxy usage backfill 与 stale attempt recovery 补齐 partial index，减少后台恢复任务对详情接口的 SQLite 争锁。
- `DashboardActivityOverview` 在 account-scoped `yesterday` 视图不再额外请求 yesterday comparison summary / timeseries，消除一个重复请求源。

## Verification

- `cargo test account_scoped_summary_and_timeseries_filter_by_payload_upstream_account_id -- --nocapture`
- `cargo test get_upstream_account_window_usage -- --nocapture`
- `cargo test ensure_schema_rebuilds_account_stats_when_live_progress_table_is_missing -- --nocapture`
- `cargo test summary_rollup_repair_refreshes_stale_repair_live_cursor_from_shared_progress -- --nocapture`
- `cargo test summary_yesterday_ignores_missing_non_overlapping_archive_batch -- --nocapture`
- `cargo test account_summary_yesterday_ignores_materialized_archive_missing_account_usage_marker -- --nocapture`
- `cargo test materialize_historical_rollups_marks_account_replay_targets_when_only_account_targets_are_pending -- --nocapture`
- `cargo test bootstrap_hourly_rollups_repairs_missing_materialized_account_replay_markers -- --nocapture`
- `cargo test latest_usage_sample_map_keeps_latest_non_empty_sample_plan_type -- --nocapture`
- `cargo test ensure_schema_migrates_codex_invocations_off_raw_expires_at_and_adds_retention_tables -- --nocapture`
- `cargo check`
- `cd web && bun run test src/hooks/useUpstreamAccounts.test.tsx src/components/DashboardActivityOverview.test.tsx`
- `cd web && bun run test -- UpstreamAccounts.test.tsx useUpstreamStickyConversations.test.tsx`
- `cd web && bun run build-storybook`

## Visual Evidence

- `assets/detail-drawer-records-loading-raw.png`
- `assets/detail-drawer-records-settled-final-raw.png`
- `assets/detail-drawer-records-live-sync-stable.png`

## 2026-06-21 Records Live Follow-up

- 账号详情抽屉 records tab 继续保留懒加载和旧请求丢弃约束，但列表本身改为消费共享 `records` SSE 实时 adapter，而不是一次性快照拉取后静止。
- 当前账号命中的新调用现在会自动插入到 records tab；同一 `invokeId` 后续收到更完整终态记录时，会自动替换掉先前的 `running/pending` 可见行。
- SSE 连接 `open` 后，records tab 会静默回源补齐重连窗口内可能漏掉的记录，同时不额外触发 overview / routing 这类重型统计面的重复 hydrate。
