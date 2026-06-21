---
title: Account detail stats must resolve from account read-models within 3 seconds
module: account-pool
problem_type: performance
component: account detail stats
tags:
  - upstream-accounts
  - read-model
  - summary
  - timeseries
  - window-usage
status: active
related_specs:
  - docs/specs/t6d9r-account-detail-stats-read-model/SPEC.md
  - docs/specs/9aucy-db-retention-archive/SPEC.md
---

# Account detail stats read-model SLA

## Context

账号详情抽屉同时消费 `window-usage`、账号 summary、账号 timeseries 与记录页顶部 summary。旧实现把这些读取叠在 live rows、archive overlap 与 hourly rollup 之上，导致详情打开时出现十秒级等待。

## Symptoms

- 打开账号详情抽屉后，统计卡片和趋势图长时间空白。
- `/api/pool/upstream-accounts/window-usage` 在多账号列表刷新时批量触发，生产日志出现单次十秒级响应。
- 账号 summary / timeseries 在 mixed archive/live 窗口上需要重复扫描 raw invocations。
- 读路径本身已经压到毫秒级后，详情抽屉仍会偶发卡住 10 秒以上；生产排查显示根因来自后台 SQLite 热查询占住连接与锁，前台详情请求被连带拖慢。

## Resolution

- 为账号详情统计建立 minute/hourly 两层 read-model，并通过 invocation 写入、archive replay 与 startup bootstrap 统一维护。
- summary / timeseries 只读账号 read-model；raw invocations 只用于 boundary 精确补齐和 cursor 之后的有界 live tail。
- `window-usage` 优先读 minute read-model，再合并缺失 hourly rows 与 live tail，不再按账号窗口常态化在线重算。
- 前端只为当前选中账号 hydrate `window-usage`，避免 roster / SSE / 列表刷新批量打后端。
- 详情抽屉只在真正需要时才启用重统计上下文：`routing` 才加载 sticky conversation 统计，`edit` / `routing` 才补拉 roster 上下文，避免 `overview` / `records` 首开把无关重查询叠上去。
- 上游账号 roster 的最新 usage 样本读取已从 `ROW_NUMBER()` 窗口排序改为索引友好的“最新样本 + 最新非空 plan type”读取，去掉 `pool_upstream_account_limit_samples` 上最重的在线窗口查询。
- summary repair 完成标记与 live cursor 分离维护：如果 repair marker 已完成但 cursor 落后于共享 invocation cursor，只刷新 cursor，不再误触发整段重修或长期读旧游标。
- archive materialization 会为账号 usage / stats read-model 一并补齐 replay markers；账号 summary / timeseries 在 materialized archive 缺 marker 的旧库上，也不会再把历史批次误判成“未物化，需要在线回补”。
- startup proxy usage backfill 改为复用共享 invocation cursor + 全表 `MAX(id)`，删除“仅扫描缺 usage 的 proxy success rows”这条生产 10 秒级热点 SQL；stale attempt recovery 同时补齐 partial index，避免后台恢复任务反复争锁。
- 账号维度昨天视图拆掉重复 comparison fetch，避免 account-scoped yesterday 面板额外再打一次 yesterday summary / timeseries。

## Guardrails / Reuse Notes

- 任何账号详情统计面一旦触发加载，首次展示的数据必须是准确值，不能先展示 stale 或 approximate 值。
- 新增账号维度统计字段时，先确认 minute/hourly read-model 都能承载，再接入详情页；不要把缺失字段临时塞回 raw 在线聚合。
- 如果 schema ensure 需要在旧库上 rebuild 账号统计，必须先确保 `hourly_rollup_live_progress` 已存在，否则 rebuild 后无法保存 cursor。
- 如果 archive batch 已经 `historical_rollups_materialized_at`，账号 usage / stats target 的 replay marker 也必须视为同一事务事实；旧库升级时先修 marker，再允许详情读路径依据“缺 marker”做 archive fallback。
- 任何 startup / maintenance backfill 只要要扫 `codex_invocations` 或 `pool_upstream_request_attempts` 大表，都必须先证明有 cursor 或 partial index；后台慢查询同样会把详情页 SLA 拖垮。
- 前端详情页的重型统计 hydrate 必须绑定“当前选中账号 + 当前 query key”；列表刷新不能把整个当前页账号重新拉一遍。
- 若 roster 仍需展示最新 usage/plan 快照，优先复用主表或按账号索引直取最新样本；不要再回到 `pool_upstream_account_limit_samples` 的窗口函数全表排名路径。

## References

- `docs/specs/t6d9r-account-detail-stats-read-model/SPEC.md`
- `src/api/slices/prompt_cache_and_timeseries/summary_queries.rs`
- `src/api/slices/prompt_cache_and_timeseries/timeseries.rs`
- `src/upstream_accounts/sync_account_imports_tags.rs`
- `web/src/hooks/useUpstreamAccounts.ts`
