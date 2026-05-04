# OpenAI 兼容 WebSocket 代理演进记录（#w5s2x）

## 2026-05-04

- 建立 WebSocket 代理 topic spec：明确只扩展 OpenAI 兼容 `/v1/*` pool proxy，不替换 Dashboard `/events` SSE。
- 修正账号池合同：WS 上游握手失败必须在 downstream upgrade 前由代理内部按号池切换候选；已建立隧道后切号边界是服务端主动发送可重连 close，downstream 重连后重新进入号池调度。
- 增加双开关与账号 capability tag：downstream 是否允许 WS、proxy-to-upstream 是否默认使用 WS 分开控制；`unsupported_transport:websocket` 系统 tag 标记不支持 WS 的上游账号并强制其使用 HTTP 路径。
- 补齐成本与 UI 证据：Responses WS terminal usage 事件进入既有 invocation/cost 统计；设置页提供两个可保存的 WS 全局开关；账号池列表展示 `不支持 WS` 标签。
- 修正 WS 开关契约：环境变量只作为首次初始化默认值，后续由数据库中的全局设置控制，UI 保存后即时影响新连接。
- 修正 WS 运行时收敛：上游 WS 握手明确返回 403/404/405/426/501 时自动补写 `unsupported_transport:websocket` 系统 tag；upstream 终态帧先转发给下游，再执行 usage 持久化，避免 `response.completed` 被记账阻塞。
