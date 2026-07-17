# 主应用常驻订阅纯 SSE 化与统一快照/回放基础设施 - Implementation

## Current State

- Canonical spec: `docs/specs/5932d-sse-proxy-live-sync/SPEC.md`
- Implementation summary: 已实现，待最终提交收口
- Lifecycle: active

## Coverage / rollout summary

- 已实现：后端新增 `SubscriptionHub`、`SubscriptionTopicDescriptor`、`SubscriptionResumeCursor` 与 `SubscriptionEventEnvelope`，主应用 `/events` 统一切到 topic-aware `snapshot/replay/live` 合同。
- 已实现：`topics` / `resume` 查询参数支持 JSON 与 base64url 编码；`topic_key` 基于 canonicalized descriptor 稳定生成。
- 已实现：后端为每个 topic 维护最近 authoritative snapshot 与进程内 replay ring，恢复规则统一为“能 replay 才 replay，否则新 snapshot”。
- 已实现：replay 窗口默认限制为最近 `60s`、每 topic `512` 个事件、约 `1 MiB` 数据；单次恢复 gap 超过 `128` 个事件或约 `256 KiB` 时直接 snapshot。
- 已实现：`src/runtime.rs` 在应用启动时挂载主应用订阅 hub，并把内部广播桥接到受影响 topic 的 snapshot refresh / live fanout。
- 已实现：前端 `web/src/lib/sse.ts` 改为单连接 topic registry，持有 topic cursor、descriptor 集与连接状态，并在 topic 集变化时重连。
- 已实现：前端 SSE registry 额外持有 `attempt/reason/activeTopics/resumeTopics/forcedSnapshotTopics` 等 diagnostics 状态；`AppLayout` 黄条直接暴露最近连接证据，便于 owner-facing 判责。
- 已实现：新增 `web/src/hooks/useSubscriptionTopic.ts`，统一封装 topic 级缓存、初始 loading 与手动 refresh。
- 已实现：手动“立即重连”改为当前 active topics 全量 forced snapshot 的同页恢复路径；新的 `/events` 连接附带 `attempt` 与 `reason=manual`，而不是复用旧 `resume`。
- 已实现：后端 `/events` 接受可选 `attempt/reason` 诊断参数，并在连接初始化日志里输出每个 topic 的 `replay_hit / resume_caught_up / snapshot_no_resume / snapshot_resume_miss` 结果。
- 已实现：`useSubscriptionTopic` 改为按 descriptor 语义 key 而不是对象引用决定是否重订阅，避免等价 topic 在 React 重渲时反复触发 `topic-change` 重建。
- 已实现：`eventsource-error` / watchdog 失败恢复改回统一指数退避；只有手动重连与真实 topic 变更走立即重建，避免 `attempt` 在断线时高频自旋。
- 已实现：开发环境把当前页使用中的 SSE 单例以 `window.__CVM_SSE__` 暴露出来，仅用于浏览器 drill 与诊断，不进入生产路径。
- 已实现：`AppLayout` 版本信息切到 `app.version` topic，主应用 shell 不再额外打 `/api/version` 作为首屏 bootstrap。
- 已实现：订阅 envelope 统一以 camelCase `topicKey/schemaEpoch` 对外发送；前端消费层同时兼容历史 `topic_key/schema_epoch`，避免灰度期间把 authoritative snapshot 吞掉。

## Migrated consumers

- `quota.current`
- `forward-proxy.live`
- `stats.parallel-work.current`
- `app.version`
- `prompt-cache.window`
- `prompt-cache.sticky.window`
- `invocations.window`
- `stats.summary.current`（开放窗口）
- `stats.timeseries.open-window`（开放窗口）
- `dashboard.activity.current`
- `dashboard.working-conversations.current`
- `invocation.pool-attempts`

## Removed mixed-mode behavior

- 已移除：覆盖范围内页面首屏先发 HTTP bootstrap。
- 已移除：`subscribeToSseOpen` 驱动的订阅类 open-resync fetch。
- 已移除：健康态 timer reconcile / 页面私有 fallback。
- 已移除：`records` 事件驱动 `dashboard.activity`、working conversations、summary、timeseries、parallel-work、prompt-cache 的额外重拉链路。
- 保留：闭合历史窗口、非订阅页面与任务型专用 SSE 的既有语义。

## Verification

- `cargo check`
- `cargo test subscriptions -- --nocapture`
- `cargo test subscription_event_envelope_serializes_camel_case_fields -- --nocapture`
- `cargo test replay_returns_gap_when_cursor_is_within_window -- --nocapture`
- `cargo test prepare_connection_reports -- --nocapture`
- `cd web && bun x tsc -b --pretty false`
- `cd web && bun x vitest run --project=unit src/lib/sse.test.ts src/features/app-shell/AppLayout.test.tsx`
- `cd web && bun x vitest run --project=unit src/lib/sse.test.ts src/hooks/useSubscriptionTopic.test.tsx src/features/app-shell/AppLayout.test.tsx`
- `cd web && bun run test -- useDashboardWorkingConversations.test.tsx useDashboardUpstreamAccountActivity.test.tsx useInvocations.test.tsx useStats.integration.test.tsx useTimeseries.integration.test.tsx`
- `cd web && bun run test -- src/features/app-shell/AppLayout.test.tsx src/hooks/useDashboardWorkingConversations.test.tsx src/hooks/useDashboardUpstreamAccountActivity.test.tsx src/hooks/useInvocations.test.tsx src/hooks/useStats.integration.test.tsx src/hooks/useTimeseries.integration.test.tsx src/lib/sse.test.ts src/hooks/useSubscriptionTopic.test.tsx`
- 浏览器侧 drill：`/#/live` 首屏 `apiCalls=[]`，只经 `/events` 接收 `snapshot` 完成 hydration；断线期间 `dashboard.activity.current` 新增 cursor 后，重连通过 `resume` 收到 `replay`，未触发额外 HTTP。
- 浏览器侧 drill：在同页阻断 `/events` 后，自动重试间隔观测为约 `4.1s -> 8.0s`，未再出现高频 `attempt` 风暴；解封后同页手动重连发出新的 `attempt=26&reason=manual`，后端日志对应 `resume_count=0`，随后状态恢复为 `connected`。

## Visual Evidence

- `assets/sse-offline-banner-desktop-reconnect.png`
- `assets/sse-offline-banner-mobile-reconnect.png`

## Related Changes

- `src/api/slices/subscriptions.rs`
- `src/api/slices/error_distribution_and_sse.rs`
- `src/app_state.rs`
- `src/runtime.rs`
- `web/src/lib/sse.ts`
- `web/src/hooks/useSseDiagnostics.ts`
- `web/src/hooks/useSubscriptionTopic.ts`
- `web/src/hooks/useSubscriptionTopic.test.tsx`
- `web/src/hooks/useDashboardUpstreamAccountActivity.ts`
- `web/src/hooks/useDashboardWorkingConversations.ts`
- `web/src/hooks/useInvocations.ts`
- `web/src/hooks/useInvocationRecordsRealtime.ts`
- `web/src/hooks/usePromptCacheConversations.ts`
- `web/src/hooks/useStats.ts`
- `web/src/hooks/useTimeseries.ts`

## Remaining gaps

- 无功能性缺口；提交前仅需完成最终验证与 review 收口。
