# Dashboard 工作区卡片双视图与上游账号活动聚合 演进历史（#z6ysw）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

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

## Key Reasons / Replacements

- `#gz5ns` 已冻结 Dashboard 顶部自然日 KPI 语义，但没有覆盖工作区 section 的双视图与账号活动聚合边界，因此需要新的 active topic spec 承接。
- `#t6d9r` 已限制账号详情统计走 account read-model，本 spec 只为 Dashboard 引入“批量账号活动摘要 + recent query”能力，不替代账号详情页的 read-model 责任。
- `#5932d` 曾冻结 Dashboard in-progress 的严格语义，但 owner-facing 视图已从“按对话观察”演进到“按调用观察”，因此本 spec 覆盖 Dashboard summary owner-facing 语义的后续收口。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
