# Dashboard 工作区卡片双视图与上游账号活动聚合 演进历史（#z6ysw）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-07-22：确认旧的 `full_range_preview_rows` 已退出健康主链后，7d 残留热点进一步收缩到 `usage_breakdown` 自身；因此冻结新的实现约束为“补内部 `model + reasoning` hourly breakdown rollup，并让 summary / dashboard / upstream-account 共用同一 `rollup + exact tail + archive hole fallback` builder”，而不是继续在各路 handler 上重复 raw aggregate 优化。
- 2026-07-22：review 收口时补充确认 rollout 迁移约束：新的 `usage_breakdown` target 不能复用 legacy account rollup 的 materialized shortcut。对已上线版本留下的老 archive materialized batch，缺 breakdown rows 时必须 reopen backlog 做历史回放，而不是继续输出看似健康但实际漏算的 `7d` usage breakdown。
- 2026-07-20：Dashboard 7d 总览残留慢点确认不只来自 account activity；同轮把 `/api/stats/summary` 与 `stats.summary.current` 的 open-range `usage_breakdown / non_success_tokens` 改成 aggregate-only builder，移除 raw preview 全窗扫描与 live-id overlap 扫描，避免 overview bundle 在 `fetchDashboardActivity("7d") + fetchSummary("7d")` 下继续把 SQLite 压力打回读侧。
- 2026-07-20：落实 Dashboard 读侧 CPU 修复。`upstream-account-activity` 与 dashboard full path 不再对完整 `7d` range 做 persisted `running/pending` preview 扫描，recent 改为“每账号 bounded candidate + hydration + runtime/live overlay”；同日把 non-`yesterday` dashboard 基础快照缓存收敛为稳定请求参数选择键与 `5s` TTL，并补齐 `route / builder / purpose / limit / cache_ttl_ms / cache_entry_age_ms / cache_entry_count / in_flight_count` 遥测，便于线上继续判定是缓存未合并还是 preview 仍过宽。
- 2026-07-20：收紧 Dashboard 对话卡片 `当前调用 / 上一条调用` 的可见用量行。此前槽位常驻展示 `IN / CW / C / O / T / $`，在窄卡里噪声偏高；本轮冻结为只显示 `Hit / Token / $` 三项，其中 `Hit` 口径固定为 `cacheInputTokens / totalTokens`，成本值复用金额主题 accent，而完整 token/cost/reasoning 明细继续保留在 hover/title，避免压缩卡面时丢失诊断能力。
- 2026-07-21：继续收紧 Dashboard 单次调用摘要合同。上游账号 recent 行不再保留独立的 `IN / CW / C / O / T` 正文格式，改为与对话卡片统一复用 `Hit / Token / $` 三字段；同时把 `Hit` 与成本的默认/warning/error 阈值着色收口到共享 helper，并明确 `90% / 50% / 0.1 / 0.5` 边界值按严格比较停留在较低一档。完整 token/cost/reasoning 明细继续保留在 hover/title/aria。
- 2026-07-22：补齐上游账号卡标题区状态 badge 的真实实现。此前虽然规格已要求移除外层 group chip，但实际 DOM 仍用一个按钮包住多个状态 badge，形成“双层 chip” 视觉；现已改为无外壳容器 + 独立可点击 chip，并补齐对应 Storybook/测试回归。
- 2026-07-19：101 线上 12 小时复盘确认 `dashboard-activity full` 慢点仍主要来自读侧合并不足，而不是内存瓶颈。已冻结两条后续约束：一，non-`yesterday` 基础快照缓存只允许按稳定请求参数 + 短 TTL 合并，不能再把 live runtime 状态或最新持久化行 ID 放进选择键；二，`/api/stats/upstream-account-activity` 不得继续借道 dashboard full snapshot，必须走独立账户活动 builder，并补足 route/builder/preview hydration telemetry 便于下一轮定位残余慢点。
- 2026-07-18：收紧上游账号卡宽屏 `split` header 的 `TPM` 横向预算。此前长 `TPM` 会按完整数字位数直接撑宽右上实时指标区；本轮固定仅 `TPM` 值本体约 `6ch` 宽度预算，超预算后继续复用既有 adaptive compact 缩写链路，`进行中调用 / 消费速率` 与窄卡 `stacked` 路径不变。
- 2026-07-16：Dashboard 工作区与顶部当前态正式并入主应用统一 topic SSE 总线。此前 `dashboardActivityLive + HTTP reconcile/open-resync` 的双轨合同在这里退场，取而代之的是 `dashboard.activity.current` 的 authoritative `snapshot/replay/live`；账号视图、顶部 KPI 与恢复语义统一由 topic cursor / schemaEpoch 驱动。

- 2026-07-17：把 Dashboard 当前态合同从 `last_complete_1m_sma` 切到 `rolling_60s_live_mean`。吞吐型指标 `TPM / 消费速率` 改为最近 60 秒滚动窗口且空窗归零；延迟型指标 `首字用时 / 响应时间` 改为“窗口优先，当前 range 最近有效结果回退”。同时账号卡新增账号级当前态延迟字段，主卡直读当前态，范围均值退回 tooltip/详情。
- 2026-07-15：收敛 Dashboard 顶部实时 KPI 的 owner-facing 合同。此前 `TPM / 消费速率 / 首字用时` 混用了完整范围平均、前端 timeseries 快照和 `modelPerformance` 总计，导致同屏 summary 与账号标题存在口径漂移。本轮固定为后端统一计算的 `last_complete_1m_sma`：`TPM` 继续沿用成功且已计费的合格 token 分子，`消费速率`、`首字总耗时` 与 `响应时间` 全部取最近 1 个完整分钟 bucket，严格空桶不回填旧值；`z9h7v` 同步收窄为完整范围模型性能明细规范。

- 2026-07-15：收紧 Dashboard `上游账号` 视图的 background refresh 表达。旧实现把状态渲染成带占位的头部 chip，导致 idle 时仍在“当前活动账号”左侧保留固定空白；本轮改为桌面端非 badge 的 spinner + `刷新中` 文本、移动端 spinner-only，并保留既有 `300ms` 延迟显示与 `600ms` 最短可见时长。

- 2026-07-15：调整 Dashboard 工作区排序语义。此前 spec 把 `cost / tokens` 定义为正序，实际运营更需要和 `createdAt / lastInvocation` 一样按倒序扫描，因此本轮将 4 种 workspace sort 全部统一为倒序；同时冻结 `未分配上游账号` 聚合项在账号视图中始终后置，避免它凭借高失败量或空账号流量抢占已分配账号前排。

- 2026-07-15：修正 Dashboard 工作区头部控制条的实现漂移。规范要求头部保持紧凑控制条：左侧仅保留 `对话 / 上游账号` tabs，右侧收口为 `当前对话` badge 与排序按钮；此前实现把顺序错排成 `badge -> tabs -> 排序`，导致视觉节奏偏离验收基线。本轮将组件顺序恢复为 spec 基线，并同步刷新视觉证据。

- 2026-07-13：确认账号视图数秒首屏等待来自 `includeAccounts=true` 路径对活动账号逐个 `await` recent query；冻结为按需激活、汇总优先、快照绑定的单次批量 recent 补齐。UI 必须区分首次骨架、真实空态、后台刷新与 recent 局部失败，不能用全页等待或错误空态掩盖慢读路径。

- 2026-07-13：生产诊断确认账号卡把 records SSE 降级成重型 `dashboard-activity` HTTP 重查通知，慢查询叠加 5 秒节流可造成超过 10 秒的过时状态；改为后端内存运行态生成版本化 `dashboardActivityLive` 快照，前端只合并权威结果，HTTP 保留历史校准职责。

- 2026-06-26：创建 active spec，冻结 Dashboard 工作区双 tabs、账号活动跟随总览 range、以及 `usage` 下 disabled 回退的交互边界。
- 2026-06-26：明确账号视图不是折叠卡、不是四小格，而是单张放大账号卡，上半部摘要、下半部最近 4 条调用记录。
- 2026-06-26：锁定 summary `inProgressConversationCount` / `inProgressRetryConversationCount` 保留 wire name 但改为 invocation-based 语义，owner-facing 文案同步改成“进行中调用 / 重试调用”。
- 2026-06-26：明确账号 tab 只能在首次激活后请求数据，未激活时不参与 SSE / records refresh budget。
- 2026-06-27：收紧账号卡 UI 合同为“紧凑信息卡”，禁止状态说明条与解释性废话常驻显示；请求数 / Token / recent bridge 分解改为卡面只显示色点与数值，完整标签仅在 hover 暴露。
- 2026-06-27：进一步收紧分解摘要常驻态，连单字 / 缩写短标签也不允许可见；卡面仅保留色点与数值。
- 2026-06-27：明确 recent 区标题行右侧统计为例外，需显示完整状态文字，并与左侧标题保持垂直对齐；请求数 / Token 分解继续维持“仅色点与数值”。
- 2026-06-27：锁定账号卡 recent 区必须完整保留 4 条调用记录，且单条 recent 行的信息密度不得低于对话卡片中的调用记录摘要。
- 2026-06-27：桌面宽屏账号卡固定高度从更高版本收敛到紧凑高度，后续新增信息优先通过行内压缩而不是继续增高卡片。
- 2026-06-28：上游账号 recent 行主标识改为“对话短 ID + 请求 ID”，并要求后端 preview 合同补出真实 `promptCacheKey`，避免详情抽屉继续把 `invokeId` 误当对话键。
- 2026-06-28：当 recent 行请求模型与响应模型规范化后仍不一致时，必须在账号卡内同时显示双模型与切换图标；同时统一 compact badge 尺寸节奏，消除同一行内高度不一致。
- 2026-06-28：账号卡顶部改为文本型实时 `TPM / 消费速率` 指标并去掉顶部 `调用`，周期统计收敛为四组，并锁定严格失败口径，避免把“其他非成功”混入失败成本、失败 Token 与失败比率。
- 2026-06-29：上游账号 recent 行把“连续色圆点 + 短码”重构为轻量 identity chip；短码文本成为主识别，颜色改为稳定离散辅助槽位，避免与运行状态灯语义混淆。
- 2026-06-29：生产诊断确认账号卡“首字用时”误接成阶段级上游首字节时延，导致真实秒级总耗时在 owner-facing 卡面被渲染成 `0ms`；因此冻结该卡主值必须回到首字总耗时口径，并要求前后端同时保留显式字段用于平滑兼容。
- 2026-06-29：线上热点复盘后，账号 tab 继续禁止逐条本地 SSE patch，并把 tab 激活态的 refresh/open-resync 统一锁到 `5s`；如果未来要恢复更高频 cadence，必须先拿到慢路径证据证明后端读路径不会再次退化成请求级热扫描。
- 2026-06-29：补充修正 identity chip 槽位算法，明确不能直接对展示短码片段做低位 `% 8` 取槽；改为混合完整 hash 后选离散槽位，避免线上真实短码因低位偏置出现大面积同色聚集。
- 2026-06-30：Dashboard `Working Conversations` 的 5 分钟 head/count 改读 write-side `prompt_cache_working_set_live`，并为 mixed-source 对话保留独立 `ProxyOnly` 聚合槽位，避免 UI 为了代理视图再次回扫 `codex_invocations`。
- 2026-06-30：修正上游账号 recent 行短 ID 的热区语义，明确 identity chip 独立打开对话详情，而整行其它区域继续保留调用详情入口，避免 operator 点短 ID 时误落到 invocation drawer。
- 2026-06-30：补充冻结工作区 tabs 的浏览器侧记忆语义：只持久化用户主动选择的偏好视图；`usage` 下的 `对话` 回退仅为临时降级，不得覆盖上次选择的 `上游账号`，以保证重新进入 Dashboard 或切回支持 range 时能自动恢复。
- 2026-07-03：上游账号四组周期统计从逐段 tooltip 收口为整张统计卡 tooltip；常驻态继续保持紧凑裸数值，完整字段名和值进入结构化浮层，避免小色点分解段产生嵌套触发区域。
- 2026-07-02：账号活动接口补出账号当前 `effectiveRoutingRule`，并将账号卡标题区空位用于只读关键策略徽章；该区域只展示 `主力 / 兜底 / 禁新 / 禁出 / 禁入 / Fast / 并发 / 重试` 等策略信号，不展示普通系统 tag 名称。
- 2026-07-04：成本周期统计中的“失败成本比率”锁定为失败成本占总成本的比例，即 `failureCost / totalCost`；请求失败率继续只属于请求数组，避免失败成本为 0 时成本卡仍显示非零失败成本比率。
- 2026-07-05：账号卡标题区的文本型实时指标修正为 `进行中调用`，口径固定为账号活动接口的 `inProgressInvocationCount`；撤回 Dashboard 账号活动接口中误加的 `activeConversationCount` 依赖，避免把 sticky route 活跃对话数误当作当前调用压力。
- 2026-07-05：运行态调用从笼统 `进行中` 拆成 `排队中 / 请求中 / 响应中`，并要求账号卡所有运行态统计只读后端账号级 live `inProgressPhaseCounts`；recent 列表是展示窗口，不再承担统计事实源职责。
- 2026-07-05：顶部实时 KPI 与上游账号卡片收敛到 `dashboard-activity` 同源快照；`TPM`、`消费速率` 与 `进行中调用` 由账号优先聚合结果求和得到，timeseries 退回趋势图职责，不再作为顶部当前值事实源。
- 2026-07-07：Dashboard 上游账号卡标题区从只读关键策略徽章演进为快捷操作面；状态 badge 只展示异常/注意态并跳转健康事件，齿轮跳转路由设置，快捷策略一律写账号级覆盖且不提供恢复继承，以避免 Dashboard 上下文里出现继承/覆盖的额外决策负担。
- 2026-07-07：Dashboard 上游账号卡快捷操作面补齐 Fast 模式四档切换，沿用账号池已有 `fastModeRewriteMode` 语义并显示为 `不改Fast / 补Fast / 强制Fast / 禁Fast`；该入口继续只写账号级覆盖，避免在 Dashboard 快速操作区引入继承恢复决策。
- 2026-07-07：Dashboard 上游账号快捷策略 chip 的颜色改为按策略意图映射，而不是按“是否激活”统一高亮：`普通 / 不改Fast` 为 neutral，`兜底 / 补Fast` 为 success，`主力 / 强制Fast` 为 primary，`禁新 / 禁Fast / 激活禁出 / 激活禁入` 为 warning。
- 2026-07-08：Dashboard 上游账号卡标题区移除本地 `#<upstreamAccountId>` 编号，只保留齿轮作为账号路由入口；账号名和异常/策略/实时指标已经足够承载当前扫描任务，内部主键不再作为常驻视觉元素。
- 2026-07-08：Dashboard 上游账号优先级快捷入口收敛为纯 `priorityTier` 四态轮换；`禁新` 直接写 `priorityTier=no_new`，不再写独立新对话允许/禁止字段。
- 2026-07-08：Dashboard 工作区对话卡片与上游账号 recent 行的长错误摘要统一冻结为“单行省略 + 共享 tooltip 完整披露”；错误文案不得再撑宽卡片或 row，且交互不再依赖浏览器原生 `title`。
- 2026-07-11：修正长错误载荷仍可经账号双列 grid 的默认最小内容宽度撑开父卡的问题；宽屏 track、账号卡、recent 行与共享错误 trigger 均冻结为可缩小链，并将回归覆盖迁移到现役 `features/dashboard` 渲染树。
- 2026-07-10：生产诊断确认活动总览“进行中调用”虚高来自已终结但未清理的 `pool-via-*` synthetic runtime snapshot；修复锁定在既有 cleanup guard 的生命周期终态收口，不增加查询端年龄过滤，以免掩盖生命周期泄漏或误排除真实长时请求。
- 2026-07-13：修正 Dashboard 账号 recent reconcile 仅从 SQLite 读取调用、遗漏尚未 batch flush 的 runtime running / pending / terminal 记录的问题；recent 候选改为合并 runtime 与 SQLite 并按稳定调用键去重，统计聚合继续以持久化 terminal 事实为准。
- 2026-07-15：修正 Dashboard `warning_success` 紧凑状态位仍走浏览器原生 `title` 的漏网点；对话卡片与上游账号 recent 行统一改用共享 UI tooltip 披露“警告成功 + downstream 诊断”。

## Key Reasons / Replacements

- `#gz5ns` 已冻结 Dashboard 顶部自然日 KPI 语义，但没有覆盖工作区 section 的双视图与账号活动聚合边界，因此需要新的 active topic spec 承接。
- `#t6d9r` 已限制账号详情统计走 account read-model，本 spec 只为 Dashboard 引入“批量账号活动摘要 + recent query”能力，不替代账号详情页的 read-model 责任。
- `#5932d` 曾冻结 Dashboard in-progress 的严格语义，但 owner-facing 视图已从“按对话观察”演进到“按调用观察”，因此本 spec 覆盖 Dashboard summary owner-facing 语义的后续收口。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`

### 2026-07-13 - Workspace card sorting

- Added independent conversation/account sort preferences and a compact keyboard-accessible cycle control.
- Account ordering now uses aggregate conversation creation and invocation timestamps rather than account metadata or truncated recent previews.
