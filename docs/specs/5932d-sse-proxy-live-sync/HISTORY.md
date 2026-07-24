# 主应用常驻订阅纯 SSE 化与统一快照/回放基础设施 - History

## Key Decisions

- 2026-07-24：离线黄条的掉线时长保留既有 SSE 状态与翻译计算，只改为标题旁紧凑等宽纯文本，避免在 warning 容器上再叠加半透明胶囊背景。AppLayout Storybook 通过仅用于故事的状态上下文稳定提供断线诊断数据，桌面与 `390px` 移动状态复用同一断线 fixture。
- 2026-07-20：`stats.summary.current` 的 open-range 残留慢链从旧 HTTP summary 构建器完全收口到共享内部 builder；同轮把 `usage_breakdown` 和 `non_success_tokens` 改成 live/archive aggregate merge，去掉 `full_range_preview_rows(limit=None)` 与 live invocation id overlap 全窗扫描，避免 topic SSE 与 Dashboard 7d overview 再次把 summary 读压打回 SQLite。
- 2026-07-17：手动“立即重连”被收紧为同页 fresh snapshot 恢复，而不是“复用旧 resume 的软重连”或整页刷新。前端现在为每次连接分配 `attempt` 和 `reason`，手动重连会对当前 active topics 全量 forced snapshot，并把同一轮证据同时暴露到黄条诊断文本与后端 `/events` 初始化日志。
- 2026-07-17：浏览器 drill 暴露出一个更底层的缺口：等价 topic descriptor 在 React 重渲时会反复退订/重订，叠加 `eventsource-error` 的立即重连，能把 `attempt` 冲到数千次。现已把订阅稳定性下沉到 `useSubscriptionTopic` 的语义 key，并把失败恢复重新收紧为指数退避。
- 2026-07-16：主应用常驻订阅从“`records` SSE + HTTP bootstrap/open-resync/reconcile + 页面私有 fallback”一次性切到单 `/events` 的 topic SSE 合同。覆盖范围内连接只消费 `snapshot/replay/live` envelope；恢复只走 replay 或新 snapshot，不再偷偷打 HTTP。
- 2026-07-16：订阅 topic 被定义为权威读模型，而不是前端二次聚合状态机。`dashboard.activity`、working conversations、summary、timeseries、parallel-work、prompt-cache、quota、forward-proxy live 等当前态统一以后端 topic payload 为真相源。
- 2026-07-16：replay 保留层明确为进程内有界窗口，不做跨重启持久化。服务重启、schema epoch 变化、topic 参数变化与 gap 超预算都统一降级为发送新 snapshot。
- 2026-07-16：端到端 drill 暴露出两个真实收口缺口，并在同轮修复：一是主应用 shell 仍额外拉 `/api/version`，现已改为纯 `app.version` topic；二是后端 envelope 实际发送 `topic_key/schema_epoch`，前端纯 SSE 消费器只认 camelCase，现已统一对外发 `topicKey/schemaEpoch`，并保留前端兼容读取。
- 2026-07-13：Dashboard 账号活动已先从“收到 `records` 就重查 HTTP”收敛为后端权威当前态快照，为后续纳入统一 topic SSE 总线提供了读模型基础。
- 2026-07-03 到 2026-07-05：runtime invocation store、admit-time running shell、terminal overlay 与 write-controller 分层完成，确保“当前进行中真相”可以通过统一读模型与 SSE 暴露，而不是依赖同步落库。
- 2026-06-21：活动调用记录列表曾统一收口到 `records` SSE + open 后静默回源；这一阶段解决了列表实时性，但仍保留了主应用订阅面的大量混合推拉语义，现已被 topic SSE 方案取代。

## Replacements

- 旧合同：`records` 事件通知页面自行回源
  - 新合同：topic authoritative payload + `snapshot/replay/live`
- 旧合同：SSE 重连后统一 HTTP open-resync
  - 新合同：cursor + `schemaEpoch` 驱动 replay，失败则 snapshot
- 旧合同：健康态定时 reconcile 校准主应用订阅 UI
  - 新合同：健康态只消费 SSE topic；HTTP 仅保留给闭合历史窗口与非订阅页面

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
