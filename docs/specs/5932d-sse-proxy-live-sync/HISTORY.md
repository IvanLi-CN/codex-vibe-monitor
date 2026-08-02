# 主应用常驻订阅纯 SSE 化与统一快照/回放基础设施 - History

- 2026-07-28：线上复查发现原 write-side Dashboard 读模型仍无界保留完整 terminal records，且 reconcile 在并发写入后丢弃完整构建并立即重试。合同因此收紧为 compact delta 双硬限、持久化 ACK/cursor 安全回收、同事务 baseline cursor 与 60 秒 reconcile；5 秒 cadence 只允许内存发布，rolling 窗口由 expiry delta 保持精确。
- 2026-07-25：Dashboard open-range 终态累计从“5 秒 topic refresh 失效 DB cache 后重建”改为 write-side idempotent delta。`today / 1d / 7d` 的 topic 与 HTTP 共享同一 warm baseline，5 秒窗口仅合并发布，60 秒 cadence 才进行 DB reconcile；reconcile 失败继续发布 last-good totals 与 runtime live overlay。
- `stats.timeseries.open-window` uses exact complete-minute projection entries with a bounded post-cursor tail. This retains point and P95 semantics while avoiding repeated full live invocation hydration for covered open windows.

## Key Decisions

- 2026-07-24：线上复查确认 Summary topic 的主要残留压力来自 terminal follow-up 对 `all/30m/1h/1d/1mo` 五个 legacy 窗口的无关重建，以及 `today` summary 的 `non_success_tokens` 仍走整窗账号活动聚合。本轮删除生产 Summary follow-up，open-range topic 改为 live overlay + 固定 `500ms` totals coalescer；`non_success_tokens` 复用 hourly v2 rollup 与 boundary scalar tail。Dashboard 保留既有 `5s` TTL，仅改用真实 owner subscriber lease 门控，并以 dirty reconnect 保证失活期间不回放旧连续性。
- 2026-07-24：补充闭区间防护：`yesterday` / `previous7d` 即使有遗留兼容 topic，也不再被 Records 或 live 广播触发重建；当前应用继续通过 HTTP exact path 获取闭区间结果。
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
