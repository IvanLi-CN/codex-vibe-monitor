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
- 2026-06-29: 101 线上 CPU 复盘确认，剩余 summary / account-activity 热点来自请求时对 `codex_invocations` 做 in-progress correlated scan / group scan；对应 live augmentation 已切到 write-side `invocation_in_progress_live` 小表，并保留 summary 全局 retry 与 account-scoped retry 的既有语义。
- 2026-06-30: 第二轮止血继续把 summary publish 的 distinct in-progress conversation count 也切到 live truth source，避免 maintenance 发布链路把 account detail 已经省下来的 SQLite 扫描又带回来。
- 2026-07-02: 健康与事件 tab 的 `includeRecentActions` 前端 query 编码改为 `true/false`，避免 Rust/Axum bool 反序列化拒绝 `1` 并返回 400。
- 2026-07-03: 账号活动总览从 records tab 迁移到 overview tab；records tab 收敛为调用表格本体，并改为固定页大小的滚动追加加载，减少统计图表与日志列表之间的视觉和请求职责混杂。
- 2026-07-03: overview 顶部账号基础属性从独立卡片网格压缩为单条元数据带，优先把首屏空间留给使用率窗口与账号活动总览。
- 2026-07-03: running runtime snapshot 从 batch writer 占位落库继续降级为进程内 runtime store。账号详情和 account activity 的 in-flight 展示通过 HTTP overlay 共享同一内存态，terminal DB 事实仍是最高优先级。
- 2026-07-03: terminal invocation 记录进入 SQLite write controller 后，账号详情 records/current 读面接受短暂最终一致窗口；SSE 与 runtime-store tombstone 继续避免 open-resync 闪断，DB terminal 行一旦落库仍覆盖内存态。
- 2026-07-10: 账号详情 records tab 增加后端锚点窗口与双向惰性分页；历史定位冻结 snapshot 并暂停 SSE，避免前端逐页扫描与实时插入改变目标索引。
- 2026-07-10: 双向历史窗口改为携带 `snapshotId + anchorId`，由后端复现定位时的 runtime overlay，确保 prepend/append 与初始锚点采用同一稳定分页序列。
- 2026-07-10: 调用 ID 改为单行完整展示，并用账号详情专用列宽回收用时、输入与输出列空间；定位高亮收敛为不改变布局的单层视觉状态，避免默认焦点轮廓叠加。
- 2026-07-13: 桌面账号详情抽屉从内容驱动的最大宽度改为 `90rem` 上限内的确定宽度，避免异步加载与 tab 内容差异引发横向抖动；紧凑视口页面化语义保持不变。
- 2026-07-17: 线上回归确认共享详情抽屉壳层仍带着基础 `w-full`，导致 `drawer-shell--detail-wide` 在最终样式里输给 utility 顺序，宽度退化成几乎铺满视口。宽详情 modifier 已改为更高优先级选择器，并补上 Storybook 像素级宽度断言，确保 `90rem` 契约真正生效。
