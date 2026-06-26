# 账号详情统计 read-model 与 3 秒准确展示 SLA - History

## Key Decisions

- 2026-06-19: 账号详情统计主路径切换为账号 read-model 读取，禁止再依赖在线全量重算作为常规正确性来源。
- 2026-06-19: 账号统计 read-model 拆为 minute/hourly 两层；minute 层承担自然窗口与边界精确性，hourly 层承担长周期聚合。
- 2026-06-19: `window-usage` 读取改为 minute read-model + 缺失 hourly rows + cursor 后 live tail，修复 partial bucket 漏计与 overlap 双计数风险。
- 2026-06-19: 前端详情抽屉的 `window-usage` hydrate 收紧为“仅当前选中账号”，防止 roster 刷新触发批量重型统计。
- 2026-06-19: schema ensure 顺序修正为先建 `hourly_rollup_live_progress` 再 rebuild 账号统计，避免旧库冷启动时 cursor 落盘失败。
- 2026-06-20: 生产复盘确认 `window-usage` 已降到毫秒级后，剩余体感慢点来自详情抽屉自身的重复请求编排；抽屉默认 roster 预取与非 routing tab 的 `sticky-keys` 预取已被移除。
- 2026-06-20: 生产复盘同时确认 roster `load_summaries_ms` 仍被 `pool_upstream_account_limit_samples` 的 `ranked_samples` 窗口查询拖慢；最新 usage 样本读取已切成索引友好的“最新样本 + 最新非空 plan type”组合查询。
- 2026-06-21: 继续线上追查后确认，账号详情接口 steady-state 已降到毫秒级，但后台 proxy usage startup backfill 与 stale attempt recovery 仍会制造 SQLite 争锁，把详情抽屉体感重新拖到 10 秒级；对应热点已改成 cursor / partial-index 驱动。
- 2026-06-21: summary repair 改为在 repair marker 已完成但 live cursor 落后时只刷新 cursor，避免详情 summary 继续绑定旧 repair cursor。
- 2026-06-21: archive materialization 与 bootstrap 会补齐账号 usage / stats replay marker，修复旧库在 materialized archive 缺 marker 时把账号 summary / timeseries 误拉回 archive fallback 的问题。
- 2026-06-21: account-scoped `yesterday` 活动总览拆掉重复 comparison fetch，避免详情抽屉在昨天视图额外触发一轮同账号 summary / timeseries。
- 2026-06-21: 账号详情抽屉 records tab 不再停留在一次性快照；它改为与 `Live` / `/records` 共用活动记录实时合并层，保证同账号的新记录自动出现、终态字段自动收敛，并在 SSE 重连后静默回源补齐。
- 2026-06-23: 线上 CPU 复盘确认账号池通用 hook 仍会把 invocation `records` SSE 升级成 roster/detail/window-usage 刷新；现已切断这条链，只保留业务写入、手动 refresh 与 SSE `open` 受控补齐。
- 2026-06-25: 线上回归确认默认 overview 首屏又被 `recentActions` 同步读取拖慢；详情接口默认改回不带 `recentActions`，仅在健康与事件 tab 按需 hydrate。
- 2026-06-25: records 顶部卡片字段调整后，account-scoped today summary 把 `nonSuccessCost` 拖回了 live augmentation 路径并丢失 live tail；现已恢复为 read-model totals + bounded tail，闭区间默认不做 live raw 重算。
- 2026-06-25: `selectedId` 暂空时的 roster 级 `window-usage` 自动 hydrate 被彻底移除；详情首屏只允许当前账号发 `window-usage`，手动 hydrate 才能批量取数。
