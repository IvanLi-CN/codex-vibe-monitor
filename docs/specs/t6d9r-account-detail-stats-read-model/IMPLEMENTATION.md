# 账号详情统计 read-model 与 3 秒准确展示 SLA - Implementation

## Current State

- Canonical spec: `docs/specs/t6d9r-account-detail-stats-read-model/SPEC.md`
- Implementation summary: 已完成

## 状态

- Status: 已完成
- Note: 账号详情统计已从在线重算主路径切到账号 read-model 主路径；live raw 只保留 boundary 与 cursor 之后的有界精确补尾。
- Note: 前端已收紧为“仅当前选中账号”的 `window-usage` hydrate，不再因 roster 刷新批量触发详情重型统计。
- Note: `useUpstreamAccounts(...)` 不再消费 invocation `records` SSE 来静默刷新 roster/detail/window-usage；账号池重型统计只保留手动 refresh、显式业务变更和 SSE `open` 后的受控补齐。
- Note: 账号详情抽屉默认不再额外预取 roster / sticky conversation 统计；只有 `edit` / `routing` 这类真正依赖上下文的 tab 才会触发对应重查询。
- Note: 账号详情默认 `overview` 首屏已改为不再同步读取 `recentActions`；健康与事件 tab 才通过显式 follow-up detail hydrate 拉取事件流。
- Note: records 顶部卡片新增的 `nonSuccessCost` 已重新回到 read-model-first 主路径；live augmentation 只保留 `nonSuccessTokens` 与 in-progress 字段，闭区间 summary / timeseries 默认不再回退到 live raw 重算。

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
- 共享账号详情抽屉桌面壳层从 `max-w-[60rem]` 放宽到 `max-w-[90rem]`；为了让新增横向空间真实转化为概览可读性，overview 下两张 usage card 提前到 `lg` 断点进入双列，而不是继续等到 `xl`。
- records 视图总 token 指标标题改为 `Token` 单数文案，并在 `TodayStatsOverview` 中对该标签单独保留 mixed case + `whitespace-nowrap`，避免窄卡片里出现 `今日` / `TOKENS` 断成两行的问题。
- 账号详情接口 `get_upstream_account` 默认改为 `includeRecentActions=false`，把 `pool_upstream_account_events` 读取从 overview 首屏热路径中移出；health events tab 再按需补一次 detail hydrate。
- `useUpstreamAccounts(...)` 在 `selectedId` 为空时不再自动对 roster 可见行批量触发 `window-usage` hydrate；只有当前选中账号或显式手动 hydrate 才会发 `window-usage` 请求。
- account-scoped summary 将 `nonSuccessCost` 固定纳入 rollup/read-model totals，并恢复带 `full_hour_range` 的 today live tail 精确补尾，避免新增字段后 today 卡片回退成 0 或强制 raw window 重扫。
- `fetch_summary` / `fetch_stats` 按窗口类型区分 live augmentation：闭区间默认跳过 in-progress 与 non-success token live 补算；SQLite `database is locked` 时非成功 token live 增量允许受限降级，不再整排卡片长期 skeleton。
- summary / account-activity 的 in-progress augmentation 已从请求时扫描 `codex_invocations` 改成 `invocation_in_progress_live` 小表读取。该 live read model 由 `codex_invocations` trigger 与 startup rebuild 同步维护，并分别保留 summary 全局 retry 与 account-scoped retry 语义，避免 Dashboard/account detail 的当前窗口 reconcile 把 read-model 节省下来的 CPU 再吃回去。
- summary publish 当前窗口里的 `inProgressConversationCount` distinct-count 现在也直接复用 live table truth source，而不是在 maintenance 路径里对 `codex_invocations` 再做一次 `COUNT(DISTINCT prompt_cache_key)`；这让 summary 广播与账号详情共用同一份 bounded in-progress truth。
- 第三轮 SQLite 止血进一步把 prompt cache working-conversation 的 snapshot count/page 从 `codex_invocations` 热表扫描切到 write-side working-set truth；虽然公开 API shape 不变，但相关详情/工作区入口现在统一接受 `<=5s` bounded freshness，而不再为严格 snapshot membership 付出请求级扫表代价。
- proxy capture 请求尾写路径也继续收敛：`codex_invocations` 终态持久化改为单路径 upsert/finalize，`pool_upstream_request_attempts` 的 phase / latency / compact-support 进度尽量并入同一条更新，减少账号详情和 Dashboard 与请求尾写争用 SQLite 单写者预算。
- 第四轮止血把账号详情依赖的 upstream account touch、invocation hourly rollup/live progress 与 attempt 中间进度迁入进程内 SQLite batch writer。账号详情仍以同步 terminal invocation 主事实为可靠来源；派生统计和进度展示接受 `<=5s` bounded freshness，队列满时 invocation 派生写会做同步补偿以降低数据漂移风险。
- Storybook 现有详情抽屉 overlay stories 继续作为 page-fallback 证据面，新增 owner-facing 首屏概览态与 records 统计卡片完成态图片，覆盖这次性能回归修复后的默认打开路径。

## Verification

- `cargo test account_scoped_summary_and_timeseries_filter_by_payload_upstream_account_id -- --nocapture`
- `cargo test get_upstream_account_window_usage -- --nocapture`
- `cargo test ensure_schema_rebuilds_account_stats_when_live_progress_table_is_missing -- --nocapture`
- `cargo test summary_rollup_repair_refreshes_stale_repair_live_cursor_from_shared_progress -- --nocapture`
- `cargo test summary_yesterday_ignores_missing_non_overlapping_archive_batch -- --nocapture`
- `cargo test account_summary_yesterday_ignores_materialized_archive_missing_account_usage_marker -- --nocapture`
- `cargo test materialize_historical_rollups_marks_account_replay_targets_when_only_account_targets_are_pending -- --nocapture`
- `cargo test bootstrap_hourly_rollups_repairs_missing_materialized_account_replay_markers -- --nocapture`
- `cargo test get_upstream_account_omits_recent_actions_by_default_and_loads_them_on_demand -- --nocapture`
- `cargo test natural_day_summary_reports_retry_wait_and_non_success_usage -- --nocapture`
- `cargo test account_scoped_natural_day_summary_keeps_augmentation_fields_scoped -- --nocapture`
- `cargo test tests::natural_day_summary_reports_retry_wait_and_non_success_usage -- --exact`
- `cargo test tests::account_scoped_natural_day_summary_keeps_augmentation_fields_scoped -- --exact`
- `cargo test latest_usage_sample_map_keeps_latest_non_empty_sample_plan_type -- --nocapture`
- `cargo test ensure_schema_migrates_codex_invocations_off_raw_expires_at_and_adds_retention_tables -- --nocapture`
- `cargo test sqlite_batch_writer`
- `cargo test pool_upstream_request_attempt -- --test-threads=1`
- `cargo check`
- `cd web && bun x vitest run --project=unit src/hooks/useUpstreamAccounts.test.tsx src/pages/account-pool/UpstreamAccounts.test.tsx src/lib/api.test.ts`
- `cd web && bun run test-storybook`
- `cd web && bun run build-storybook`

## Visual Evidence

- `assets/detail-drawer-records-loading-raw.png`
- `assets/detail-drawer-records-settled-final-raw.png`
- `assets/detail-drawer-records-live-sync-stable.png`
- `assets/detail-drawer-records-settled-wide.png`
- `assets/detail-drawer-records-token-label-nowrap.png`

## 2026-06-21 Records Live Follow-up

- 账号详情抽屉 records tab 继续保留懒加载和旧请求丢弃约束，但列表本身改为消费共享 `records` SSE 实时 adapter，而不是一次性快照拉取后静止。
- 当前账号命中的新调用现在会自动插入到 records tab；同一 `invokeId` 后续收到更完整终态记录时，会自动替换掉先前的 `running/pending` 可见行。
- SSE 连接 `open` 后，records tab 会静默回源补齐重连窗口内可能漏掉的记录，同时不额外触发 overview / routing 这类重型统计面的重复 hydrate。
