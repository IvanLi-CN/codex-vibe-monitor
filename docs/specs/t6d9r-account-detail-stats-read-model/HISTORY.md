# 账号详情统计 read-model 与 3 秒准确展示 SLA - History

## Key Decisions

- 2026-06-19: 账号详情统计主路径切换为账号 read-model 读取，禁止再依赖在线全量重算作为常规正确性来源。
- 2026-06-19: 账号统计 read-model 拆为 minute/hourly 两层；minute 层承担自然窗口与边界精确性，hourly 层承担长周期聚合。
- 2026-06-19: `window-usage` 读取改为 minute read-model + 缺失 hourly rows + cursor 后 live tail，修复 partial bucket 漏计与 overlap 双计数风险。
- 2026-06-19: 前端详情抽屉的 `window-usage` hydrate 收紧为“仅当前选中账号”，防止 roster 刷新触发批量重型统计。
- 2026-06-19: schema ensure 顺序修正为先建 `hourly_rollup_live_progress` 再 rebuild 账号统计，避免旧库冷启动时 cursor 落盘失败。
