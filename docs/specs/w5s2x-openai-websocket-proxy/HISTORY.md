# OpenAI 兼容 WebSocket 代理演进记录（#w5s2x）

## 2026-05-04

- 建立 WebSocket 代理 topic spec：明确只扩展 OpenAI 兼容 `/v1/*` pool proxy，不替换 Dashboard `/events` SSE。
- 修正账号池合同：WS 上游握手失败必须在代理内部按号池切换候选，不能依赖 downstream 客户端重连；已建立隧道后的透明换号仍明确为非目标。
