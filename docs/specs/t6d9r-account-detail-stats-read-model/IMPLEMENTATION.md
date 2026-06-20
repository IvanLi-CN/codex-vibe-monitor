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

## Verification

- `cargo test account_scoped_summary_and_timeseries_filter_by_payload_upstream_account_id -- --nocapture`
- `cargo test get_upstream_account_window_usage -- --nocapture`
- `cargo test ensure_schema_rebuilds_account_stats_when_live_progress_table_is_missing -- --nocapture`
- `cargo test latest_usage_sample_map_keeps_latest_non_empty_sample_plan_type -- --nocapture`
- `cargo check`
- `cd web && bun run test src/hooks/useUpstreamAccounts.test.tsx src/components/DashboardActivityOverview.test.tsx`
- `cd web && bun run test -- UpstreamAccounts.test.tsx useUpstreamStickyConversations.test.tsx`
- `cd web && bun run build-storybook`

## Visual Evidence

- `assets/detail-drawer-records-loading-raw.png`
- `assets/detail-drawer-records-settled-final-raw.png`
