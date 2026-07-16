# Dashboard 上游账号网速与活动总览 Network Tab 实现说明

## 后端

- 新增 `src/dashboard_network_speed.rs`，集中维护：
  - 账号级最近 15 秒秒桶滚动窗口。
  - 当前开放 5 分钟桶的上传/下载累计。
  - invocation 级 request/response 字节追踪、终态清理与 Dashboard live stream 心跳预算。
- 代理热路径在以下时机写入缓存：
  - 请求体大小确定后记录上传字节。
  - 每次转发响应 chunk 时记录下载字节。
  - invocation 终态清理时回收 runtime 跟踪状态，避免重复计数。
- `AppState` 新增 `process_started_at_utc` 与 `dashboard_network_speed_cache`，runtime 初始化时统一挂载。
- `GET /api/stats/dashboard-activity` / `dashboardActivityLive` 扩展账号级实时上传/下载速率字段。
- 新增 `GET /api/stats/dashboard-network-timeseries`：
  - 只接受 `today | yesterday | 1d`。
  - 固定返回 5 分钟桶。
  - 闭合桶从 `codex_invocations` 聚合应用层字节。
  - 当前开放桶先做一次 lazy seed，再以内存覆盖末桶。

## 前端

- 新增 `web/src/hooks/useDashboardNetworkTimeseries.ts`，仅在 `network` metric 激活时加载。
  - 首次通过 HTTP hydrate 全量 5 分钟桶。
  - `today / 1d` steady-state 改为消费 `dashboardActivityLive` SSE 推送的当前开放桶。
  - 桶切换或 SSE 重连时只做静默回补，不再每秒轮询整段时序。
- 新增 `web/src/features/dashboard/DashboardNetworkActivityChart.tsx`，使用 Recharts 双面积图展示上传/下载速率。
- `DashboardActivityOverview`：
  - `today / yesterday` 的 metric toggle 新增 `network`，保留 `trend`。
  - `24 小时` 新增 `network`，选中时用网速面积图替换 heatmap。
  - `7 日 / 历史` 保持原行为不变。
- `useDashboardActivitySnapshot` 调整为“账号汇总常开、最近调用按账号 tab 按需加载”：
  - Dashboard 页面始终拿到账号级实时速率，保证对话工作区右上角总网速可见。
  - `recentInvocations` 仍只在上游账号 tab 激活时补拉，避免把最近调用查询常驻到非账号视图。
- `DashboardWorkingConversationsSection` 同时在两个位置展示网速：
  - 工作区右上 badge 区展示所有上游账号请求的总上/下行实时速率。
  - 账号卡标题区保留单账号实时网速，继续与进行中调用 / TPM / 消费速率并列。
- 新增 `dashboardNetworkFormatting.ts` 统一处理 `B/s / KiB/s / MiB/s` 与字节单位格式化。

## 测试与 Storybook

- 前端定向单测覆盖：
  - live snapshot 合并账号级网速字段。
  - Dashboard `network` metric 的可见性与 24 小时图切换。
  - 对话工作区右上总网速与账号卡标题区网速行渲染。
- Dashboard Activity Overview Storybook mock API 新增 dashboard-only 网速时序响应，并补充 `TodayNetworkView` / `Day24NetworkView` 场景。
- Dashboard Working Conversations Storybook 新增 `ConversationTabWithUpstreamNetworkSpeed`，并更新 `UpstreamAccountTab` 断言覆盖 header 总网速与账号卡网速。
