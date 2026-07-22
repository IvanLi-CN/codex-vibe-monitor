# Dashboard 上游真值网速与活动总览 Network Tab 实现说明

## 后端

- 新增 `src/dashboard_network_speed.rs`，集中维护：
  - global、host、account 三维原始秒桶与上一完整秒快照读取路径。
  - 当前开放 5 分钟桶的上传/下载累计。
  - invocation 级连接字节追踪、终态清理与 Dashboard live stream 心跳预算。
- 代理热路径在以下时机写入缓存：
  - 上游 HTTP / WebSocket socket 实际写入时记录上传字节，并同时写入 global、host、account 三个维度。
  - 上游 HTTP / WebSocket socket 实际读取时记录下载字节，并同时写入 global、host、account 三个维度。
  - transport future 终态或 drop 时做最终 flush，避免 timeout / early close 吃掉最后一段真实字节。
- pool attempt 持久化新增 `upstream_base_url_host`；direct 路径继续从 invocation payload 中读取 `upstreamBaseUrlHost`。
- 新增 `upstream_socket_network_minute`，按 `(bucket_start_epoch, source, upstream_base_url_host, upstream_account_id)` 持久化真实 socket 分钟累计。
- socket minute materializer 复用现有 live replay 框架，但首次只把 cursor seed 到当前 live table 尾部，不做历史回填。
- `AppState` 新增 `process_started_at_utc` 与 `dashboard_network_speed_cache`，runtime 初始化时统一挂载。
- `GET /api/stats/dashboard-activity` / `dashboardActivityLive` 在兼容保留账号级速率字段的同时，新增全局 `networkLiveBucket` 与 `networkRealtimeRate`。
- 新增 `GET /api/stats/dashboard-network-timeseries`：
  - 只接受 `today | yesterday | 1d`。
  - 固定返回 5 分钟桶。
  - 无 scope 时闭合桶从 `upstream_socket_network_minute` 聚合，当前开放桶读全局 live bucket。
  - `upstreamAccountId` 存在时同样改读 `upstream_socket_network_minute` 的账号 scoped 聚合。
- `DashboardNetworkSpeedCache` 的全局秒桶保留窗口扩展到完整 300 秒，并新增 recent snapshot builder：
  - 固定返回最近 300 个上一完整秒样本。
  - 进程启动前的前导区间统一标记 `isAvailable=false`，数值清零但不代表真实零流量。
  - recent 秒级历史只保留运行期内存窗口，不写 SQLite，也不从分钟表反推。
- 新增 `GET /api/stats/dashboard-network-recent` 与 `dashboard.network-recent.current`：
  - 由后端直接组装 `DashboardRecentNetworkWindowResponse` / `DashboardRecentNetworkWindowPoint`。
  - topic payload 与 HTTP response 共用同一权威读模型，固定 `windowSeconds=300`、`sampleSeconds=1`。
  - 只要存在 `dashboard.network-recent.current` 订阅者，服务端由 `SubscriptionHub` 按 topic 共享 1 秒 cadence 推送 live payload，用于推进 recent 窗口右边界；前端不再通过 `refresh()` 维持 steady-state。
  - 共享 cadence 由 topic 订阅 lease 计数驱动，多条 SSE 连接订阅同一 topic 时只保留一个 server-push task，避免连接数放大 payload build / broadcast。
- `src/oauth_bridge.rs` 新增 counted OAuth transport path，避免 OAuth HTTP 请求继续走 reqwest body 近似值。
- `src/proxy/upstream_transport.rs` 提供 counted HTTP transport，`src/proxy/websocket.rs` 复用同一套 meter / reporter，统一 direct、pool、OAuth 与 WebSocket 的真实网速事实源。

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
- `useDashboardUpstreamAccountActivity` 与相关 normalize 逻辑把 `networkLiveBucket` 和 `networkRealtimeRate` 一起透传给上游账号 tab。
- `Dashboard.tsx` 页面壳层也必须把 `dashboardActivity.networkRealtimeRate` 继续下传给 `DashboardWorkingConversationsSection`；否则即使 `/api/stats/dashboard-activity` 已返回非零 realtime 快照，工作区顶部总速率胶囊仍会退回 `0 B/s`。
- `DashboardWorkingConversationsSection` 同时在两个位置展示网速：
  - 工作区右上 badge 区展示所有上游请求的全局总上/下行实时速率，直接读取 `networkRealtimeRate`，口径固定为上一完整 1 秒。
  - 账号卡标题区删除单账号上传/下载速率，只保留活动账号数量、TPM、消费速率、进行中等摘要。
- 新增 `dashboardNetworkFormatting.ts` 统一处理 `B/s / KiB/s / MiB/s` 与字节单位格式化。
- 新增 `web/src/hooks/useDashboardRecentNetworkWindow.ts`：
  - 只消费 `dashboard.network-recent.current` topic。
  - 仅在 recent 面板打开期间订阅服务端 push，关闭即停止订阅。
  - 不再设置前端 interval，也不在 steady-state 或手动 reload 中调用 `subscription.refresh()`。
  - 以最后一次 topic payload 到达时间判断 stale；超过 5 秒未收到 payload 时让 panel 显示图表级 Loading/Spinner 遮罩。
- 新增 `web/src/features/dashboard/DashboardNetworkRecentPopover.tsx`：
  - 以 `NetworkSpeedInline` 作为唯一触发器。
  - 网速胶囊不再设置 `title`，避免浏览器原生 tooltip 覆盖悬浮面板。
  - 桌面端复用现有 popover chrome，实现 `hover 打开 + click 固定 + 再次点击/外点/Esc 关闭`。
  - 窄屏端改用 dialog/sheet 承载同一 panel 内容。
  - panel 右上角从最近一帧可用样本派生两行当前摘要：`上行：<speed>` 与 `下行：<speed>`；两行使用图表同源的上传蓝、下载绿裸文本编码，不再使用 chip 轮廓或额外卡片包裹。
  - 图表层将 `isAvailable=false` 样本渲染成空档；UI 不再显示 warming callout 或不可用点 tooltip，避免在窄屏面板中出现额外提示。
  - topic payload 超过 5 秒未同步时，仅图表区域显示 Loading/Spinner stale 遮罩；旧图保留，不再显示局部“刷新中”。
  - `DashboardNetworkRecentPanel` 的秒级图表 tick 计算改成普通派生值，避免组件从 loading 切到有数据时因条件分支后的额外 hook 触发 React hook order 崩溃。

## 测试与 Storybook

- 前端定向单测覆盖：
  - live snapshot 合并全局 `networkLiveBucket` 与 `networkRealtimeRate`。
  - Dashboard `network` metric 的可见性与 24 小时图切换。
  - 对话工作区右上总网速与账号卡速率删除断言。
- 前端新增 recent 面板单测覆盖：
  - `useDashboardRecentNetworkWindow` 的 topic descriptor、无前端 interval refresh、无手动 refresh、last payload stale 判断。
  - `DashboardNetworkRecentPopover` 的桌面 hover/click 固定、`Esc` 关闭、窄屏 dialog 打开、前导空档无提示、右上角上/下行摘要与 stale 遮罩渲染。
  - `DashboardNetworkRecentPanel` 从 loading 切到真实数据时的 hook order 回归，防止线上打开 recent 面板直接触发 React 310。
- 后端定向单测覆盖：
  - global/host/account runtime bucket 记账。
  - socket minute rollup 的 direct 写入、cursor seed 与 pool retry host split。
  - Dashboard 无 scope timeseries 改读 socket minute 5 分钟聚合。
  - counted HTTP、OAuth timeout / retry、WebSocket usage 持久化的真实字节计数。
- 后端新增 recent 面板定向单测覆盖：
  - 300 秒 recent 窗口保留与上一完整秒快照语义。
  - 进程启动不足 5 分钟时 recent 前导空档的 `isAvailable=false` 语义。
  - recent endpoint response builder、subscription topic schema epoch、服务端 live payload 推送与多订阅者共享 cadence。
- `DashboardPage.stories.tsx` 新增页面级 SSE / HTTP bootstrap，确保整页证据能同时覆盖活动总览网速图和上游账号顶部总速率胶囊，不再依赖缺失首帧 snapshot 的假空态。
- Storybook 继续使用 `DashboardNetworkActivityChart` 与 `UpstreamAccountTab` 场景验证图表背景与账号 tab 顶部总速率展示，并补充整页 `UnifiedActivitySnapshot` 证据验证 page-shell 接线。
- 新增 `DashboardNetworkRecentPopover.stories.tsx`，提供桌面固定展开态、stale 遮罩态与窄屏 sheet 前导空档无提示态，作为 recent 诊断面板的稳定视觉证据入口。
