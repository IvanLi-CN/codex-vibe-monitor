# Dashboard 上游真值网速与活动总览 Network Tab 演进记录

## 关键决策

- 选择 HTTP 近似真值而不是 network-layer 带宽作为事实源：只记最终可见 header 字段与实际 body 字节，不把 TLS / HTTP/2 framing 等低层开销混入口径。
- 活动总览大图继续采用 5 分钟历史桶，但顶部总速率胶囊改成“上一完整 1 秒”的原始速率；runtime cache 继续承载 `global + host + account` 三维记账。
- pool retry 的上传/下载必须按 attempt 累加；只按 invocation 记账会漏掉跨 host 重试流量，也无法支撑 host 维度分钟 rollup。
- host minute rollup 选择单独建表 `upstream_host_network_minute`，并复用既有 live replay/materializer；首次只 seed cursor 到当前 live tail，不做历史回填。
- Dashboard 无 scope 网速改成“系统对所有上游”的全局真值，闭合历史桶从 host minute rollup 汇总，live 末桶继续读 runtime global bucket。
- 工作区标题区的总速率胶囊保留，但改读全局 `networkRealtimeRate`；账号卡中的上传/下载速率删除，避免让 owner 把账号卡局部速率误读成系统总量。
- 顶部网速图的 steady-state 更新继续保持“首次 hydrate + `dashboardActivityLive` SSE 推送当前桶”，只在桶切换 / SSE 重连时静默回补。
- 2026-07-18 上线后发现一个 page-shell 漏接线问题：工作区总速率胶囊消费的全局速率字段没有被 `Dashboard.tsx` 继续下传，导致线上胶囊长期显示 `0 B/s`。修复后补充页面级回归测试与整页 Storybook bootstrap，防止“接口有值、页面为零”的回归再次漏过。
- 2026-07-19 在 Docker 部署诊断中确认：HTTP 近似真值仍会让 OAuth HTTP 与 WebSocket 的 live 网速出现漏记或 `0 B/s` 假零值，因此事实源切换为连接级真实 socket 字节。
- 真实网速实现保留 host + account 归因，但 Dashboard 展示仍只暴露全局总量与账号 scoped 视图；不新增 host UI。
- 为避免 timeout / cancel 导致已发生的真实流量被吞掉，HTTP counted transport 与 WebSocket flush task 都补了 drop-time final flush。
- 2026-07-19 进一步确认：15 秒 SMA 不符合“实时网速”语义，因此顶部胶囊改为读取上一完整 1 秒快照；历史图继续显示 5 分钟桶，并移除左上角误导性说明文案。
- 2026-07-20 为排查“网速看起来诡异”的现场问题，工作区顶部总速率胶囊新增 recent 诊断面板，但不改胶囊本身的 `networkRealtimeRate` 口径。
- 2026-07-20 recent 诊断面板选择独立的全局 300 秒、1 秒粒度读模型，由 `DashboardNetworkSpeedCache` 直接产出，并通过 `GET /api/stats/dashboard-network-recent` 与 `dashboard.network-recent.current` 暴露；不复用分钟桶，也不新增 SQLite 落盘。
- 2026-07-20 recent 面板前端明确遵守 topic-only SSE 约束：只消费 `dashboard.network-recent.current`，仅在面板打开期间以 1 秒 cadence refresh，同屏不再引入 open-resync 或第二真相源。
- 2026-07-20 进程启动不足 5 分钟时，recent 面板前导区间统一显示为 `isAvailable=false` 空档并标记 warming；这些空档不再伪装成真实 `0 B/s` 历史。
- 2026-07-20 线上验证发现：recent 面板首次打开会先经过 loading 分支，再切到真实图表数据；原实现把一个 `useMemo` 放在早退分支之后，导致同一组件实例在 `loading -> data` 间触发 React hook order 崩溃（线上表现为 React 310）。修复改为无条件的普通 tick 派生计算，并补回归测试锁住这个路径。
- 2026-07-22 recent 面板的 steady-state 从前端 1 秒 `refresh()` 改为服务端 SSE live payload 推送；前端只消费 topic payload，并用最后一次 payload 到达时间判定 5 秒 stale 遮罩，避免出现“刷新中”局部状态和第二真相源。
- 2026-07-22 网速胶囊移除浏览器原生 `title` tooltip；recent 面板头部右上角改为显示最近一帧可用样本的两行上行/下行摘要，让 owner 不需要依赖 hover tooltip 读取当前值。
- 2026-07-22 窄屏 recent 面板不再显示“正在积累 5 分钟历史”类 warming 提示；前导不可用样本继续通过 `isAvailable=false` 渲染为空档，但 UI 不额外弹出提示。
- 2026-07-22 PR merge-proof 发现连接级 recent cadence 会在多 tab 下放大 payload build / broadcast；实现收敛为 `SubscriptionHub` topic 级共享 server-push task，并用订阅 lease 计数停止闲置 task。
