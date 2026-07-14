# Dashboard 工作区卡片双视图与上游账号活动聚合（#z6ysw）

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## 背景 / 问题陈述

- Dashboard 当前“工作中对话”区域只支持按对话查看，无法在同一块工作区里观察“当前活跃的上游账号”及其范围内聚合指标。
- 顶部总览已经拥有 `today / yesterday / 1d / 7d / usage` 的范围切换，但工作区部分没有共享这套状态，导致账号级视图无法与总览范围保持一致。
- Dashboard / account-scoped summary 现有 `inProgressConversationCount` / `inProgressRetryConversationCount` 仍沿用“按对话去重”的旧语义，与 owner-facing 认知中的“进行中的调用数”不一致。
- Dashboard 顶部实时 KPI 与工作区上游账号卡片必须共享同一个当前活动快照；同一屏内可见的 `TPM`、`消费速率` 与 `进行中调用` 不得分别由前端 timeseries rate 和后端 account-activity rate 两套算法生成。
- 上游账号完整快照会为每个账号读取 recent invocation；逐账号串行读取会让账号卡片整体等待数秒，并在激活请求建立前短暂误显真实空态。

## 目标 / 非目标

### Goals

- 把 Dashboard 当前“工作中对话”区域改成右上角 `对话 / 上游账号` 双 tabs，保留现有对话视图行为不变。
- 新增懒加载的 `上游账号` 视图，只展示当前所选 Dashboard 总览范围内有调用的账号，并跟随 `today / yesterday / 1d / 7d` 聚合。
- 提供一个 Dashboard 专用的后端批量读接口，一次返回账号级摘要与最近 4 条调用记录，禁止前端 fanout 账号详情或 `window-usage` 做 N+1 聚合。
- 把 summary 中现有 `inProgressConversationCount` / `inProgressRetryConversationCount` 语义统一改成 invocation-based，并同步更新 owner-facing 文案为“进行中调用 / 重试调用”。
- 新增 Dashboard 专用活动快照读路径，使顶部当前 KPI 与账号卡片使用同一个 `rangeEnd`、同一份 runtime overlay 与同一套账号优先聚合算法。

### Non-goals

- 不把现有 `对话` tab 改成跟随总览范围；它继续保持当前 5 分钟工作集与现有 SSE patch 行为。
- 不把账号视图做成账号池 roster/table 的嵌入版，也不复用账号详情抽屉的整页布局。
- 不支持 `usage` 范围下的账号活动聚合，也不为此新增替代语义。
- 不引入账号卡展开态、二级 tabs、四小格内层卡片或额外 drill-down 交互。
- 不要求趋势图与顶部瞬时 KPI 数值一致；趋势图继续展示 timeseries 历史走势，顶部实时 KPI 以活动快照为事实源。

## 范围（Scope）

### In scope

- `web/src/pages/Dashboard.tsx`：把 `DashboardActivityOverview` 的 range 状态提升为 Dashboard 共享状态，并接线到工作区 section。
- `web/src/features/dashboard/DashboardActivityOverview.tsx`：支持 controlled range 输入，同时保留既有持久化 key 与独立复用能力。
- `web/src/features/dashboard/DashboardWorkingConversationsSection.tsx` 及新增账号视图组件：右上 tabs、badge、usage disabled 回退、账号卡布局与最近 4 条调用记录。
- `web/src/hooks/useDashboardUpstreamAccountActivity.ts` 与 API 层：账号 tab 懒加载、范围跟随、激活态刷新预算。
- `src/api/slices/invocations_and_summary.rs`、`src/api/slices/settings_models_and_cache.rs`、`src/maintenance/hourly_rollups.rs`：新增 `GET /api/stats/upstream-account-activity`，并修正 summary in-progress 语义。
- `src/api/slices/invocations_and_summary.rs`、`src/api/slices/settings_models_and_cache.rs`、`src/maintenance/hourly_rollups.rs`：新增 `GET /api/stats/dashboard-activity`，返回同一次取数的 summary-only 或 summary + accounts 活动快照。
- 相关 Storybook、前后端测试与视觉证据。

### Out of scope

- 调整 Dashboard 顶部总览范围集合本身，或新增新的全局范围枚举。
- 改造 working conversations 的卡片结构、详情抽屉、抽屉路由或 5 分钟工作集筛选逻辑。
- 修改账号详情整页的布局范式。
- 把历史 `usage`、自然日总量、成本或 Token 事实源全部迁入内存。

## 需求（Requirements）

### MUST

- Dashboard 工作区区块右上必须新增 `对话 / 上游账号` tabs，默认保持在 `对话`。
- `上游账号` tab 首次打开前不得发请求；首次激活后才加载，并在 tab 未激活时不参与 SSE/records 刷新预算。
- `上游账号` 视图只展示当前共享 range 内“至少有 1 条调用”的账号；账号标题直接使用 `displayName`。
- `上游账号` 视图仅支持 `today / yesterday / 1d / 7d`；当共享 range 为 `usage` 时，该 tab 必须 disabled，且若当前停留在账号 tab，必须自动回退到 `对话`。
- 账号活动接口必须一次返回每个账号的 `upstreamAccountId`、`displayName`、`groupName`、`planType`、`enabled`、`displayStatus`、`enableStatus`、`workStatus`、`healthStatus`、`syncState`、`lastError`、`lastActionReasonMessage`、`requestCount`、`successCount`、`failureCount`、`nonSuccessCount`、`totalTokens`、`successTokens`、`nonSuccessTokens`、`failureTokens`、`cacheHitRate`、`tokensPerMinute`、`spendRate`、`totalCost`、`failureCost`、`firstByteAvgMs`、`avgTotalMs`、`inProgressInvocationCount`、`inProgressPhaseCounts`、`retryInvocationCount`、`effectiveRoutingRule` 与 `recentInvocations[4]`。
- `recentInvocations` 必须限制在当前所选范围内，按 `occurredAt DESC` 排序，并使用后端 bounded query 返回；尚未完成 SQLite batch flush 的 runtime running / pending / terminal 记录必须参与 recent 候选，与 SQLite 行按 `(invokeId, occurredAt)` 去重后再截断到 `recentLimit`，不得等待后续调用事件才能显示。
- `recentInvocations[]` 必须额外返回真实 `promptCacheKey?: string | null`，供账号卡 recent 行生成稳定的对话短 ID 与详情抽屉 selection。
- 账号卡不是折叠卡，也不是 `2 x 2` 小格子；它是单张放大卡片，桌面宽屏 `>=1660px` 时每行 2 张，其余断点为 1 列。
- 账号卡必须保持紧凑信息卡定位；在桌面宽屏下允许按放大卡呈现，但不得因为固定高度或装饰性留白把视觉效果拉成整页面板。
- 单账号卡标题行必须展示账号名、异常/注意状态 badge 集合、计划/活动状态、固定快捷策略 chip、账号 ID 与路由设置按钮，并把实时主指标 `进行中调用`、`TPM`、`消费速率` 作为文本型行内指标放在同一顶部区域；`进行中调用` 必须来自账号活动接口的 `inProgressInvocationCount`，当值为 `null` 时显示 `—`；标题区还必须用紧凑 chips 拆分展示 `排队中 / 请求中 / 响应中`，数值只取账号活动接口的 `inProgressPhaseCounts`，不得从卡内 `recentInvocations` 推导；不得用卡片型容器展示这些实时指标，且账号卡内不得再渲染 `渠道 xxx / 分组` 或顶部 `调用` 指标。
- 账号卡标题行的状态 badge 集合只显示异常/注意态，不为正常/空闲状态保留占位；集合至少覆盖 `禁用`、`同步中`、`上游拒绝`、`上游不可达`、`需重登`、`限流`、`降级`、`其它异常` 等状态，点击集合必须打开当前账号详情的 `healthEvents` 标签页。
- 账号卡标题行的快捷策略 chip 必须固定展示账号级快速操作入口：优先级、Fast 模式、`禁出`、`禁入`；优先级入口按 `普通 → 兜底 → 主力 → 禁新 → 普通` 轮换，并分别写账号级 `priorityTier=normal|fallback|primary|no_new`；Fast 模式按 `不改Fast → 补Fast → 强制Fast → 禁Fast → 不改Fast` 轮换，并写账号级 `fastModeRewriteMode=keep_original|fill_missing|force_add|force_remove`；`禁出 / 禁入` 分别切换账号级 `allowCutOut / allowCutIn`。Dashboard 快捷入口不得清除账号覆盖或恢复继承。
- Dashboard 快捷策略保存必须使用乐观 UI 与 1 秒 debounce；debounce 窗口内只提交最终值，失败时回滚到最近已提交状态，并在账号卡内暴露可见错误。保存复用 `PATCH /api/pool/upstream-accounts/:id` 的 `routingRule` payload，不新增 mutation endpoint。
- 账号卡右侧必须提供齿轮 icon button；点击后打开当前账号详情的 `routing` 标签页。
- 账号活动接口中的 `tokensPerMinute` 与 `spendRate` 必须使用响应窗口末端最近 5 分钟活跃尾段口径：以当前响应 `rangeEnd` 为 anchor，仅看最近 5 分钟，跳过窗口前置空闲分钟，并分别以第一个有 Token / Cost 的分钟作为有效分母起点；`requestCount`、`totalTokens`、`totalCost`、recent 调用与排序继续使用所选 range 的总量口径。
- 已选中上游账号的 pool running 调用必须在账号活动 live rows、账号卡 `inProgressInvocationCount` / `retryInvocationCount` 与 account-scoped summary 中归属到该账号；当 invocation payload 尚未写入 `upstreamAccountId` 时，可以用同 `invokeId` 的 `pool_upstream_request_attempts.upstream_account_id` 作为读侧 fallback，并且账号级 retry 计数必须基于该 fallback 后的账号重新判定。
- 单账号卡周期统计必须改为四组：`首字用时 + 响应时间`、`请求数 + 成功 / 失败 / 其他`、`成本 + 失败 / 失败成本比率(%)`、`Token + 缓存命中率 / 失败`。前者为主参数，后者为附加参数；成本组里的失败成本比率必须按 `failureCost / totalCost` 计算，不得复用请求失败率。
- 单账号卡四组周期统计必须以整张统计卡作为 hover / focus / click / long-press 的浮层触发区域；浮层顶部展示该卡主字段和值，下方按“当前字段 / 相关数据”分组明确列出字段名和值，不得只展示裸数值。
- 单账号卡四组周期统计的卡内分解段落不得再各自挂载独立 tooltip，避免在整卡 tooltip 内形成嵌套 trigger；recent 区标题行右侧状态分解不受此限制，继续保留自身 hover/title 行为。
- 单账号卡四组周期统计浮层的补充数据最多 3 项，且只能来自账号活动接口已有字段或前端可安全计算值；不得为了 tooltip 新增后端字段、接口或改变聚合口径。
- 四组周期统计与顶部实时指标中的所有数值必须使用 Dashboard 既有滚动数字效果。
- 工作区在 `对话 / 上游账号` tabs 右侧必须显示当前排序名称的循环按钮；点击按 `createdAt -> lastInvocation -> cost -> tokens` 循环。两个视图分别使用独立 localStorage key，首次均默认 `createdAt`，切换视图不得覆盖另一视图的选择。
- 对话的 `createdAt`、`lastInvocation`、`cost` 与 `tokens` 必须全部按倒序排列；缺失时间置后，最终以 `promptCacheKey` 作为稳定次级排序。
- 账号活动响应必须返回 `latestConversationCreatedAt` 与 `lastInvocationAt`。前者取账号关联对话中最新的真实 conversation `createdAt`，后者取账号关联调用的最新 `occurredAt`；不得用账号创建时间或 recent preview 的截断结果替代。账号排序使用这两个时间字段与成本/Token 全部按倒序排列，最终以 `accountKey` / 账号 ID 稳定排序；其中 `isUnassigned=true` 或 `upstreamAccountId=null` 的 `unassigned` 聚合项必须统一排在所有已分配账号之后，再在未分配项内部应用当前倒序规则。
- 排序只能从现有 SSE patch 或账号活动快照派生，不得新增请求或改变 refresh cadence；重排后继续复用既有虚拟列表锚定逻辑。
- 摘要区不得加入低价值说明型文案；“按调用计数，不按对话去重”、“仍在重试链路中的调用”、“账号状态说明条”之类解释性文字不得出现在卡面常驻内容里。
- 请求数、成本与 Token 附加分解摘要在卡面常驻态只显示色点与数值；不得出现任何可见文字标签（包括单字、缩写、短标签）。完整 `label + value` 只通过 hover / title 暴露，不得额外占用版面。
- 账号卡内部所有结构性描边（外框、摘要格子、recent 行、分隔线）必须统一使用低对比中性边框，不得把主题主色、语义色或任意彩色边框用于结构分割；颜色只保留给状态点、数值与徽章等语义元素。
- recent bridge 作为 recent 区标题行右侧统计例外，必须显示完整状态文字；运行态必须拆成 `排队中 / 请求中 / 响应中`，数值来自账号级 `inProgressPhaseCounts`，终态继续使用账号级 `successCount / failureCount / nonSuccessCount`，并与左侧“最近 4 条调用”标题保持同一垂直对齐节奏。
- 单账号卡下半部分必须展示当前范围内最近 4 条调用记录，复用现有紧凑调用行语言，而不是再做卡中卡；4 条记录必须在卡内完整可见，不得依赖展开、滚动或裁切。
- 账号卡内每条 recent 调用记录的信息密度不得低于 Dashboard 对话卡片中的调用记录：至少需要覆盖状态、模型、endpoint、Token 用量摘要，以及 `RQ / UP / ED / TT` 时序摘要。
- Dashboard 工作区 `对话` tab 的 recent/current 调用错误摘要，以及 `上游账号` tab recent 行错误摘要，必须统一保持单行省略；摘要文本本身就是 tooltip trigger，hover / focus / long-press 时使用 UI 库 tooltip 在 trigger 下方优先展示完整错误，除非浮层系统因视口避让自动翻转；不得依赖浏览器原生 `title` 作为最终交互。
- 宽屏上游账号双列 grid 必须使用可缩小 track；账号卡、recent 调用行与错误摘要 trigger 必须组成连续的 `min-w-0` / 最大宽度约束链，确保任意长度的错误载荷都不能扩大 grid track、账号卡或 recent 行。
- 账号卡 recent 调用记录的主标识行必须改为“对话短 ID + 分隔符/图标 + 请求 ID”；其中对话短 ID 固定基于真实 `promptCacheKey` 走既有 working-conversation 哈希与格式化规则，展示值去掉 `WC-` 前缀；请求 ID 显示完整 `invokeId` 并允许单行截断。
- recent 行里的对话短 ID 必须渲染为轻量 identity chip，而不是独立彩色圆点；chip 以短码文本为主识别，颜色只作辅助 cue，不得与运行状态徽标争夺语义。
- 上游账号 recent 行中的 identity chip 必须作为独立“对话详情”入口；点击或在 chip 上按 `Enter / Space` 时，打开对应 `promptCacheKey` 的对话详情抽屉，不得退化成调用详情。
- identity chip 的颜色映射必须使用稳定离散槽位，而不是连续 hue；同一 `promptCacheKey` 在刷新、排序和 range 切换后应落到同一槽位，不同对话复用同一槽位可接受。
- identity chip 的槽位计算必须混合完整稳定 hash 的高低位；不得直接对展示短码片段做低位 `% 8` 取槽，避免真实数据因为低位偏置而出现成片撞色。
- 账号卡 recent 调用记录不得重复显示所属账号名；调用已嵌在账号大卡内时，账号名必须让位给请求标识、状态与时序摘要。
- 账号卡 recent 调用记录中的紧凑 badge 必须统一高度、字号、圆角、padding 与 line-height；至少 `reasoning effort`、endpoint 与 recent 行双模型显示要复用同一 compact recipe，不得再出现同一行内视觉尺寸不一致。
- 当 recent 调用记录的 `requestModel` 与 `responseModel` 规范化后仍不一致时，账号卡 recent 行必须同时显示“请求模型 + 模型切换图标 + 响应模型”；模型一致时继续显示单模型 badge。
- `StatsResponse.inProgressConversationCount` 与 `StatsResponse.inProgressRetryConversationCount` 必须保留 wire name，但语义改为 invocation-based；所有 Dashboard owner-facing 文案同步改成“进行中调用 / 重试调用”。
- `StatsResponse.inProgressPhaseCounts` 与账号活动接口的 `accounts[].inProgressPhaseCounts` 必须表示当前 live in-progress 调用的三阶段拆分：`queued` 表示尚未选定或开始上游请求，`requesting` 表示连接/发送请求/等待首字节，`responding` 表示已收到首字节并在流式响应中；该字段只代表当前 live 状态，不改写历史终态统计。
- `recentInvocations[]` 与共享 `ApiInvocation` / prompt-cache invocation preview 可带 `livePhase?: queued | requesting | responding | null`；前端展示运行态时必须优先使用后端 `livePhase`，缺失时才允许用 `status`、timing 与 attempt phase 兜底推断，终态成功/失败/HTTP 状态不得强行归入三阶段。
- `today / 1d / 7d` 的 `inProgressInvocationCount / retryInvocationCount` 允许使用 live augmentation 语义；`yesterday` 为 closed range，这两项必须返回 `null` 并在前端显示 `—`。
- `GET /api/stats/dashboard-activity` 必须在请求开始时固定 `rangeEnd=now`，一次读取 runtime invocation overlay，并在同一个响应内返回 `rangeStart`、`rangeEnd`、`snapshotId`、`rateWindow`、`summary` 与可选 `accounts`。
- Dashboard 顶部当前 `TPM`、`消费速率` 与 `进行中调用` 必须来自 `dashboard-activity.summary`；当账号 tab 已打开并请求 `includeAccounts=true` 时，顶部 KPI 与账号卡片必须消费同一个响应的 `snapshotId/rangeEnd`。
- `dashboard-activity.summary.tokensPerMinute`、`summary.spendRate` 与 `summary.stats.inProgressConversationCount` 必须由账号聚合结果求和得到；同一响应内允许的差异仅限前端格式化取整。
- `dashboard-activity.accounts[]` 必须包含真实上游账号聚合项；如果存在无法归属到账号的活动流量，必须返回明确的 `unassigned` 聚合项，而不是让顶部总数无法被明细解释。
- `dashboard-activity.accounts[]` 与 `upstream-account-activity.accounts[]` 必须携带最小账号状态快照字段：`enabled/displayStatus/enableStatus/workStatus/healthStatus/syncState/lastError/lastActionReasonMessage`；这些字段只服务 Dashboard 状态 badge 与健康入口，不改变账号活动聚合口径。
- `includeAccounts=false` 必须支持顶部轻量使用，只返回同源 `summary` 与快照元数据；该路径不得先构建、排序完整账号 preview/archive 明细再丢弃，只能读取 summary/read-model、live overlay 与短尾速率窗口所需数据；账号 tab 首次打开后升级为 `includeAccounts=true`，并用该 full snapshot 同步刷新顶部和账号卡片。
- `DashboardActivityOverview` 不得再用 `buildDashboardTodayRateSnapshot` 作为顶部当前 KPI 的事实来源；该前端 timeseries rate 只能作为无活动快照上下文下的兼容回退或图表趋势辅助。

### SHOULD

- 当前进行中、重试和阶段计数必须由后端基于一次 runtime store 读取生成版本化 `dashboardActivityLive` SSE 快照；前端不得从 recent/records 自行推导。历史聚合、recent 与账号元数据继续沿用 5 秒 HTTP reconcile budget。
- `dashboardActivityLive.revision` 必须单调递增；其快照由 SQLite live read model 与 runtime overlay 的同一合并算法生成。`GET /api/stats/dashboard-activity.liveRevision` 标识 HTTP 返回的实时字段版本。前端不得用较旧 HTTP 或 SSE revision 覆盖较新的实时字段，SSE 重连必须立即下发当前快照。
- Dashboard 默认 `对话` 视图不得预取账号活动；进入 `上游账号` 后必须按“账号卡骨架 -> 汇总卡片 -> recent 调用”渐进展示。只有汇总请求成功且账号数组确实为空时才允许显示真实空态。
- `GET /api/stats/dashboard-activity` 新增可选 `includeRecent`；省略时默认 `true` 保持兼容。Dashboard 第一阶段必须发送 `includeAccounts=true&includeRecent=false`，使账号身份、状态、策略和聚合指标不等待 recent invocation。
- `GET /api/stats/dashboard-activity/recent` 必须接收第一阶段响应的 `rangeStart/rangeEnd/snapshotId` 与 `recentLimit`，用一次批量读取返回所有活动账号的 bounded recent rows；不得逐账号发 SQL，也不得以并发 N 个 SQL 伪装批量读取。响应必须回显快照边界，前端只合并当前请求序列与当前快照一致的结果。
- 首次账号汇总加载时计数 badge 不得显示误导性的 `0`；范围切换与静默校准应保留旧卡片并标注更新中。已有卡片时，更新提示必须收口到头部紧凑状态 chip，而不是在卡片网格上方插入临时行；该 chip 在 idle 与 visible 两种状态下都应按自身内容自然宽度占位，避免为了稳态预留额外固定宽度空白。该 chip 仅在 background refresh 持续超过 `300ms` 后出现，且出现后最少保留 `600ms`，避免高频 reconcile 造成闪烁。新汇总原子替换后，其 recent 区域独立加载；recent 失败不得撤销汇总卡片，必须提供局部错误与重试。
- 账号视图切换后的下一次绘制必须出现稳定骨架；固定多账号本地 fixture 下，第一阶段账号汇总应在 1 秒内完成，且第一阶段 SQL 数量不得随账号数量线性增长。
- 当前实现中，账号视图与 current summary 一样统一收口到 `5s` reconcile/open-resync 预算；任何更激进的 cadence 变更都必须先补充 slow-path 证据。
- 共享 range 状态应继续使用现有 localStorage key，避免打断用户已保存的 Dashboard 偏好。
- 账号卡内的最近调用记录应复用已有 invocation 语义 helper，保证状态、模型、耗时与账号 badge 文案一致。

### COULD

- `groupName` 与 `planType` 可作为 badge 或辅信息展示，但不得替代主标题 `displayName`。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- Dashboard 页面加载后，顶部总览和工作区 section 共享同一个 `activeRange` 状态；总览的已有行为、持久化 key 与渲染顺序保持不变。
- Dashboard 工作区 `对话 / 上游账号` 视图必须记住用户上次主动选择的 tab；下次重新打开 Dashboard，或从其他页面切回总览页时，若该视图在当前 range 下仍可用，则默认恢复到该已记住的选择。
- `对话` tab 继续显示最近 5 分钟内有终态调用，或当前仍处于运行中 / 排队中的对话卡片。
- 用户首次切到 `上游账号` tab 时，前端发起一次账号活动批量请求；后续只在账号 tab 激活时，随共享 range 变化或节流 refresh 再次请求。
- `上游账号` 视图中的 badge 显示“当前活动账号数”；`对话` 视图中的 badge 继续显示“当前对话数”。
- 账号卡顶部显示 `displayName` 与必要 badge；主体显示账号级 KPI；底部显示最近 4 条调用记录。
- 账号卡摘要区保持两行 KPI 栅格，底部 recent 列表优先压缩行内密度而不是继续增加卡片高度。
- `yesterday` 账号视图中的 `进行中调用` 与 `重试调用` 显示 `—`，因为它是 closed range，不做 live augmentation。
- 当当前 range 为 `usage` 时，工作区可以临时强制显示 `对话` 作为降级视图，但不得覆盖用户上次主动选择的 tab 偏好；一旦切回支持账号活动的 range，若用户上次偏好是 `上游账号`，则必须自动恢复该视图。

### Edge cases / errors

- 当账号活动接口返回空列表时，账号 tab 需要显示空态而不是沿用对话列表占位。
- 当账号活动接口失败时，账号 tab 只在自身视图内显示错误态，不影响对话 tab 与顶部总览。
- 当用户停留在账号 tab，随后把范围切到 `usage`，UI 必须立即切回 `对话`，且不触发账号活动请求。
- 当某账号范围内只有失败 / 中断调用时，请求分解、Token 分解与最近 4 条记录仍需稳定显示，不得因为缺少 success 样本而隐藏整卡。
- 当 `cacheHitRate`、`firstByteAvgMs` 或 live augmentation 值缺失时，对应字段显示 `—`，但账号卡其余部分继续渲染。
- 当错误摘要很长或包含上游 JSON 载荷时，Dashboard 对话卡片与账号 recent 行都不得被错误文案横向撑宽；inline 摘要继续单行省略，完整文本只通过共享 tooltip 披露。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                                           | 类型（Kind）        | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers）                       | 备注（Notes）                                                |
| ------------------------------------------------------ | ------------------- | ------------- | -------------- | ------------------------ | --------------- | ----------------------------------------- | ------------------------------------------------------------ |
| `GET /api/stats/dashboard-activity`                    | http-endpoint       | external      | Add            | None                     | backend/stats   | Dashboard top KPI, account activity tab   | 同一快照返回 summary 与可选 accounts；顶部实时 KPI 事实源    |
| `GET /api/stats/dashboard-activity/recent`             | http-endpoint       | external      | Add            | None                     | backend/stats   | Dashboard account activity tab            | 绑定 summary snapshot 的批量 bounded recent rows             |
| `GET /api/stats/upstream-account-activity`             | http-endpoint       | external      | Add            | None                     | backend/stats   | Dashboard account activity tab            | range 聚合 + effective routing rule + recent 4 bounded query |
| `StatsResponse.inProgressConversationCount`            | http-response-field | external      | Modify         | None                     | backend/stats   | Dashboard natural-day KPI, account detail | wire name 保留，语义改为 invocation-based                    |
| `StatsResponse.inProgressRetryConversationCount`       | http-response-field | external      | Modify         | None                     | backend/stats   | Dashboard natural-day KPI, account detail | wire name 保留，语义改为 invocation-based retry              |
| `StatsResponse.inProgressPhaseCounts`                  | http-response-field | external      | Add            | None                     | backend/stats   | Dashboard natural-day KPI, account detail | live invocation 三阶段拆分                                   |
| `UpstreamAccountActivityAccount.inProgressPhaseCounts` | http-response-field | external      | Add            | None                     | backend/stats   | Dashboard account activity tab            | 账号级 live invocation 三阶段拆分，不从 recent 列表推导      |
| `ApiInvocation.livePhase` / preview `livePhase`        | http-response-field | external      | Add            | None                     | backend/stats   | Dashboard, Live, Prompt Cache             | 单调用 live 阶段，终态为空                                   |
| `DashboardActivityOverview` range contract             | ui-component-prop   | internal      | Modify         | None                     | web/dashboard   | Dashboard page, account detail overview   | 支持 controlled / uncontrolled 双模式                        |
| `Dashboard workspace double-tab section`               | ui-component-prop   | internal      | Modify         | None                     | web/dashboard   | Dashboard page                            | `对话 / 上游账号` tabs + count badge                         |
| `useDashboardUpstreamAccountActivity`                  | ui-hook             | internal      | Add            | None                     | web/dashboard   | Dashboard account activity tab            | lazy load + tab-active refresh gate                          |

### 契约文档（按 Kind 拆分）

- `None`

### Shared preview contract notes

- `GET /api/stats/upstream-account-activity.recentInvocations[]` 复用现有 invocation preview wire shape，并额外包含 `promptCacheKey?: string | null`。
- `GET /api/stats/upstream-account-activity.accounts[].effectiveRoutingRule` 与 `GET /api/stats/dashboard-activity.accounts[].effectiveRoutingRule` 复用账号池现有 `EffectiveRoutingRule` wire shape，用于 Dashboard 标题区固定快捷策略 chip 的初始状态；普通系统 tag 仍不在账号活动接口中展示。
- `GET /api/stats/upstream-account-activity.accounts[]` 与 `GET /api/stats/dashboard-activity.accounts[]` 的状态字段复用账号池状态模型：`enabled/displayStatus/enableStatus/workStatus/healthStatus/syncState/lastError/lastActionReasonMessage`，前端只把异常/注意态渲染为状态 badge。
- Dashboard 快捷策略写入复用 `PATCH /api/pool/upstream-accounts/:id`，payload 仅包含 `routingRule` 中被触碰过的账号级覆盖字段；该入口不支持恢复继承。
- `GET /api/stats/dashboard-activity.summary` 复用 `StatsResponse` wire shape，并额外返回 `tokensPerMinute` / `spendRate`；`accounts[]` 复用账号活动卡片所需字段，并允许 `upstreamAccountId: null` 的 `isUnassigned` 聚合项。
- SSE `dashboardActivityLive` 返回 `revision/generatedAt`、总览进行中/重试/阶段计数和按 `accountKey` 分组的相同计数；账号无 live 项时其实时字段归零。
- `GET /api/stats/dashboard-activity.rateWindow.mode` 固定描述当前速率算法来源；当前值为账号活跃尾段求和，不代表 timeseries 图上任一 bucket 的事实。
- 前端共享 `PromptCacheConversationInvocationPreview` 合同同步包含 `promptCacheKey?: string | null`；`DashboardWorkingConversationInvocationSelection.promptCacheKey` 语义不变，仍表示真实对话键。

## 验收标准（Acceptance Criteria）

- Given Dashboard 工作区加载完成，When 查看右上角，Then 可以看到 `对话 / 上游账号` tabs，默认激活 `对话`，且现有 working-conversation 卡片交互不变。
- Given 共享 range 为 `today / yesterday / 1d / 7d`，When 切到 `上游账号`，Then 账号集合与汇总指标随范围变化，且只包含该范围内至少有一条调用的账号。
- Given 当前在 `上游账号` tab，When 把共享 range 切到 `usage`，Then 账号 tab disabled，界面自动回退到 `对话`，且不会发账号活动请求。
- Given 用户上次主动选择了 `上游账号`，When 刷新页面、重新打开 Dashboard，或从其他页面切回 Dashboard，Then 在当前 range 允许账号视图时，工作区默认恢复到 `上游账号`。
- Given 用户上次主动选择了 `上游账号`，When 当前 range 临时切到 `usage` 后再切回 `today / yesterday / 1d / 7d`，Then 工作区应重新恢复到 `上游账号`，而不是把偏好永久改写成 `对话`。
- Given 从未打开过账号 tab，When 停留在 `对话` tab，Then 前端不会请求账号活动接口。
- Given 某账号有范围内调用，When 查看账号卡，Then 标题使用 `displayName`，顶部同一行包含异常/注意状态 badge、固定快捷策略 chip、文本型 `TPM`、`消费速率` 实时指标、账号 ID 与齿轮入口，且卡内不再出现 `渠道 xxx / 分组` 行或顶部 `调用` 指标。
- Given 账号活动接口返回的 `effectiveRoutingRule` 命中快捷策略，When 账号卡渲染，Then 标题区固定显示优先级入口、Fast 模式、`禁出`、`禁入`，其中优先级入口至少覆盖 `普通`、`兜底`、`主力`、`禁新` 四态，Fast 模式至少覆盖 `不改Fast`、`补Fast`、`强制Fast`、`禁Fast` 四态，且不显示普通系统 tag 名称。
- Given 用户连续点击优先级入口，When 状态轮换，Then 顺序必须为 `普通 → 兜底 → 主力 → 禁新 → 普通`，且 1 秒 debounce 内只提交最终账号级 `routingRule.priorityTier`。
- Given 用户连续点击 Fast 模式入口，When 状态轮换，Then 顺序必须为 `不改Fast → 补Fast → 强制Fast → 禁Fast → 不改Fast`，且 1 秒 debounce 内只提交最终账号级 `fastModeRewriteMode`。
- Given 用户点击 `禁出 / 禁入`，When 状态切换，Then UI 立即乐观更新，并在 1 秒后保存账号级 `allowCutOut / allowCutIn` 覆盖；失败时回滚并显示卡内错误。
- Given 账号状态为异常/注意态，When 账号卡渲染，Then 状态 badge 集合只显示异常/注意态；点击该集合打开账号详情 `healthEvents` 标签页。
- Given 用户点击账号卡齿轮按钮，When 打开账号详情，Then 必须进入 `routing` 标签页。
- Given 查看账号卡周期统计，When 卡片渲染完成，Then 可见四组统计：`首字用时 + 响应时间`、`请求数 + 成功 / 失败 / 其他`、`成本 + 失败 / 失败成本比率(%)`、`Token + 缓存命中率 / 失败`，且所有数值使用滚动数字效果；当 `failureCost=0` 时，成本组失败成本比率显示为 `0%`。
- Given 查看账号卡四组周期统计，When 对任一统计卡 hover、focus、点击或移动端长按，Then 整张统计卡打开结构化浮层，浮层明确展示主字段名和值、卡面已有分解字段名和值，以及 0 到 3 个相关补充数据。
- Given 查看账号卡四组周期统计，When 卡片常驻态渲染完成，Then 卡内分解段落不再各自创建独立 tooltip trigger，避免和整卡浮层形成嵌套触发区域。
- Given 两个工作区视图分别选择了不同排序，When 切换标签或刷新页面，Then 每个标签恢复自己的选择，并显示对应排序名称。
- Given 多个对话或账号收到 SSE patch / 活动快照更新，When 当前排序字段发生变化，Then 卡片立即按当前模式重排，且不产生额外刷新请求。
- Given 账号或对话存在多个候选项，When 选择 `createdAt / lastInvocation / cost / tokens` 任一排序模式，Then 4 种模式都按倒序排列；时间缺失项继续置后且平局稳定。
- Given 账号列表中同时存在已分配账号和 `未分配上游账号` 聚合项，When 切换任意工作区排序模式，Then `未分配上游账号` 始终排在所有已分配账号之后，并只在未分配项内部继续应用当前倒序规则。
- Given 某账号有至少 4 条范围内调用，When 查看账号卡底部，Then 只显示最近 4 条，按 `occurredAt DESC` 排序。
- Given 某账号 recent 调用记录存在真实 `promptCacheKey`，When 查看请求标识主行，Then 可见基于该键计算出的对话短 ID、分隔图标与完整请求 ID，且短 ID 展示值不带 `WC-` 前缀。
- Given 某账号 recent 调用记录渲染主标识行，When 查看对话短 ID，Then 它表现为轻量短码 chip，且颜色来自稳定离散辅助色槽位，而不是单独的状态样式圆点。
- Given 用户点击某账号 recent 行里的对话短 ID identity chip，When 交互发生，Then 只打开对应 `promptCacheKey` 的对话详情抽屉，不会误打开调用详情抽屉。
- Given 查看账号卡摘要区，When 卡片处于常驻态，Then 不出现解释性废话或状态说明条，请求数 / Token 分解只显示色点与数值，且不出现任何可见文字标签。
- Given 查看账号卡 recent 区标题行，When 右侧存在 recent bridge 统计，Then 显示完整状态文字，并与左侧“最近 4 条调用”标题保持同一垂直对齐。
- Given 查看账号卡内 recent 调用记录，When 与对话卡片调用记录对照，Then recent 行至少包含状态、模型、endpoint、Token 摘要与 `RQ / UP / ED / TT` 时序摘要，且 4 条记录完整留在卡内。
- Given 账号卡 recent 调用记录所在账号已由大卡标题表达，When 查看 recent 行辅助元信息，Then 不再重复渲染账号名。
- Given 账号卡 recent 调用记录的 `requestModel` 与 `responseModel` 规范化后不一致，When recent 行渲染模型区域，Then 同时显示请求模型、模型切换图标与响应模型；若两者等价，则只显示单模型。
- Given 点击账号卡 recent 调用记录打开详情，When 详情抽屉接收 selection，Then `selection.promptCacheKey` 必须等于真实 preview `promptCacheKey`，而不是 `invokeId`。
- Given Dashboard 顶部 KPI 使用 `StatsResponse.inProgressConversationCount` / `inProgressRetryConversationCount`，When 显示 owner-facing 文案，Then 标签为“进行中调用 / 重试调用”，并按 invocation-based 计数，而不是按 prompt-cache 对话去重。
- Given 后端账号活动接口需要账号摘要与最近 4 条记录，When 发起请求，Then 响应来自单个 batch endpoint，不依赖前端 fanout `upstream-account detail` 或 `window-usage`。
- Given 同一个 mock/fixture 返回 `dashboard-activity` full snapshot，When 同屏渲染顶部 KPI 与账号卡片，Then `top.inProgressInvocationCount === sum(accounts.inProgressInvocationCount)`。
- Given 已收到 revision 更高的 `dashboardActivityLive`，When 较旧 HTTP reconcile 或乱序 SSE 到达，Then 顶部与账号卡保持较新 live 数值且不会回退为 0。
- Given 同一个 mock/fixture 返回 `dashboard-activity` full snapshot，When 同屏渲染顶部 KPI 与账号卡片，Then `top.tokensPerMinute === sum(accounts.tokensPerMinute)`，允许仅因小数格式化产生显示级差异。
- Given 同一个 mock/fixture 返回 `dashboard-activity` full snapshot，When 同屏渲染顶部 KPI 与账号卡片，Then `top.spendRate === sum(accounts.spendRate)`，允许仅因货币格式化产生显示级差异。
- Given 账号视图首次激活且汇总尚未返回，When UI 渲染工作区，Then 下一次绘制显示与最终 grid 尺寸一致的账号卡骨架，计数不显示 `0`，且不出现“暂无活动”。
- Given 第一阶段汇总已返回，When recent 批量请求仍在进行，Then 汇总卡片立即可操作且每张卡的 recent 区显示局部骨架。
- Given recent 批量请求失败，When 汇总卡片仍存在，Then 卡片保留并在 recent 区显示可重试错误；重试成功后只替换同一快照的 recent 数据。
- Given 已有账号卡片并切换 range，When 新汇总尚未返回，Then 保留旧卡片并显示更新中；旧 range 或旧 snapshot 的迟到响应不得覆盖当前状态。
- Given N 个活动账号，When 第一阶段使用 `includeRecent=false`，Then 不执行 recent query；When 第二阶段加载 recent，Then 使用一次批量读取并把每账号结果限制到 `recentLimit`。
- Given 账号 tab 尚未打开，When Dashboard 顶部需要当前活动 KPI，Then 前端只请求 `includeAccounts=false` 的 summary-only 快照，不请求账号明细。
- Given 账号 tab 打开，When Dashboard 同屏显示顶部 KPI 与账号卡片，Then 两者来自同一个 `snapshotId/rangeEnd` 的 full snapshot。
- Given 图表趋势与顶部瞬时 KPI 数值不同，When 审查 UI 口径，Then 该差异被接受，因为 chart 是趋势展示，顶部 KPI 是活动快照事实源。

## 验收清单（Acceptance checklist）

- [x] 核心路径的长期行为已被明确描述。
- [x] 关键边界/错误场景已被覆盖。
- [x] 涉及的接口/契约已写清楚或明确为 `None`。
- [x] 相关验收条件已经可以用于实现与 review 对齐。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: 账号活动 hook / 账号视图组件 / Dashboard range 共享与回退行为。
- Integration tests: summary invocation-based 计数、账号活动接口 range + recent query、Dashboard 页面 tabs / lazy load / disabled 回退。
- Integration tests: `dashboard-activity` summary 的 `TPM`、`消费速率`、`进行中调用` 等于 accounts 加总，并覆盖同一 `rangeEnd`、runtime overlay 与 `unassigned` 流量。
- E2E tests (if applicable): None。

### UI / Storybook (if applicable)

- Stories to add/update: `DashboardWorkingConversationsSection`、Dashboard page story、账号活动卡状态图库。
- Interaction coverage to add/update: 上游账号四组统计卡整卡 tooltip 的 hover/focus/click 入口与字段明细。
- Docs pages / state galleries to add/update: working conversations / account activity 双视图状态。
- `play` / interaction coverage to add/update: `today / yesterday / 1d / 7d / usage`、tab 切换、空态、错误态。
- Visual regression baseline changes (if any): 以本 spec 的 `## Visual Evidence` 为准。

### Quality checks

- `cargo test`（summary / account activity 相关 targeted tests）
- `cargo check`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run build-storybook`

## Visual Evidence

PR: include

![Dashboard workspace sorting controls](./assets/dashboard-workspace-controls-focused.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-sort-descending-order`
  scenario: `workspace sort modes descend, unassigned account stays last`
  evidence_note: 验证 Dashboard `上游账号` tab 在工作区排序切到 `Token` 时，已分配账号仍按倒序排在前，`未分配上游账号` 聚合卡固定落在最后；同一 story 的 play 与浏览器 DOM 验证覆盖 `createdAt / lastInvocation / cost / tokens` 四种倒序模式。
  image:
  PR: include
  ![Dashboard 上游账号排序倒序与未分配置后证据](./assets/upstream-account-sort-desc-all.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--error-summary-tooltips`
  scenario: `conversation card error summary tooltip`
  evidence_note: 验证 Dashboard `对话` tab 当前调用错误摘要保持单行省略，不撑宽卡片；hover 摘要文本时共享 UI tooltip 优先在触发文本下方展开完整上游错误，不再依赖原生 `title`。
  image:
  PR: include
  ![Dashboard 对话卡片错误摘要 tooltip 证据](./assets/dashboard-working-conversation-error-tooltip.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--error-summary-tooltips`
  scenario: `upstream account recent error summary tooltip`
  evidence_note: 验证 Dashboard `上游账号` tab recent 行错误摘要同样保持单行省略，不撑宽 row；hover 摘要文本时通过同一共享 UI tooltip 在下方优先展示完整错误，实现与对话卡片一致的错误披露语义。
  image:
  PR: include
  ![Dashboard 上游账号 recent 错误摘要 tooltip 证据](./assets/dashboard-upstream-account-error-tooltip.png)

- source_type: mock_ui
  story_id_or_title: `dashboard-working-conversations-layout.spec.ts`
  scenario: `wide upstream account long error summary no overflow`
  evidence_note: 验证双列账号卡在长 429 错误载荷下保持等宽；错误摘要在 recent 行内单行截断，且 grid、账号卡与调用行均不越过父容器右边界。
  image:
  PR: include
  ![Dashboard 上游账号长错误摘要不溢出证据](./assets/dashboard-upstream-account-error-summary-no-overflow.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--running-only-conversation`
  scenario: `inline invocation phase status`
  evidence_note: 验证 Dashboard 对话卡片头部与当前调用槽位的运行态状态已改为同一行 inline 图标 + 彩色文字；`响应中` 不再使用 badge 背景、边框或胶囊 padding，同时保留 endpoint、reasoning effort 等元信息 badge。
  image:
  PR: include
  ![Dashboard 对话卡片运行态 inline 状态证据](./assets/dashboard-working-conversation-inline-status.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-tab`
  scenario: `live phase split`
  evidence_note: 验证 Dashboard 上游账号卡标题区与最近调用标题右侧都按账号级 `inProgressPhaseCounts` 显示 `排队中 2 / 请求中 3 / 响应中 4`；Storybook fixture 中 recent 列表只有 4 条，证明该统计不从卡内列表 reduce。
  image:
  PR: include
  ![Dashboard 上游账号三阶段实况统计证据](./assets/dashboard-upstream-account-live-phase-counts.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-tab`
  scenario: `account header in-progress invocations`
  evidence_note: 验证上游账号卡片标题区在关键策略徽章与实时 `TPM / 消费速率` 指标之间展示 `进行中调用` 文本型读数，值来自账号 `inProgressInvocationCount`，并保持同一行内紧凑扫描节奏。
  image:
  PR: include
  ![Dashboard 上游账号进行中调用标题区证据](./assets/dashboard-upstream-account-in-progress-invocations.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-metric-tooltips`
  scenario: `metric card whole-card tooltip`
  evidence_note: 验证上游账号四组统计卡支持整卡触发结构化浮层；截图保留成本卡打开态，浮层按主字段、当前字段与相关数据分层展示，并明确列出字段名和值。
  image:
  PR: include
  ![Dashboard 上游账号统计卡整卡浮层证据](./assets/dashboard-upstream-account-metric-tooltips.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-tab`
  scenario: `fast mode quick policy chip`
  evidence_note: 验证 Dashboard 上游账号卡片标题区固定快捷策略 chip 已包含 Fast 模式入口；当前账号显示 `强制Fast` 态，策略 chip 与异常/注意状态 badge、计划 badge、右侧 `进行中调用 / TPM / 消费速率 / 齿轮` 保持同一行内扫描节奏，且不再渲染本地账号编号。
  image:
  PR: include
  ![Dashboard 上游账号 Fast 快捷策略 chip 证据](./assets/dashboard-upstream-account-fast-policy-chip.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-tab`
  scenario: `quick policy chips and account attention badges`
  evidence_note: 验证 Dashboard 上游账号卡片标题区显示异常/注意状态 badge 集合（`上游拒绝 / 限流`）、固定快捷策略 chip（`禁新 / Fast / 禁出 / 禁入`）与右侧齿轮路由入口；高亮/弱化态可在同一张账号卡内扫描。
  image:
  PR: include
  ![Dashboard 上游账号快捷策略与状态入口证据](./assets/dashboard-upstream-account-quick-policy-status.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-quick-policy-tone-palette`
  scenario: `quick policy semantic tones light`
  evidence_note: 验证 Dashboard 上游账号快捷策略 chip 在浅色主题下按语义配色：`兜底` 为 success、`强制Fast` 为 primary、激活 `禁出` 为 warning、未激活 `禁入` 为 neutral。
  image:
  PR: include
  ![Dashboard 上游账号快捷策略语义色浅色证据](./assets/dashboard-upstream-account-policy-tones-light.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-quick-policy-tone-palette-dark`
  scenario: `quick policy semantic tones dark`
  evidence_note: 验证 Dashboard 上游账号快捷策略 chip 在深色主题下保持同一语义配色与可读性：success / primary / warning / neutral 四个色槽同屏可见。
  image:
  PR: include
  ![Dashboard 上游账号快捷策略语义色深色证据](./assets/dashboard-upstream-account-policy-tones-dark.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-tab`
  scenario: `account header without legacy activity status badge`
  evidence_note: 验证 Dashboard 上游账号卡片标题区只保留异常/注意状态 badge 集合（`上游拒绝 / 限流`）与固定快捷策略 chip；旧的活动状态 badge（`关注 / 繁忙 / 稳定`）不再渲染，避免与账号健康状态入口重复。
  image:
  PR: include
  ![Dashboard 上游账号移除旧活动状态 badge 证据](./assets/dashboard-account-card-no-legacy-status.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-tab`
  scenario: `desktop1660`
  evidence_note: 验证 Dashboard 工作区已切换到 `上游账号` tab，桌面宽屏下账号卡按 2 列紧凑放大布局展示账号级 KPI、轻量对话短码 identity chip + 请求 ID 主标识行，以及请求/响应模型不一致时的双模型切换展示。
  image:
  PR: include
  ![Dashboard 上游账号 tab 桌面宽屏证据](./assets/dashboard-upstream-account-tab-desktop.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-tab`
  scenario: `mobile390`
  evidence_note: 验证相同账号 tab 在移动视口下收敛为单列卡片，并保留摘要区与最近 4 条调用记录。
  image:
  ![Dashboard 上游账号 tab 移动视口证据](./assets/dashboard-upstream-account-tab-mobile.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-tab`
  scenario: `first-response-byte-total desktop card`
  evidence_note: 验证账号卡“首字用时”主值使用 owner-facing 的首字总耗时口径；当后端同时返回阶段级 `firstByteAvgMs` 与显式 `firstResponseByteTotalAvgMs` 时，卡面主值显示秒级总耗时而不是被极小的上游首字节时延误导成 `0ms`。
  image:
  ![Dashboard 上游账号首字总耗时证据](./assets/dashboard-upstream-account-first-byte-total.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-tab`
  scenario: `remembered-workspace-view desktop`
  evidence_note: 验证 Dashboard 工作区右上 tabs 右贴边呈现；当浏览器已记住用户上次主动切到 `上游账号` 时，重新进入总览页会恢复该视图，且 `usage` 下的临时回退不会抹掉该偏好。
  image:
  ![Dashboard 工作区视图记忆与右贴边证据](./assets/dashboard-workspace-view-memory.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-refreshing`
  scenario: `desktop header refresh chip`
  evidence_note: 验证已有账号卡时，background refresh 状态收口到头部紧凑 chip；账号卡网格上方不再插入单独提示行，且 chip 只按自身内容自然宽度占位，不会额外吃掉固定空白。
  image:
  ![Dashboard 上游账号头部刷新 chip 桌面证据](./assets/dashboard-upstream-account-refresh-chip-desktop.png)

- source_type: storybook_canvas
  story_id_or_title: `dashboard-workingconversationssection--upstream-account-refreshing-mobile`
  scenario: `mobile header refresh chip`
  evidence_note: 验证移动视口下同一刷新状态也收口为头部紧凑 chip；状态切换不会额外挤出一条临时行，也不会为了 idle 状态预留不合理的固定宽度空位。
  image:
  ![Dashboard 上游账号头部刷新 chip 移动证据](./assets/dashboard-upstream-account-refresh-chip-mobile.png)

- source_type: ui_demo
  story_id_or_title: `#/dashboard?demoScene=progressive-loading&demoTheme=dark`
  scenario: `desktop progressive summary skeleton`
  evidence_note: mock-only demo 将账号汇总请求延迟，切换到上游账号后下一帧显示与双列布局匹配的骨架；计数显示“账号加载中”，不会误显 0 或“暂无活动”。
  image:
  PR: include
  ![Dashboard 上游账号渐进加载桌面骨架](./assets/dashboard-progressive-skeleton-desktop.png)

- source_type: ui_demo
  story_id_or_title: `#/dashboard?demoScene=progressive-loading&demoTheme=dark`
  scenario: `desktop summary complete and recent batch complete`
  evidence_note: mock-only demo 完成两阶段请求后显示 12 张账号汇总卡与批量 recent 行；账号卡保持可操作，recent 数据按快照边界补齐。
  image:
  PR: include
  ![Dashboard 上游账号渐进加载桌面完成态](./assets/dashboard-progressive-complete-desktop.png)

- source_type: ui_demo
  story_id_or_title: `#/dashboard?demoScene=progressive-loading&demoTheme=dark`
  scenario: `mobile upstream account complete`
  evidence_note: 移动视口下账号区收敛为单列卡片，保留汇总指标、recent 记录和页头 tab 操作，不出现横向溢出。
  image:
  PR: include
  ![Dashboard 上游账号渐进加载移动完成态](./assets/dashboard-progressive-complete-mobile.png)

## Related PRs

- None

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：账号活动接口若直接扫 live invocations，未来数据量继续增长时可能需要进一步下沉到 read-model / materialized rollup，但本轮先保证 bounded recent query 与单次聚合链路正确。
- 风险：summary wire field 保留旧名字但改语义，会要求所有 owner-facing 文案和测试同时更新；遗漏任何一处都可能造成“字段值对、文案错”的混乱。
- 风险：账号卡若继续通过增高固定高度容纳信息，会重新滑向“整页面板”观感；后续新增字段时应优先压缩行内布局与摘要表达，而不是继续加高卡片。
- 假设：recent 行 identity chip 仅在上游账号 tab 内收口为当前真相；对话 tab 主卡片与详情抽屉的短码呈现方式不在本 spec 本轮改动范围内。
- 假设：`today / 1d / 7d` 的进行中调用与重试调用使用 live augmentation 语义；`yesterday` closed range 返回 `null`。
- 假设：活动账号判定是“当前所选范围内至少有 1 条调用的账号”。
- 假设：`渠道名 = displayName`，`groupName / planType` 仅作辅信息。

## 参考（References）

- `docs/specs/gz5ns-dashboard-natural-day-kpi-semantics/SPEC.md`
- `docs/specs/t6d9r-account-detail-stats-read-model/SPEC.md`
- `docs/specs/5932d-sse-proxy-live-sync/SPEC.md`
- `docs/solutions/performance/realtime-dashboard-reconcile-budget.md`
