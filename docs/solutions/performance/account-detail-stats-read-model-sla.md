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
- window usage handler 通过 bounded StoragePlane 协调同参读取；selection 以 LRU 管理，不把缓存 TTL 当成统计正确性。冷 baseline、archive coverage hole 或当前活动窗口内匹配的 legacy account row 尚未完成结构化回填时显式返回 `202 preparing`，已有 last-good 最多服务 60 秒。合法 unassigned terminal 使用 NULL，但不属于 legacy；legacy 仅指 payload 指定账号而结构化列缺失。并发相同账号集合共享 preflight，完成 proof 以账号作用域 durable cursor 绑定，不让无关账号 terminal 使健康 selection 反复重建。只有 preflight 已证明相同账号、窗口与 reset 配置时才可复用 last-good，并以 durable cursor 而非构建完成时间选择最新值；legacy completion cache 也必须同时受 128 项上限和 durable cursor 约束，不能将旧 completion proof 用于后续 terminal。回填 cursor 必须与 selection 绑定并单调前进，future-reset generation 不能与 rolling generation 共享 cursor；不能让持续的新行反复遮住较早的 legacy 行，也不能让窗口外历史 backlog 阻断当前选择。结构化归属回填最多 32 行且目标事务为 200ms；它不重复重建已经按 `COALESCE(structured, attempt/payload)` 正确 materialize 的小时 rollup。live coverage repair 独立执行，先获 SQLite write coordinator，再取得 global pressure permit，取消、压力或两秒执行预算超时均重新排队，避免 maintenance waiter 占用 background slot 并阻塞 P1。一个缺 hourly coverage 的完整小时必须选择 exact bucket 或 complete minute/hour rollup 中的一条路径，不能叠加 partial minute 与 raw fallback。legacy archive 缺少 additive account column 时从 payload 恢复归属。archive fallback 与 bootstrap rebuild 都先验证 completed manifest hash，replay marker 也必须保存该 hash；旧 marker 缺失 hash 时必须 repair，不能把同路径的替换文件当成原历史事实。单账号详情在 preparing 期间保留可空 usage 或共享的 last-good，绝不启动全窗 archive/raw 兼容读取。
- `codex_invocations.upstream_account_id` 是新 terminal 的结构化账号维度，并由 partial index 支持 bounded exact tail；旧 payload 归属只允许 pressure-gated 小批回填，不能继续作为健康查询过滤条件。
- 前端只为当前选中账号 hydrate `window-usage`，避免 roster / SSE / 列表刷新批量打后端。
- invocation `records` SSE 不得直接驱动账号池 roster 或 `window-usage` refresh；这些刷新会把“记录实时性”误升级成重型统计重算。
- 详情抽屉只在真正需要时才启用重统计上下文：`routing` 才加载 sticky conversation 统计，`edit` / `routing` 才补拉 roster 上下文，避免 `overview` / `records` 首开把无关重查询叠上去。
- 上游账号 roster 的最新 usage 样本读取已从 `ROW_NUMBER()` 窗口排序改为索引友好的“最新样本 + 最新非空 plan type”读取，去掉 `pool_upstream_account_limit_samples` 上最重的在线窗口查询。
- summary repair 完成标记与 live cursor 分离维护：如果 repair marker 已完成但 cursor 落后于共享 invocation cursor，只刷新 cursor，不再误触发整段重修或长期读旧游标。
- archive materialization 会为账号 usage / stats read-model 一并补齐 replay markers；账号 summary / timeseries 在 materialized archive 缺 marker 的旧库上，也不会再把历史批次误判成“未物化，需要在线回补”。
- startup proxy usage backfill 改为复用共享 invocation cursor + 全表 `MAX(id)`，删除“仅扫描缺 usage 的 proxy success rows”这条生产 10 秒级热点 SQL；stale attempt recovery 同时补齐 partial index，避免后台恢复任务反复争锁。
- 账号维度昨天视图拆掉重复 comparison fetch，避免 account-scoped yesterday 面板额外再打一次 yesterday summary / timeseries。
- 默认账号详情接口不再同步读取 `recentActions`；事件流读取改成 health/events tab 的显式 follow-up fetch，避免 `pool_upstream_account_events` 把 overview 首屏重新拖慢。
- 新增的 `nonSuccessCost` 重新回到 summary read-model totals；today 等开区间仍只做 bounded live tail，`yesterday` / `previous7d` 等闭区间不再因为字段补算回退到 raw live augmentation。
- `selectedId` 暂空时不再触发 roster 级 `window-usage` 自动 hydrate；只有当前选中账号或显式手动批量 hydrate 才会命中 `/api/pool/upstream-accounts/window-usage`。

## Guardrails / Reuse Notes

- 任何账号详情统计面一旦触发加载，首次展示的数据必须是准确值，不能先展示 stale 或 approximate 值。
- 新增账号维度统计字段时，先确认 minute/hourly read-model 都能承载，再接入详情页；不要把缺失字段临时塞回 raw 在线聚合。
- 如果 schema ensure 需要在旧库上 rebuild 账号统计，必须先确保 `hourly_rollup_live_progress` 已存在，否则 rebuild 后无法保存 cursor。
- 如果 archive batch 已经 `historical_rollups_materialized_at`，账号 usage / stats target 的 replay marker 也必须视为同一事务事实；旧库升级时先修 marker，再允许详情读路径依据“缺 marker”做 archive fallback。
- 任何 startup / maintenance backfill 只要要扫 `codex_invocations` 或 `pool_upstream_request_attempts` 大表，都必须先证明有 cursor 或 partial index；后台慢查询同样会把详情页 SLA 拖垮。
- 前端详情页的重型统计 hydrate 必须绑定“当前选中账号 + 当前 query key”；列表刷新不能把整个当前页账号重新拉一遍。
- 默认详情首屏如果只需要概览信息，接口必须把非首屏字段拆成显式 opt-in 参数或 follow-up fetch；不要把事件流、审计流或其他历史列表默默塞回默认 detail 响应。
- 账号作用域的近期尝试列表可以按 `(upstream_account_id, occurred_at)` 索引读取并只在 `SELECT` 中从 invocation JSON payload 投影展示字段；对旧 SQLite 数据库，不能把尚未存在的可选列放进查询，即使该列只用于显示模型名称。
- 若某条 SSE 只携带 invocation records，它最多只能更新 records/live 表层；不能反向触发 roster、summary、timeseries 或 `window-usage` 这类重型面。
- 若 roster 仍需展示最新 usage/plan 快照，优先复用主表或按账号索引直取最新样本；不要再回到 `pool_upstream_account_limit_samples` 的窗口函数全表排名路径。
- minute/hourly rollup 的覆盖粒度必须与回退粒度一致。单个 minute row 只排除同一分钟的 exact row；只有 60 个完整 minute buckets 才能替代一个 hourly bucket。

## References

- `docs/specs/t6d9r-account-detail-stats-read-model/SPEC.md`
- `src/api/slices/prompt_cache_and_timeseries/summary_queries.rs`
- `src/api/slices/prompt_cache_and_timeseries/timeseries.rs`
- `src/upstream_accounts/sync_account_imports_tags.rs`
- `web/src/hooks/useUpstreamAccounts.ts`
