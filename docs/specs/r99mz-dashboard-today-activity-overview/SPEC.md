# Dashboard：把“今日”并入“活动总览”，并为今日新增分钟级柱状 / 累计面积图（#r99mz）

## 状态

- Status: 已实现，PR #321 收敛中
- Created: 2026-04-08
- Last: 2026-04-10

## 背景 / 问题陈述

- `#7s4kw` 已把“历史”并入 `活动总览`，但 Dashboard 顶部仍然保留了一块独立的“今日统计信息”卡，页面信息仍被切成上下两个中心区域。
- 主人要求继续收拢：将“今日”也并入 `活动总览`，统一由同一张总览卡承载 `今日 / 24 小时 / 7 日 / 历史` 四段。
- 新的“今日”视图不再复用热力图，而是切换成分钟级图表：`次数` 要展示成功 / 失败分离柱状图，`金额 / Tokens` 要展示今日累计面积图。

## 目标 / 非目标

### Goals

- Dashboard 页面删除独立的顶部 `TodayStatsOverview` 卡，只保留合并后的 `DashboardActivityOverview`。
- `活动总览` 范围切换升级为 `今日 / 24 小时 / 7 日 / 历史` 四段，并新增 localStorage 记忆最近一次访问的范围。
- `今日` 范围顶部嵌入 5 个 KPI；下方图表随统一 metric toggle 切换：`次数` 显示成功正柱 / 失败负柱，`金额 / Tokens` 显示“今日整天 24 小时横轴”的累计面积图。
- `今日 / 次数` 的前后端统计口径必须一致：成功 / 失败柱共享同一时间槽位，`running/pending` 及其瞬时失败元数据不会在初载、live patch 或归档 rollup 中被临时算作失败。
- `今日 / 次数` 的前端绘制必须只消费显式 `inFlightCount`：legacy 中性残差允许继续留在 `totalCount`，但不能再被 `total - success - failure` 反推成 `进行中` 柱。
- `24 小时 / 7 日 / 历史` 维持现有热力图 / 日历形态，仅共享头部 metric toggle，并保持按视图记忆 metric 行为不回退。
- `活动总览` 的非激活范围改为按需挂载与按需请求：默认进入 Dashboard 只加载当前页签，未访问的 `24 小时 / 7 日 / 历史` 不再首屏预取，也不再常驻隐藏面板。
- Dashboard 工作中对话的 prompt-cache 会话工作集必须有界：authoritative 刷新后只保留“当前响应中的 key + 仍有 live record 的 key”，selection 切换或卸载后释放旧工作集。
- prompt-cache 会话图表 / 表格必须按 `outcome` 区分 `success / failure / neutral / in_flight`：blank/null `status` 且缺少失败元数据的历史行保持中性，不得再渲染成错误红色。
- Dashboard KPI 必须能识别卡片宽度不足导致的数值溢出，并自动切换到紧凑记数法（例如 `1.31B`）以保持布局稳定。
- 补齐 Storybook、Vitest、spec 与视觉证据，并按 fast-track 路径收敛到 merge-ready。

### Non-goals

- 不新增统计 API、数据库 schema 或 SSE 协议字段；后端改动仅限于现有 summary / timeseries / rollup 的口径校正与 live/archive 一致性修复。
- 不把 `24 小时 / 7 日 / 历史` 的可视化统一重写成折线 / 面积图；它们继续沿用现有热力图 / 日历方案。
- 不把每个范围的 metric 选择写入 localStorage；本轮只持久化最近一次访问的范围。

## 范围（Scope）

### In scope

- `web/src/pages/Dashboard.tsx`：移除独立今日卡，只保留合并后的总览与工作中对话区。
- `web/src/components/DashboardActivityOverview.tsx`：新增 `today` 范围、范围持久化与嵌入式今日面板。
- `web/src/components/TodayStatsOverview.tsx`：支持嵌入模式，便于在“今日”页签内复用 KPI 行。
- `web/src/components/DashboardTodayActivityChart.tsx`：新增分钟级今日图表组件，负责柱状 / 累计面积两种模式。
- `web/src/hooks/useTimeseries.ts` 与相关 API typings：修正 live patch、in-flight seed 分页快照与 remount/live delta 去重，避免同一 invocation 在今日次数图中重复累计。
- `src/api/slices/prompt_cache_and_timeseries.rs`、`src/api/slices/invocations_and_summary.rs`、`src/stats/mod.rs`、`src/maintenance/archive.rs`：统一 `running/pending`、legacy `http_200` 与 archive rollup 的 success/failure/TTFB 统计口径。
- `web/src/components/*.stories.tsx`、相关 Vitest：补齐四段切换、今日图表、页面级 Dashboard 的稳定 Storybook 与回归覆盖。
- `src/tests/slices/pool_failover_window_*.rs`：补齐 summary / timeseries / hourly rollup / archive 相关回归测试。
- `docs/specs/README.md` 与本 spec：登记新 follow-up，并承载后续视觉证据。

### Out of scope

- `src/` 下任意后端实现、数据库 schema 或 API 合约变更。
- 历史半年日历之外的更长期统计范围或额外 summary API。
- 任何与本轮无关的 Dashboard 工作中对话卡片、抽屉或其他页面重排。

## 验收标准（Acceptance Criteria）

- Given 打开 Dashboard，When 查看页面顶部，Then 不再存在独立的 `today-stats-overview-card` 外层卡片，“今日”能力只出现在 `活动总览` 内。
- Given 查看 `活动总览` 范围切换，When 进入页面，Then 显示 `今日 / 24 小时 / 7 日 / 历史` 四段；首次进入默认 `今日`，之后优先恢复最近一次访问的范围；localStorage 值非法时回退到 `今日`。
- Given 处于 `今日` 视图，When 查看总览内容，Then 顶部显示 5 个 KPI、下方显示一张分钟级图表；`24 小时 / 7 日` 仍显示既有 KPI + 热力图；`历史` 仍只显示半年日历。
- Given `今日` 视图切到 `次数`，When 查看图表，Then 每个时间桶的成功 / 失败柱共享同一 X 槽位，成功柱位于 0 轴上方、失败柱位于 0 轴下方；当 bucket 带有显式 `inFlightCount > 0` 时，图表会在同一正半轴槽位额外绘制中性 `进行中` 柱，并且 tooltip 至少给出成功 / 失败 / 进行中 / 总数；仅因为 `totalCount > successCount + failureCount` 产生的 legacy 中性残差不得再被反推成 `进行中` 柱。
- Given `今日 / 次数` 存在 `running/pending` live row、带失败元数据的临时态，或 legacy blank/null status 行缺少失败元数据，When 页面初载、silent refresh、SSE 多次更新或数据归档后重新读取，Then 这些非终态 / 中性 legacy 记录都不会被计入 failure；只有终态失败或带明确失败元数据的 legacy 行才会进入失败柱与失败汇总。
- Given prompt-cache 会话图表或表格读到 blank/null `status` 且缺少失败元数据的历史行，When 前端按 authoritative 数据或 live patch 渲染该记录，Then 该记录会保持 `neutral` 语义与中性色，而不是被渲染成失败红色或进行中态。
- Given in-flight seed 需要跨多页抓取 `running/pending` 记录，When 前端顺序拉取后续页，Then 所有页都复用第一页返回的 `snapshotId`，不会因底层集合变化而重复或漏算 seeded live delta。
- Given `今日` 视图切到 `金额` 或 `Tokens`，When 查看图表，Then 图表切换为“今日整天 24 小时横轴”的累计面积图；未来分钟不渲染，缺失分钟补 0 以保持曲线连续。
- Given 在四个范围间切换 `次数 / 金额 / Tokens`，When 来回切换范围，Then 每个范围仍保留各自上次选中的 metric。
- Given 默认进入 `/dashboard`，When 页面首次完成 hydration，Then 仅当前 active range 对应的数据请求会首屏触发，未访问的隐藏范围不会提前发起 summary / timeseries 请求。
- Given 已切到其他 prompt-cache selection 或离开页面，When 旧 selection 的 authoritative / live 数据不再属于当前工作集，Then 旧 key 会被释放，不再随着历史唯一 `promptCacheKey` 数量单调增长。
- Given KPI 卡片的完整数字在当前宽度下会溢出，When 组件完成布局测量，Then 卡片数值会自动切换为紧凑写法（如 `1.31B`），并通过 tooltip 保留完整值。
- Given 运行前端验证命令，When 执行 `cd web && bun run test && bun run build && bun run build-storybook`，Then 命令通过。

## 非功能性验收 / 质量门槛（Quality Gates）

### Visual / UX

- `今日` KPI 必须嵌入在总览内部，不新增重复 panel 层级，也不重新引入顶部独立今日卡。
- `次数` 柱状图要清晰区分成功 / 失败语义，失败必须保留错误色；`金额 / Tokens` 面积图需要保持累计阅读语义。
- KPI 数值在桌面单行布局下不得撑破卡片；超长数字应优先收敛为紧凑记数法而不是溢出裁切。
- `历史` 继续沿用 `#7s4kw` 的半年日历外观，不重新引入重复标题 / 时区说明或月份标签重叠。

### Testing

- Frontend targeted tests:
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor/web && bunx vitest run src/components/DashboardTodayActivityChart.test.tsx src/components/PromptCacheConversationTable.test.tsx src/lib/promptCacheLive.test.ts src/hooks/useTimeseries.test.ts src/lib/api.test.ts`
- Backend targeted tests:
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test timeseries_and_summary_do_not_treat_running_rows_with_failure_metadata_as_failures -- --nocapture`
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test prompt_cache_last24h_requests_keep_null_status_rows_neutral -- --nocapture`
- Storybook build:
  - `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor/web && bun run build-storybook`

### Quality checks

- `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor/web && bun run test`
- `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor/web && bun run build`
- `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor/web && bun run build-storybook`
- `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test fetch_invocation_summary_normalizes_top_level_success_and_failure_counts -- --nocapture && cargo test hourly_backed_summary_replays_pre_cutoff_full_hour_live_rows_after_rollup_cursor -- --nocapture && cargo test timeseries_hourly_backed_repairs_stale_archived_rollup_counts_before_querying -- --nocapture && cargo test timeseries_and_summary_do_not_treat_running_rows_with_failure_metadata_as_failures -- --nocapture && cargo test prompt_cache_last24h_requests_keep_null_status_rows_neutral -- --nocapture && cargo check`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/r99mz-dashboard-today-activity-overview/SPEC.md`

## 计划资产（Plan assets）

- Directory: `docs/specs/r99mz-dashboard-today-activity-overview/assets/`
- In-plan references: `![...](./assets/<file>.png)`
- Visual evidence source: Storybook canvas / docs（以稳定 mock 为准）

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 follow-up spec 并登记 `docs/specs/README.md`。
- [x] M2: Dashboard 页面移除独立今日卡，并将总览范围扩为四段。
- [x] M3: 实现 `DashboardTodayActivityChart` 与 `TodayStatsOverview` 嵌入模式，接入今日 summary / timeseries。
- [x] M4: 补齐 Dashboard / ActivityOverview / Today chart / Today KPI 的 Storybook 与 Vitest 覆盖。
- [x] M5: 完成本地全量验证与视觉证据归档。
- [ ] M6: fast-track 推进到 merge+cleanup。

## 方案概述（Approach, high-level）

- 复用现有 `useSummary('today')` 与 `useTimeseries('today', { bucket: '1m' })`，不动后端 API，仅在前端把“今日”作为总览的第四个内嵌视图；今日 KPI 与图表都保持“今日”语义，但图表横轴扩展为整天 24 小时。
- `TodayStatsOverview` 通过 `showSurface / showHeader / showDayBadge` 拆成可复用内容层，使它既能作为独立卡，也能作为总览内嵌 KPI 区块。
- `DashboardTodayActivityChart` 负责将分钟序列补齐到“本地自然日 00:00 -> 23:59”的完整横轴，`次数` 模式用重叠 category slot 的正负柱对齐成功 / 失败语义，`金额 / Tokens` 模式将每分钟增量累积为面积图；当前时刻之后的未来分钟只保留横轴空间，不渲染柱 / 面积。
- `DashboardActivityOverview` 继续保留按范围记忆 metric 的行为，并新增最近访问范围的 localStorage 恢复；非法或不可用值统一回退到 `today`。
- `DashboardActivityOverview` 的各范围面板改成只在 active range 时挂载，并把对应 summary / timeseries 请求下沉到面板内部，避免隐藏页签常驻 hook / timer / 请求。
- `usePromptCacheConversations` 通过 bounded history + live-record pinning 维护当前工作集；authoritative 刷新、selection 切换与卸载都会主动裁剪旧 key，防止长时间停留时因历史 churn 导致内存累积。
- `useTimeseries` 对同一 invocation 的 live delta 采用“减旧加新”，并在 in-flight seed 跨页抓取时固定第一页 `snapshotId`；今日图表只消费后端显式返回的 `inFlightCount`，不再把 legacy neutral residual 从 `totalCount` 反推成进行中；后端 summary / timeseries / archive rollup 统一把非终态从 failure 口径中排除，同时让 blank/null legacy status 在 prompt-cache 与今日图表里保持 neutral outcome。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：分钟级图表点数明显多于现有热力图，若 Storybook / build 使用不稳定的时间源，会导致快照或视觉证据难以复现；因此必须使用固定 mock 时间轴。
- 风险：`今日` 视图现在默认显示 KPI + 图表，如果 `TodayStatsOverview` 嵌入模式仍保留独立标题，会与总览标题重复；本轮通过隐藏内层 header 避免重复语义。
- 风险：localStorage 恢复若未做白名单校验，会把历史无效值带入初始渲染；本轮必须在 helper 层做硬回退。
- 假设：`今日` 页签中的 KPI 与图表都按浏览器本地时区自然日处理；图表需要铺满当天 24 小时横轴，而不是只渲染到当前时间。
- 假设：视觉证据继续采用 Storybook 稳定 mock，不截真实线上数据页面。

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

## Visual Evidence

- Storybook覆盖=通过
- 视觉证据目标源=storybook_canvas（mock-only）
- Validation: `cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor/web && bunx vitest run src/components/DashboardTodayActivityChart.test.tsx src/components/PromptCacheConversationTable.test.tsx src/lib/promptCacheLive.test.ts src/hooks/useTimeseries.test.ts src/lib/api.test.ts && bun run test && bun run build && bun run build-storybook`；`cd /Users/ivan/.codex/worktrees/1918/codex-vibe-monitor && cargo test fetch_invocation_summary_normalizes_top_level_success_and_failure_counts -- --nocapture && cargo test hourly_backed_summary_replays_pre_cutoff_full_hour_live_rows_after_rollup_cursor -- --nocapture && cargo test timeseries_hourly_backed_repairs_stale_archived_rollup_counts_before_querying -- --nocapture && cargo test timeseries_and_summary_do_not_treat_running_rows_with_failure_metadata_as_failures -- --nocapture && cargo test prompt_cache_last24h_requests_keep_null_status_rows_neutral -- --nocapture && cargo check`
- Story id: `dashboard-dashboardtodayactivitychart--count-bars-dense-pairing`
- Scenario: `今日 / 次数` 高密度 minute bucket，对齐验证 success / in-flight / failure 共用同一时间槽位并围绕 0 轴展开。
- Evidence note: 验证柱子不再左右错位；`running/pending` 与其临时失败元数据不会把 failure 柱短时拉长后再回落；图表只会为显式 `inFlightCount` 绘制中性 `进行中` 正柱，legacy neutral residual 不再被误画成进行中；本轮 rebuilt Storybook 已重新绑定到最新本地验证 head。
- Live patch note: 最新收口额外验证了 `running/pending` seed 的跨页同快照一致性，以及“新到达但同桶”的 settled 记录不会错误吞掉旧 placeholder；anonymous placeholder 只允许回收 authoritative `snapshotId` 之前的同桶记录，而 authoritative refresh 会把仍在 TTL 内的 tracked live deltas 合并回 fresh response；当匿名 placeholder 独占 bucket 时，终态 SSE 也会把 provisional token/cost 直接修正到最终值；`current-day-local` 模式仅抓取当前日 bucket 的 in-flight seed，不再为长范围日历扫描整段窗口。
- Repair note: 最新收口额外验证了 all-time summary repair 在 mixed preserve 场景下会补回被 boundary archive 重放清空的历史 live rows，并确保“只缺 failure replay marker”的归档回填不会误删已有正确的 `invocation_rollup_hourly` 总数；同时历史 hourly-backed timeseries / summary / failure 读取 archived rollup 前会先 refresh rollup，再冻结同一个 `snapshotId`，并对 `rollup_live_cursor < id <= snapshotId` 的 full-hour tail 做 exact replay，不再在升级后继续暴露陈旧 failure counts。
- Prompt-cache note: blank/null `status` 且缺少失败元数据的历史 prompt-cache 记录现在会带 `outcome=neutral`，keyed chart 与表格统一使用中性色呈现，不再误标成 error 红线。
- 聊天回图=已展示（本轮使用本地裁剪后的 Storybook canvas 截图完成 owner review）
- 证据落盘=未落盘（本次未提交新的截图文件，避免在未获主人截图提交授权前把 refreshed capture 推上远端）
- Stale evidence handling: 本节已移除旧的静态图片引用，避免在本轮 `今日 / 次数` 语义变更后继续保留过期截图引用。
