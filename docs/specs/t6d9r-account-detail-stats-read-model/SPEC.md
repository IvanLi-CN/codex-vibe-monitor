# 账号详情统计 read-model 与 3 秒准确展示 SLA（#t6d9r）

## 背景 / 问题陈述

- 账号详情抽屉的统计面长期依赖在线重算：`/api/pool/upstream-accounts/window-usage`、账号维度 `/api/stats/summary`、`/api/stats/timeseries`、概览页账号活动总览都会在读路径上混合 live rows、archive overlap、hourly rollup 与内存聚合。
- 该读路径在生产数据量下已经退化为秒级到十秒级响应；现网日志已出现单次 20 个账号批量统计约 12 秒，直接导致详情抽屉打开后长时间空白。
- 现有 `upstream_account_usage_hourly` 只能覆盖部分数量类指标，不足以支撑详情页全部精确字段，尤其无法可靠承接 `firstResponseByteTotalP95Ms`。

## 目标 / 非目标

### Goals

- 为账号详情页所有统计面建立精确 read-model 读路径，避免在线扫描大段 `codex_invocations`、大 OR 窗口条件与 archive/live 回补拼装。
- 将准确性与时效性前移到写入、archive materialization 与启动追平阶段，确保统计面一旦触发加载，就在 3 秒内展示准确结果。
- 收紧前端详情抽屉的数据编排：保留懒加载，但禁止 roster 刷新、SSE、tab 切换与选中账号变化重复触发重型统计。
- 把账号详情统计的启动追平、接口预算与前端状态流转固定为可回归验证的契约。

### Non-goals

- 不改账号详情统计 read-model、概览统计 SLA、roster 刷新机制或其他 tab 的数据编排；请求 tab 的尝试列表可按 `z9h7v` 合同补充诊断筛选，但不纳入本 read-model 的统计口径。
- 不使用 stale cache、近似值或“先展示旧值再纠正”的首屏兜底。
- 不把全站 dashboard 统计统一重构为同一套 read-model，只修账号详情链路。
- 不变更 upstream account roster、分组、标签、路由与限流语义。

## 范围（Scope）

### In scope

- `actualUsage` 卡片的自然窗口汇总。
- `DashboardActivityOverview` 的账号维度 summary 与 trend / heatmap timeseries。
- 概览页账号活动总览。
- 账号统计 read-model 的 schema、增量维护、archive 重放、启动追平与 readiness 语义。
- 详情抽屉前端懒加载、请求去重、取消旧请求与 SSE / roster refresh 编排。

### Out of scope

- 记录表正文行数据本身的预物化。
- 非账号维度的全局统计面。
- archived 明细在线检索能力。

## Read-model 契约

### 数据模型

- 新增 `upstream_account_stats_hourly`，用于小时级范围聚合与长周期回读。
- 新增 `upstream_account_stats_minute`，用于自然窗口、详情页近端时间范围与边界精确拼装。
- 两层表都必须覆盖详情页实际消费的精确字段：
  - `total_count`
  - `success_count`
  - `failure_count`
  - `in_flight_count`
  - `total_tokens`
  - `input_tokens`
  - `output_tokens`
  - `cache_input_tokens`
  - `total_cost`
  - `first_byte_*`
  - `first_response_byte_total_*`
- `firstResponseByteTotalP95Ms` 不能退化成均值或最大值；bucket 内必须保留可重建该分位数的样本表示。

### 写入与补齐

- invocation 持久化成功后，同步通过 `INVOCATION_HOURLY_ROLLUP_TARGETS` 更新账号 minute/hourly read-model。
- archive materialization 与 live replay 必须通过同一套 `upsert_invocation_hourly_rollups_tx(...)` 目标集合写入账号统计层，禁止并行维护两套口径。
- 历史重建使用 `rebuild_upstream_account_stats_rollups_from_sources(...)`，先回放 archive，再回放 live rows，并将最终 live cursor 写回 `hourly_rollup_live_progress`。
- 任何重建或 replay 都必须幂等；重复执行不得双计数。

### 启动追平与 readiness

- 冷启动阻塞阶段必须先完成 schema ensure、账号统计表存在性修复、hourly/minute read-model bootstrap，再进入 HTTP ready。
- 若账号统计表缺失、为空或 live cursor 丢失，启动阶段必须先重建并保存 cursor，之后 readiness 才能通过。
- readiness 通过后的账号详情统计读取不得再依赖“在线全量回补”作为正确性来源；允许的 live tail 仅限基于已冻结 cursor 的有界补尾。

## 读路径契约

### Summary / timeseries

- 账号维度 summary 与 timeseries 默认只读 `upstream_account_stats_hourly` / `upstream_account_stats_minute`。
- 读路径允许的 raw invocation 补读只限两类：
  - bucket 边界上的精确补齐。
  - `hourly_rollup_live_progress` 之后的有界 live tail。
- 不允许再用“当前 retention cutoff 之后所以 live-only 足够”这种推断替代 read-model。

### Window usage

- `/api/pool/upstream-accounts/window-usage` 优先读取 minute read-model，再合并缺失 hourly usage rows 与 cursor 之后的 live raw tail。
- partial bucket 不得丢失，也不得因为 archive/live overlap 被双计数。
- 不允许按账号窗口构造大型 live SQL + 内存重算作为常规路径。

## 前端编排契约

- 详情抽屉允许懒加载 skeleton，但任何统计面一旦触发加载，展示出来的第一份数据必须是准确值。
- 首次打开详情时，统计面按需加载；不再因为 roster 首屏拉取、include-all、SSE 或列表刷新去 hydrate 当前页所有账号的 `window-usage`。
- invocation `records` SSE 只用于 records/live surfaces 的实时补丁；不得再作为账号池 roster、详情或 `window-usage` 的通用刷新触发器。
- 同账号重复请求必须去重。
- 切换账号、关闭抽屉或 query key 失效时，旧请求必须被取消或结果丢弃。
- SSE 与列表刷新最多触发一次受控详情刷新，不能叠加出重复重型请求。
- 请求 ID 历史定位必须由后端返回目标所在固定页窗口；前端以该页为锚点向新、旧两个方向按需加载，不得逐页扫描目标或预取无关页。
- 锚点窗口必须冻结到定位响应的 `snapshotId + anchorId` 并暂停 records SSE；`anchorId` 负责让双向分页复现定位时的 runtime overlay，用户返回最新请求后才恢复第一页与实时订阅。
- 账号详情请求 tab 中的请求摘要卡必须复用调用详情里的 pool attempt 摘要样式；`attemptId` 作为主请求 ID 保留醒目入口，次级调用短 ID 与模型映射在摘要卡内清晰可见，点击摘要卡即可展开对应详情面板。
- 账号详情请求 tab 顶部必须提供类型、模型与对话筛选控件；这些筛选只影响尝试列表正文，不改变概览统计、账号活动总览或其他 tab 的读模型。
- 外部传入 `focusedAttemptId` 时，前端必须先回到“全部类型 / 全部模型 / 全部对话”再调用定位接口，避免用户当前筛选把目标尝试隐藏。
- 程序化定位目标行必须使用不改变布局的单层短暂高亮，并抑制浏览器默认 outline，避免边框、阴影与焦点轮廓叠加。
- 桌面 overlay 模式的共享账号详情抽屉必须使用 `min(90rem, calc(100vw - 2rem))` 的确定宽度；加载状态、数据结算与 tab 切换不得改变抽屉宽度。紧凑视口页面化模式继续占满可用宽度。

## 验收标准（Acceptance Criteria）

- 打开账号详情抽屉后，`actualUsage`、概览页活动总览 summary 与活动趋势都必须在 3 秒内完成准确渲染。
- `/api/pool/upstream-accounts/window-usage` 不再走按账号批量 live 重算热路径；批量账号统计恢复到现有性能预算内，不再出现 10 秒级响应。
- 账号维度 summary / timeseries 对 success / failure / in-flight、tokens、cost、`firstResponseByteTotalAvgMs`、`firstResponseByteTotalP95Ms` 的结果，与既有精确口径逐项一致。
- 冷启动或 archive 回放后，read-model 未追平时 readiness 不得通过。
- 前端必须覆盖：首次懒加载、切换账号取消旧请求、SSE / roster refresh 不重复触发重型统计。
- 后端必须覆盖：增量维护、历史补齐幂等、boundary + live tail 精确性、cursor 恢复、接口只读 read-model 行为。
- 前端必须覆盖：锚点页虚拟定位、向上 prepend 保持视口、向下 append、稳定去重、返回最新恢复 SSE，以及定位 `404`/请求失败提示。
- 前端必须覆盖：桌面与移动布局均正确渲染请求摘要卡、点击摘要展开详情面板、目标项聚焦后自动展开且只出现一层清晰高亮。
- 前端必须覆盖：请求 tab 筛选控件常驻、筛选后空结果仍显示在列表 body 内、筛选后分页保留当前筛选、外部定位会清空筛选并定位目标。
- 前端必须通过真实浏览器布局覆盖：详情加载态、数据结算态及概览、请求、编辑、路由、健康与事件 tab 的抽屉宽度保持一致。
- 后端必须覆盖：账号作用域首/中/末页定位、runtime overlay、固定快照、账号不匹配和 retention 后未找到语义，且定位响应不得包含全量历史。

## Visual Evidence

- 账号详情抽屉固定桌面宽度（mock Storybook；`1920px` viewport 下抽屉保持 `1440px`，五个 tab 切换前后宽度一致）

![账号详情抽屉固定桌面宽度](./assets/detail-drawer-fixed-width-health-events.png)

- 账号详情请求 ID 锚点定位成功态（mock Storybook；冻结历史窗口、返回最新入口、目标行居中聚焦与非布局型高亮）

![账号详情请求 ID 锚点定位成功态](./assets/detail-drawer-invocation-locate-success.png)

- 账号详情请求 ID 未找到态（mock Storybook；停留请求 tab，警告包含目标 ID 且获得焦点）

![账号详情请求 ID 未找到态](./assets/detail-drawer-invocation-locate-not-found.png)

- 账号详情请求 ID 移动端定位态（mock Storybook；ID 保持单行完整显示，抽屉无横向溢出）

![账号详情请求 ID 移动端定位态](./assets/detail-drawer-invocation-locate-mobile.png)

- 详情抽屉概览页活动总览（mock Storybook；账号活动总览已归属概览页，记录页不再承载统计图表）

![账号详情概览页活动总览](./assets/detail-drawer-overview-activity-overview.png)

- 详情抽屉请求 tab 摘要卡列表（mock Storybook；请求 tab 移除外层 records 卡片、标题说明与记录数量选择，直接复用 pool attempt 摘要卡列表）

![账号详情请求 tab 摘要卡列表](./assets/detail-drawer-records-bare-table.png)

- 详情抽屉请求 tab 分页加载（mock Storybook；固定页大小加载后切换下一页，保留尝试请求摘要卡列表）

![账号详情请求 tab 无限滚动](./assets/detail-drawer-records-infinite-scroll.png)

- 详情抽屉记录页默认态（mock Storybook，记录页 tab 已选中；右上角请求日志证明此时只读取详情与账号 stats，不再额外预取 roster `/api/pool/upstream-accounts` 或 sticky `/sticky-keys`）

![账号详情记录页请求收口](./assets/detail-drawer-records-request-gating.png)

- 详情抽屉路由页按需加载（mock Storybook，从记录页切到路由页后才触发 roster、sticky keys 与 window usage 的受控请求）

![账号详情路由页按需加载](./assets/detail-drawer-routing-request-gating.png)

- 详情抽屉请求 tab 稳定态（mock Storybook；记录列表已进入 settled state，请求 tab 只保留尝试请求摘要卡列表，不再承载账号活动总览）

![账号详情记录页实时推送稳定态](./assets/detail-drawer-records-live-sync-stable.png)

- 详情抽屉请求 tab 诊断筛选态（Storybook state gallery；默认态、图片类型筛选、模型筛选、对话筛选与筛选后空结果均保持筛选条常驻并复用同一 attempts API）

PR: include
![详情抽屉请求 tab 诊断筛选默认态](./assets/detail-drawer-records-filter-gallery-default.png)

PR: include
![详情抽屉请求 tab 图片类型筛选态](./assets/detail-drawer-records-filter-gallery-image.png)

- 详情抽屉请求 tab 宽屏放宽态（mock Storybook；共享抽屉桌面宽度提升到 `90rem` 后，请求摘要卡列表有更稳定的阅读空间，不再被旧的 `60rem` 壳层过早压缩）

![账号详情记录页宽屏放宽态](./assets/detail-drawer-records-settled-wide.png)

- 详情抽屉概览页活动总览窄宽度 token 标签稳定态（mock Storybook；总 token 指标标题从 `今日 Tokens` 缩短为 `今日 Token`，并保持单行显示，不再在窄卡片里被拆成两行）

![账号详情记录页 token 标签单行态](./assets/detail-drawer-records-token-label-nowrap.png)

- 全站列表 body 初始错误态（mock Storybook；首屏无已有数据时，错误信息和重试入口展示在列表 body 内，而不是漂在列表外层）

![列表 body 初始错误态](./assets/list-body-state-initial-error.png)

## 参考

- `docs/specs/9aucy-db-retention-archive/SPEC.md`
- `docs/solutions/performance/rollup-backed-summary-window-consistency.md`
- `src/api/slices/prompt_cache_and_timeseries/summary_queries.rs`
- `src/api/slices/prompt_cache_and_timeseries/timeseries.rs`
- `src/upstream_accounts/sync_account_imports_tags.rs`
- `src/upstream_accounts/sync_group_sessions.rs`
- `web/src/hooks/useUpstreamAccounts.ts`
