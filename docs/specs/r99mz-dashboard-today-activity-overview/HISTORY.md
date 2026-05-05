# Dashboard：把“今日”并入“活动总览”，并为今日新增分钟级柱状 / 累计面积图 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/r99mz-dashboard-today-activity-overview/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-08: 创建 follow-up spec，冻结“今日并入活动总览 + 四段切换 + 今日分钟柱状 / 累计面积图 + merge-ready 收口”的范围与验收标准。
- 2026-04-08: 已完成 Dashboard 页面重排、今日 KPI 嵌入、分钟级图表组件、范围持久化，以及相关 Vitest / Storybook 入口补齐。
- 2026-04-08: 完成全量前端验证与 Storybook 视觉证据归档，并修正今日 `次数` 图中失败柱错误堆叠到正半轴的问题，确保失败柱始终以 0 轴为基线向下绘制。
- 2026-04-08: 为 PR 收口修复跨平台午夜时间格式差异，强制分钟轴午夜显示为 `00:00`，并将今日图表数据构建逻辑拆出组件文件以满足 `react-refresh/only-export-components` lint 约束。
- 2026-04-08: 根据 review-proof 修复 `today + 1m` 长驻会话跨午夜不自动刷新旧日数据的问题；今日视图现在会在本地下一次自然日边界强制静默重拉，并把本地补丁窗口约束回当前自然日。
- 2026-04-09: 为 Dashboard 长时间放置崩溃问题补充前端性能硬化：`DashboardActivityOverview` 改为按需挂载 / 按需请求非激活范围，`usePromptCacheConversations` 与 prompt-cache history 改成仅保留当前工作集，并补齐高 churn / selection 切换回归测试。
- 2026-04-09: 修复 Dashboard KPI 数值溢出；当卡片宽度不足以容纳完整值时，前端会自动切换到紧凑记数法（如 `1.31B`），并保留完整值 tooltip。
- 2026-04-09: 修正 `今日` 页签分钟图的时间语义：保留“今日”自然日范围，但将横轴扩展为当天完整 24 小时；当前时刻之后的未来分钟不渲染，从而避免图表只占用 `00:00 -> 当前时间` 的前半段宽度。
- 2026-04-09: 刷新 Storybook 证据夹具与截图，确保 `今日 / 金额` 图的累计终值与 KPI 总成本一致，不再出现 `US$539.42` KPI 对应 `US$58` 曲线终点的错图。
- 2026-04-10: 修复 `今日 / 次数` 柱状图的双系列对位与 live failure 口径：成功 / 失败柱现在通过重叠 category slot 共用同一时间槽位并围绕 0 轴对称渲染；前后端 live 聚合同时排除 `running/pending` 的临时失败计数，避免失败柱短时异常拉长后再回落。
- 2026-04-10: 根据 fresh review 继续补齐稳定性修复：前端 in-flight seed 分页改为复用第一页 `snapshotId`，避免多页 `running/pending` 抓取在高 churn 下重复或漏算；同一 invocation 的 live patch 继续维持“减旧加新”而不是反复叠加。
- 2026-04-10: 对齐 authoritative/live/archive 三条统计路径：`src/api/slices/prompt_cache_and_timeseries.rs`、`src/stats/mod.rs` 与 `src/maintenance/archive.rs` 统一把带失败元数据的 `running/pending` 排除在 failure 汇总之外，并让 structured legacy `http_200` failure 不再误入 archived success-like TTFB / pruned-success 判定。
- 2026-04-10: 根据 fresh review 继续收口 legacy 空状态语义：blank/null `status` 且缺少失败元数据的历史行现在保持中性，不再在 summary / timeseries / archive rollup 中被误算为 failure；只有带明确失败元数据的 legacy 行才会保留失败统计。
- 2026-04-10: 根据 fresh review 继续收口本地 live patch 稳定性：`useTimeseries` 现在把近期终态 delta 连同其受 TTL / 上限约束的去重元数据一起写入 remount cache，并在复水后继续吸收 duplicate SSE；同时活跃会话里的 tracked delta 仍会按 TTL / 上限裁剪，避免长时间停留页面时 `liveRecordDeltaRef` 单调增长。
- 2026-04-10: 根据 round16 review 继续收口 `今日 / 次数` tooltip 与 all-time summary repair：当 minute bucket 含有 `running/pending` 残差时，tooltip 现在会显式展示 `进行中`，不再让 `总数` 与成功/失败小计失配；同时 summary repair 仅在 materialized 归档文件真实缺失时保留既有 rollup 历史，避免已 prune 的旧归档触发全量重建失败，而文件仍在时继续重放归档以修正陈旧 failure 计数。
- 2026-04-10: 为清除 PR freshness gate，同步最新 `main` 到当前修复分支，并补齐 `PromptCacheConversationsQuery` 新增分页字段在 prompt-cache 回归测试里的构造参数；本 spec 的功能范围与验收口径保持不变，验证基线刷新到同步后的最新 head。
- 2026-04-10: 根据 fresh review 继续收口 remount-cache 与 mixed materialized archive repair：`useTimeseries` 的 silent refresh 现在只回填近期终态 delta 触达的 bucket，并保留 TTL/上限约束内的 settled delta 去重记忆，避免复水后 duplicate settled SSE 再次叠加；summary repair 在 mixed preserve 路径下按 bucket/source 仅清一次既有 rollup，再跨归档批次重放现存 materialized archive，避免旧 failure 值无法修复或同桶多批次重放时发生双算/漏算。
- 2026-04-10: 根据 fresh review 最后一轮阻塞项继续收口：mixed preserve repair 现在会对所有需要重放的现存 archive（不区分 materialized / non-materialized）按 bucket/source 先清旧值再重建，避免部分 repair 重试把旧 rollup 累加成双算；`useTimeseries` 的 silent refresh 也会跳过已滑出新 `rangeStart/rangeEnd` 的 settled bucket，不再把窗口左边界外的旧点短暂塞回图里。
- 2026-04-10: 根据 fresh review 最后一轮继续收口前端 live seed：`running/pending` 的 seed 现在统一复用同一个第一页 `snapshotId`，避免状态在两次分页快照之间迁移时被漏抓；匿名 in-flight placeholder 只允许被“创建于 authoritative load 之前”的同桶记录回收，避免新到达的同桶 invocation 错吞旧 placeholder 并把分钟柱长期低估。
- 2026-04-10: 根据 fresh review 继续收口 archived all-time summary repair：mixed preserve 路径会把被 archive 重放清空过的 boundary bucket 内、且 `shared_live_cursor` 之前的 live rows 重新灌回 hourly rollup，避免 archive/live 同小时交界在 repair 后丢失已落盘的 live 计数；仅缺失 failure replay marker 的归档回填现在只修复 failure 侧 replay 状态，不再误删已有正确的 `invocation_rollup_hourly` 总数。
- 2026-04-10: 根据 latest review 继续收口历史 hourly 视图与今日图表呈现：`/api/timeseries`、hourly-backed summary 与 failure rollup 读取 archived hourly 数据前都会先触发同一条 summary repair/backfill 路径，避免升级后长范围图表继续读取陈旧 failure counts；同时 `今日 / 次数` 图会把 `running/pending` 残差直接画成中性 `进行中` 正柱，不再只藏在 tooltip 里；anonymous placeholder 只允许消费 authoritative `snapshotId` 之前的旧记录，而 authoritative refresh 会把仍在 TTL 内的 tracked live deltas 合并回 fresh response，避免静默重载期间被新 SSE 或本地时钟漂移打出双算。
- 2026-04-10: 根据 merge 前最后一轮 review 继续收口本地 live patch：当匿名 in-flight placeholder 独占 bucket 且 authoritative 数据已带 provisional token/cost 时，终态 SSE 现在会把该 bucket 直接修正到最终 token/cost，而不是继续停留在 provisional 值；`current-day-local` 的 seed 抓取也缩到“当前自然日 bucket”本身，不再在 `1d bucket + 长范围` 视图里分页扫描整段历史窗口的 `running/pending` 记录。
- 2026-04-10: 根据 fresh review 最后一轮继续补齐 authoritative/live 对账：hourly-backed summary / timeseries / failure 读取在 rollup refresh 之后统一冻结同一个 `snapshotId`，再对 `rollup_live_cursor < id <= snapshotId` 的 full-hour tail 做 exact replay，确保 archived rollup 与 live rows 落在同一 cutoff；`fetch_invocation_summary` 也把 legacy `http_200` success-like 行重新计入 `success_count`，避免 records summary undercount。
- 2026-04-10: merge-path freshness sync 实际落到当前收敛 head 后，再次完成 `web` 全量 `test/build/build-storybook` 与 targeted cargo 回归；本 spec 与 `docs/specs/README.md` 同步刷新到这次 mainline 兼容收口后的最新事实，不扩展功能范围。
- 2026-04-10: 根据 fresh review 新一轮阻塞项继续收口 neutral / in-flight 语义：`今日 / 次数` 图现在只消费显式 `inFlightCount`，不再把 `totalCount - successCount - failureCount` 的 legacy 中性残差误画成 `进行中`；prompt-cache blank/null 历史行新增 `outcome=neutral`，会在 keyed chart / 表格里保持中性色而不是错误红色；前后端对应测试、全量 `web` 验证与 targeted cargo 回归已刷新到当前本地 head。
- 2026-04-10: 根据最新 fresh review 继续收口 live prompt-cache 失败呈现：`resolvePromptCacheInvocationOutcome` 现在会优先尊重 `errorMessage` / `downstreamErrorMessage` / `failureKind` 等显式错误元数据，再决定 success/neutral fallback，避免刚结算但 `failureClass` 尚未回填的 live 记录在下一次 authoritative refresh 前短暂误显示为成功或中性。
- 2026-04-10: 根据最新 fresh review 再补齐 `failure metadata > in_flight` 优先级：无论 authoritative `last24hRequests` 还是 live SSE merge，只要 `running/pending` 行已经带有明确失败元数据，就会立即产出 `outcome=failure` 并保持失败色；只有既无失败元数据、又仍处于 `running/pending` 的记录才继续显示为 `in_flight`。
- 2026-04-10: 根据最新 fresh review 再补 authoritative prompt-cache request-point 对账：`last24hRequests` authoritative refresh 现在不仅识别 `failureClass`，也会把 `errorMessage` / `downstreamErrorMessage` / `failureKind` 视为显式失败元数据，因此 `running/pending` 但已带错误文本的记录不会再被 authoritative 刷新错误降级回 `in_flight` 或 `neutral`。
- 2026-04-10: 根据最新 fresh review 再统一 downstream-only failure metadata：`INVOCATION_RESOLVED_FAILURE_CLASS_SQL` 的 legacy `success / http_200 / blank-status` 快路径现在会同时检查 `downstreamErrorMessage`，避免仅靠 downstream 错误文本支撑的旧记录被误判成 success/neutral；前端 `useTimeseries` 的 live classifier 也把 `downstreamErrorMessage` 视为显式失败元数据，因此 today minute chart 不会再把这类实时记录短暂画成绿色。
- 2026-04-10: 根据最新 fresh review 再补 exact/live aggregate 读取：`query_invocation_aggregate_records_from_live_range*` 现在直接投影 `INVOCATION_FAILURE_KIND_SQL` 与 `INVOCATION_RESOLVED_FAILURE_CLASS_SQL`，并按 resolved class 重新计算 `is_actionable`，所以 current/live summary、today exact bucket 与 full-hour tail replay 不会再漏掉仅存在于 `payload.downstreamErrorMessage` 的 legacy 失败元数据。
- 2026-04-10: 根据最新 fresh review 再统一 `completed` success-like 口径：`resolve_failure_classification`、`INVOCATION_RESOLVED_FAILURE_CLASS_SQL`、recent/exact summary helper、hourly rollup success-like 判定与 `query_combined_totals` 现在都会把无失败元数据的 `status='completed'` 视为成功，从而让 Dashboard summary、今日次数图、TTFB 样本与 invocation summary 不再把常见成功终态漏算成 failure 或 missing sample。
- 2026-04-10: 为修复 PR 收敛阶段暴露的 historical range regression，hourly-backed `timeseries` / duration-summary / failure-summary 读取 archived 数据前改为 best-effort rollup refresh：若只遇到“archive manifest 已存在但文件被移除”的缺失批次，会复用当前已存在的 hourly rollup 返回范围结果而不是直接 500；但 `window=all` 的严格 summary repair 仍保留 missing archive fail-fast，不会把 repair marker 误标完成。
- 2026-04-11: 为解除 `#321` 合并后的发布阻塞，测试基座现在会为使用默认 `target/archive-tests` / `target/proxy-raw-tests` / `target/xray-forward-tests` 的 stateful 后端用例自动分配按 `db_id` 隔离的运行目录，避免并行 `cargo test --all-features` 时因共享归档路径互相覆盖而触发 all-time summary / historical range 相关用例的偶发失败。
- 2026-04-28: 修正 `今日 / 趋势` 图表的降采样速率口径：10 分钟趋势点现在展示有效分钟内的 TPM / 消费速率均值，不再把 10 分钟 token 或 cost 总量直接标成每分钟速率；新增 `DashboardTodayActivityChart` 回归测试并用 Storybook canvas 复核右轴量级。
