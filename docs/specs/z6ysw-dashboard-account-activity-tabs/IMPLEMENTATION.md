# Dashboard 工作区卡片双视图与上游账号活动聚合 实现状态（#z6ysw）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已实现，待提交收口
- Lifecycle: active
- Catalog note: dashboard workspace dual tabs, account activity aggregation, invocation-based in-progress semantics

## Coverage / rollout summary

- 已实现：Dashboard 上游账号视图拆为汇总优先与快照绑定的 recent 批量补齐；首屏使用局部骨架，范围刷新保留旧卡片，recent 失败保留汇总并局部重试。
- 已实现：`includeRecent=false` 的第一阶段对保留期外数据执行 archive 内部分组聚合，只返回账号指标而不读取、排序或传输 invocation preview；兼容 combined 响应与第二阶段 recent 接口继续按相同精确快照边界读取 bounded preview。
- 已实现：Dashboard 工作区头部控制条重新对齐 spec 基线，桌面布局恢复为“左侧 tabs、右侧 当前对话 badge + 排序按钮”的紧凑顺序，不再出现 `badge -> tabs -> 排序` 的错误节奏；对应视觉证据已刷新为当前实现。

- 已实现：Dashboard 页面提升并共享顶部 range 状态，工作区 section 接入 `对话 / 上游账号` 双 tabs，并保留既有对话 working-set 行为。
- 已实现：Dashboard 工作区 tabs 额外持久化用户上次主动选择的视图；重新打开 Dashboard 或切回总览页时，在当前 range 允许的前提下恢复该视图；`usage` 仅临时强制回退到 `对话`，不会覆盖已记住的 `上游账号` 偏好。
- 已实现：新增 `GET /api/stats/upstream-account-activity` 批量接口，返回账号级聚合摘要、recent 4 bounded query，以及 `yesterday` closed-range 的空 live count 语义。
- 已实现：summary `inProgressConversationCount` / `inProgressRetryConversationCount` 改为 invocation-based 语义，Dashboard owner-facing 文案同步为“进行中调用 / 重试调用”。
- 已实现：账号 tab 懒加载、`usage` disabled + 自动回退、独立账号卡布局、Storybook 交互场景、视觉证据与 targeted validation。
- 已实现：账号卡收敛为紧凑信息卡，移除状态说明条和解释性废话；请求数 / Token 分解仅保留色点与数值，连单字 / 缩写短标签也不常驻显示，完整标签通过 hover 暴露；recent 区标题行右侧统计保留完整状态文字并与标题垂直对齐。
- 已实现：账号卡内部结构描边统一压回低对比中性边框，外框、摘要格子、recent 行与分隔线不再复用主题蓝或其他语义色作为结构边界；颜色仅保留在状态点、数值和 badge 上。
- 已实现：账号卡底部 4 条 recent 调用记录全部留在卡内可见，单条 recent 行补齐 endpoint、Token 摘要与 `RQ / UP / ED / TT` 时序摘要，使信息密度不低于对话卡片中的调用记录。
- 已实现：桌面宽屏账号卡固定高度收敛到更紧凑值，避免整页面板观感，同时保持 4 条 recent 记录完整可见。
- 已实现：上游账号 recent 行改为“对话短 ID + 请求 ID”主标识布局，短 ID 基于真实 `promptCacheKey` 计算并去掉 `WC-` 前缀；点击详情时传递的 `selection.promptCacheKey` 也已修正为真实对话键。
- 已实现：上游账号 recent 行的对话短 ID 从“连续色圆点 + 短码”收口为轻量 identity chip；chip 以短码文本为主识别，颜色降为离散辅助槽位，不再与状态徽标混淆语义。
- 已实现：上游账号 recent 行的 identity chip 不再继承整行的调用详情点击语义；chip 现作为独立对话详情入口，点击或键盘触发时只打开对应 `promptCacheKey` 的对话抽屉，而 recent 行其余区域仍保持打开调用详情。
- 已实现：identity chip 的离散槽位改为基于完整稳定 hash 做高低位混合后再映射，修正真实线上数据因低位 `% 8` 偏置导致的短码成片撞色问题。
- 已实现：上游账号 recent 行不再重复显示账号名；当 `requestModel` / `responseModel` 规范化后仍不一致时，recent 行改为同时展示请求模型、切换图标与响应模型。
- 已实现：上游账号 recent 行的 endpoint、reasoning effort 与双模型 badge 统一复用 compact 尺寸 recipe，消除同一行内 badge 高度不一致问题。
- 已实现：上游账号卡片标题区改为账号名 + 文本型实时 `TPM / 消费速率` 指标，删除卡内 `渠道 / 分组` 行和顶部 `调用` 指标；周期统计重排为首字用时、请求数、成本、Token 四组，并沿用滚动数字效果。
- 已实现：上游账号卡片标题区补充文本型实时 `进行中调用` 指标，取账号活动接口的 `inProgressInvocationCount`，并与 `TPM / 消费速率` 保持同一行内读数语言；Dashboard 账号活动接口不再返回 `activeConversationCount`。
- 已实现：运行中调用统一拆为 `queued / requesting / responding` 三阶段；`StatsResponse`、账号活动接口与 invocation preview 暴露 `inProgressPhaseCounts` / `livePhase`，Dashboard 上游账号卡标题区与 recent bridge 均读取账号级 live 统计，不再从卡内 recent 列表推导运行态数量。
- 已实现：上游账号卡片四组周期统计改为整张统计卡触发结构化 tooltip；浮层按主值、当前字段、相关数据分层展示字段名和值，并关闭卡内分解段落的逐段 tooltip，避免嵌套触发区域。
- 已实现：账号活动接口返回每个账号的 `effectiveRoutingRule` 与最小账号状态字段；Dashboard 账号卡标题区固定展示优先级、Fast 模式、`禁出`、`禁入` 快捷策略 chip，并只把异常/注意态状态渲染为可点击 badge 集合。
- 已实现：Dashboard 账号卡快捷策略入口使用乐观 UI + 1 秒 debounce 写入账号级 `routingRule` 覆盖；优先级入口按 `普通 → 兜底 → 主力 → 禁新 → 普通` 轮换并写 `priorityTier=normal|fallback|primary|no_new`，Fast 模式按 `不改Fast → 补Fast → 强制Fast → 禁Fast → 不改Fast` 轮换，`禁出 / 禁入` 分别写账号级 `allowCutOut / allowCutIn`，该入口不提供恢复继承。
- 已实现：Dashboard 账号卡快捷策略 chip 使用独立语义 tone helper；`普通 / 不改Fast` 为 neutral，`兜底 / 补Fast` 为 success，`主力 / 强制Fast` 为 primary，`禁新 / 禁Fast / 激活禁出 / 激活禁入` 为 warning，并通过 `data-policy-tone` 固化回归检查。
- 已覆盖：Dashboard 上游账号快捷策略语义色在浅色与深色 Storybook 场景中同屏展示 success / primary / warning / neutral 四个色槽，并写入 `SPEC.md` 视觉证据。
- 已覆盖：Dashboard 上游账号卡 Fast 模式快捷入口的组件测试断言 debounce 窗口内不会禁用 chip，可连续点击到最终目标态，并且 1 秒窗口内只提交最终 `fastModeRewriteMode`。
- 已实现：账号卡异常/注意状态 badge 集合点击进入账号详情 `healthEvents` 标签页，右侧齿轮按钮进入账号详情 `routing` 标签页；`useUpstreamAccountDetailRoute` 已支持 `healthEvents` tab。
- 已实现：Dashboard 上游账号卡标题区不再渲染本地 `#<upstreamAccountId>` 编号；标题区保留账号名、异常/注意状态 badge、快捷策略 chip、实时指标与齿轮路由入口，避免把内部主键暴露成主要扫描元素。
- 已实现：账号活动接口补出 `avgTotalMs`、`totalCost`、严格失败 `failureCost` 与 `failureTokens`；请求组的非成功率由前端按 `nonSuccessCount / requestCount` 计算，成本组的失败成本比率由前端按 `failureCost / totalCost` 计算，`其他` 按 `nonSuccessCount - failureCount` 下限归零。
- 已实现：账号活动接口中的 `tokensPerMinute` / `spendRate` 改为按每个账号最近 5 分钟活跃尾段计算；账号卡今日总量、recent 调用与排序仍使用所选 range 总量口径。
- 已实现：账号活动 live rows、账号卡 `inProgressInvocationCount` 与 account-scoped summary 对 pool running 调用使用同 `invokeId` 的 pool attempt 账号作为 fallback，避免已选账号但 payload 尚未写入 `upstreamAccountId` 时形成未归属 running 行。
- 已实现：账号卡列表按 `totalTokens` 倒序排列，并用最近调用时间与账号 ID 作稳定排序兜底。
- 已实现：账号卡“首字用时”从阶段级 `t_upstream_ttfb_ms` 纠偏为 owner-facing 的首字总耗时口径；后端聚合现在复用 `resolve_first_response_byte_total_ms(...)`，并额外暴露显式 `firstResponseByteTotalAvgMs` 供前端优先消费，避免真实秒级总耗时被渲染成 `0ms`。
- 已实现：工作区 `对话` tab 当前 5 分钟 working-set 的 head/count 改读 write-side `prompt_cache_working_set_live`，并为 mixed-source key 保留 `All / ProxyOnly` 两套聚合列，避免换源后 `ProxyOnly` 视角丢 key 或排序漂移。
- 已实现：工作区 `对话` tab 的 snapshot count/page 也收口到同一份 live working-set truth，不再通过 `WITH recent_terminal` 对 `codex_invocations` 做严格历史重算。公开字段、cursor 形态、recent preview 与主排序语义保持不变，但 snapshot membership 明确接受 `<=5s` bounded freshness。
- 已实现：Dashboard 工作区 `对话` 当前/最近调用错误摘要与 `上游账号` recent 行错误摘要统一接入共享 `InvocationErrorSummary`；inline 文案固定单行省略并保持 `min-w-0` 布局约束，完整错误只通过现有 UI tooltip 在 hover / focus / long-press 时披露，不再依赖原生 `title`。
- 已实现：上游账号宽屏双列 grid 使用 `minmax(0, 1fr)` track，账号卡、recent 行和共享错误 trigger 均显式允许收缩；错误摘要无法再通过 intrinsic width 撑开调用行或父账号卡。现役 feature 的 unit、Storybook play 契约与 Playwright 几何回归共同覆盖该链路。
- 已实现：Dashboard 相关的 working-set / account-activity 派生维护继续遵守 `<=5s` bounded freshness；proxy capture 请求尾的 rollup/live progress、upstream account touch 与 attempt 中间进度已迁入 SQLite batch writer，避免 Dashboard reconcile 与请求收尾派生写在 SQLite 单写者上持续争用。
- 已实现：Dashboard current records、summary/timeseries、上游账号活动与工作区 `对话` tab 的 running 视图统一 overlay 进程内 runtime invocation store。SSE 仍即时广播 `running/pending` 记录，HTTP open-resync/current reconcile 即使 DB 不再刷新 running 行也不会短暂丢行；terminal DB 事实优先并会移除对应内存记录。
- 已实现：terminal invocation 记录进入 SQLite write controller，代理业务响应不等待落库。Dashboard 账号 recent reconcile 会先合并 runtime running / pending / terminal 候选，并与 SQLite 行按稳定调用键去重；统计聚合仍以持久化 terminal 事实为准，running snapshot 不再产生 DB/batch 写入。
- 已实现：新增 `GET /api/stats/dashboard-activity` 活动快照读路径；请求开始时固定 `rangeEnd`，一次读取 runtime overlay，并返回 summary-only 或 summary + accounts 两种形态。
- 已实现：Dashboard 顶部当前 `TPM / 消费速率 / 进行中调用` 改读 `dashboard-activity.summary`；账号 tab 打开后升级为 `includeAccounts=true`，顶部 KPI 与账号卡片共享同一个 `snapshotId/rangeEnd` 响应。
- 已实现：Dashboard 当前进行中、重试与阶段计数改由后端 SQLite live read model 加 runtime overlay 的统一算法生成版本化 `dashboardActivityLive` SSE 快照；前端按 revision 覆盖顶部与账号卡 live 字段，旧 HTTP reconcile 不再把新状态回写为 0，重连时服务端立即种入当前快照。
- 已实现：运行态变更只无阻塞地递增 live snapshot 序列号；单个后台 worker 在 100ms 合并窗口内收敛多次变更后读取 SQLite 并广播，避免把实时查询放进代理上游派发和首字节关键路径。
- 已实现：`dashboard-activity.summary` 的 `tokensPerMinute`、`spendRate` 与 in-progress 调用数由账号聚合结果求和得到；无账号流量进入 `unassigned` 聚合项，避免顶部数字无法由同屏明细解释。
- 已实现：via-pool 请求级 cleanup guard 在响应消费期间保留 `pool-via-*` synthetic runtime snapshot，并随最终 stream task 生命周期收口；成功、失败、所有重试耗尽、下游断开或任务取消后清除残留非终态 snapshot，单次 upstream attempt 终结不会提前移除；普通 invocation runtime 与短暂终态 overlay 仍由其原有终态持久化路径负责。
- 已实现：timeseries 继续只服务趋势图与兼容回退，不再作为 Dashboard 顶部当前速率类 KPI 的事实来源。
- 已实现：账号活动快照的终态 live 数据改为账号级窄聚合与按模型用量分组，避免为整个 range 传输完整 invocation preview 行；运行态 runtime overlay、归档折叠、四个时间范围和公开响应字段保持原有语义。
- 已实现：账号卡 recent 调用改为每个候选账号按时间倒序的受限读取，数量仍严格受请求 `recentLimit` 限制。
- 已实现：账号卡 recent 调用在 SQLite batch flush 前也会读取同一 runtime store；同键 runtime 行覆盖非终态 DB shell，短暂 terminal overlay 立即可见，落库后不会形成重复行。

## Remaining Gaps

- 无功能性缺口。提交前仅需完成最终 review、测试收口与截图提交授权确认。

## Related Changes

- `src/api/slices/invocations_and_summary.rs`
- `src/api/slices/settings_models_and_cache.rs`
- `src/app_state.rs`
- `src/proxy/request_entry.rs`
- `src/proxy/route_selection.rs`
- `src/maintenance/hourly_rollups.rs`
- `web/src/pages/Dashboard.tsx`
- `web/src/features/dashboard/DashboardActivityOverview.tsx`
- `web/src/features/dashboard/DashboardWorkingConversationsSection.tsx`
- `web/src/components/InvocationErrorSummary.tsx`
- `web/src/features/dashboard/DashboardWorkingConversationsSection.stories.tsx`
- `web/src/features/dashboard/DashboardWorkingConversationsSection.test.tsx`
- `web/src/hooks/useDashboardUpstreamAccountActivity.ts`
- `web/src/lib/api/core-foundation.ts`
- `web/src/features/dashboard/DashboardPage.stories.tsx`

## References

- `./SPEC.md`
- `./HISTORY.md`

### Dashboard workspace card sorting

- Implemented independent persisted sort state for conversations and upstream accounts with a four-step cycle: created time, latest invocation, cost, and tokens.
- Added stable secondary keys, missing-time-last handling, accessible cycle control, and immediate derived reordering without changing SSE or reconcile cadence.
- Dashboard activity API now exposes `latestConversationCreatedAt` and `lastInvocationAt` from aggregate data for account sorting.
- Verification: Rust aggregation test, Dashboard hook/unit tests, Storybook test suite, and web production build pass.
