# Dashboard 上游真值网速与活动总览 Network Tab 实现说明

## 后端

- 新增 `src/dashboard_network_speed.rs`，集中维护：
  - global、host、account 三维最近 15 秒秒桶滚动窗口。
  - 当前开放 5 分钟桶的上传/下载累计。
  - invocation 级 request/response 字节追踪、终态清理与 Dashboard live stream 心跳预算。
- 代理热路径在以下时机写入缓存：
  - 请求体大小确定后记录上传字节，并同时写入 global、host、account 三个维度。
  - 每次转发响应 chunk 时记录下载字节，并同时写入 global、host、account 三个维度。
  - invocation 终态清理时回收 runtime 跟踪状态，避免重复计数。
- pool attempt 持久化新增 `upstream_base_url_host`；direct 路径继续从 invocation payload 中读取 `upstreamBaseUrlHost`。
- 新增 `upstream_host_network_minute`，按 `(bucket_start_epoch, source, upstream_base_url_host)` 持久化 host 维度分钟累计。
- host minute materializer 复用现有 live replay 框架，但首次只把 cursor seed 到当前 live table 尾部，不做历史回填。
- `AppState` 新增 `process_started_at_utc` 与 `dashboard_network_speed_cache`，runtime 初始化时统一挂载。
- `GET /api/stats/dashboard-activity` / `dashboardActivityLive` 在兼容保留账号级速率字段的同时，新增全局 `networkLiveBucket`。
- 新增 `GET /api/stats/dashboard-network-timeseries`：
  - 只接受 `today | yesterday | 1d`。
  - 固定返回 5 分钟桶。
  - 无 scope 时闭合桶从 `upstream_host_network_minute` 聚合，当前开放桶读全局 live bucket。
  - `upstreamAccountId` 存在时保留既有账号 scoped 查询路径。

## 前端

- 新增 `web/src/hooks/useDashboardNetworkTimeseries.ts`，仅在 `network` metric 激活时加载。
  - 首次通过 HTTP hydrate 全量 5 分钟桶。
  - `today / 1d` steady-state 改为消费 `dashboardActivityLive` SSE 推送的当前开放桶。
  - 桶切换或 SSE 重连时只做静默回补，不再每秒轮询整段时序。
- 新增 `web/src/features/dashboard/DashboardNetworkActivityChart.tsx`，使用 Recharts 双面积图展示上传/下载速率。
- `DashboardNetworkActivityChart` 的 panel 背景改成中性 surface，不再给整块图表容器染青绿色底色。
- `DashboardActivityOverview`：
  - `today / yesterday` 的 metric toggle 新增 `network`，保留 `trend`。
  - `24 小时` 新增 `network`，选中时用网速面积图替换 heatmap。
  - `7 日 / 历史` 保持原行为不变。
- `useDashboardUpstreamAccountActivity` 与相关 normalize 逻辑把新的 `networkLiveBucket` 完整透传给上游账号 tab。
- `DashboardWorkingConversationsSection` 同时在两个位置展示网速：
  - 工作区右上 badge 区展示所有上游请求的全局总上/下行实时速率，直接读取 `networkLiveBucket`。
  - 账号卡标题区删除单账号上传/下载速率，只保留活动账号数量、TPM、消费速率、进行中等摘要。
- 新增 `dashboardNetworkFormatting.ts` 统一处理 `B/s / KiB/s / MiB/s` 与字节单位格式化。

## 测试与 Storybook

- 前端定向单测覆盖：
  - live snapshot 合并全局 `networkLiveBucket`。
  - Dashboard `network` metric 的可见性与 24 小时图切换。
  - 对话工作区右上总网速与账号卡速率删除断言。
- 后端定向单测覆盖：
  - global/host/account runtime bucket 记账。
  - host minute rollup 的 direct 写入、cursor seed 与 pool retry host split。
  - Dashboard 无 scope timeseries 改读 host minute 5 分钟聚合。
- Storybook 继续使用 `DashboardNetworkActivityChart` 与 `UpstreamAccountTab` 场景验证图表背景与账号 tab 顶部总速率展示。
