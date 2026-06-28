# 请求日志可观测性增强（IP / Cache Tokens / 分阶段耗时 / Prompt Cache Key / Body Logging Toggles） - Implementation

## Current State

- Canonical spec: `docs/specs/z9h7v-invocation-log-observability/SPEC.md`
- Implementation summary: 已完成，且已扩展 request/response body logging 双开关
- 号池尝试详情会将真实上游请求尝试与 `budget_exhausted_final` / `sameAccountRetryIndex <= 0` 合成终态记录分开展示；终态记录只展示未发起新请求的终态说明与上一失败账号上下文。
- proxy settings 现在持久化 `request_body_logging_enabled` / `response_body_logging_enabled`；关闭后只阻止新的 raw body / response preview 留存，不影响结构化 payload、usage、timing、routing/account、prompt cache key 等字段。
- response body logging 关闭时，运行态记录与终态持久化都会将 `raw_response` preview 置空，并跳过 `response_raw_path` 元数据。
- invocation payload 现在额外携带 `compactionRequestKind` / `compactionResponseKind`，用于区分旧 `Compact` endpoint 与 `/v1/responses` 内的 remote compaction V2 语义。
- invocation payload 现在额外携带 `imageIntent`，并通过 `/api/invocations`、SSE `records`、Prompt Cache / Dashboard preview 一路透出，公开合同为 `yes | direct_image | no | unknown | null`。
- Records 与 Dashboard 列表共用图片信号 resolver：仅 `yes` / `direct_image` 渲染独立“图片工具”徽标；详情区保留四态文本区分，历史缺字段降级为 `—`。
- invocation payload 现已对外打通 `requestModel` / `responseModel`；Records、InvocationTable、Dashboard working conversations 与详情抽屉统一采用“响应模型优先”显示，并在规范化后的请求/响应模型真正不一致时显示上游路由差异图标。
- 共享 invocation preview 现已继续透出真实 `promptCacheKey`；Dashboard 上游账号活动 recent 行据此恢复稳定的对话短 ID 生成与详情抽屉 selection 关联，不再误用 `invokeId` 充当对话键。
- 调用详情现在固定展示“请求模型 / 响应模型”两个 badge；旧记录仅有历史 `model` 时，响应模型回填旧值，请求模型显示 `—`。
- 本次修复是 future-only：不改 SQLite schema，不对历史 invocation 回填 `imageIntent` 或 `compactionRequestKind`。
- 运行态 V2 识别来自 request body 的 `context_management[type=compaction][compact_threshold]`，终态识别来自响应内实际出现的 compaction item；两者独立写入 payload，不回填历史记录。

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-02-25
- Last: 2026-02-25

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test`
- `cargo check`
- `cargo check --tests`
- `cargo test prepare_target_request_body_detects_remote_v2_compaction_requests`
- `cargo test parse_target_response_payload_detects_remote_v2_compaction_stream_events`
- `cargo test parse_target_response_payload_detects_response_compaction_json_shape`
- `cargo test proxy_openai_v1_responses_pool_`
- `cargo test proxy_openai_v1_direct_image_pool_persists_direct_image_intent`
- `cd web && bun run test -- src/lib/api.test.ts src/hooks/useSettings.test.tsx src/pages/Settings.test.tsx src/hooks/useAvailableModelOptions.test.ts`
- `cd web && bun run test InvocationTable.test.tsx DashboardWorkingConversationsSection.test.tsx`
- `cd web && bun run test -- --run InvocationTable.test.tsx InvocationRecordsTable.test.tsx DashboardWorkingConversationsSection.test.tsx DashboardInvocationDetailDrawer.test.tsx promptCacheLive.test.ts invocationLiveMerge.test.ts`
- `cd web && bun run build`
- `cd web && bun run build-storybook`

## Migrated Implementation Sections

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: specs-first 建档与索引更新。
- [x] M2: 后端采集与接口输出增强（IP/prompt cache key/payload 投影）。
- [x] M3: 前端表格与文案升级（主表 + 详情）。
- [x] M4: 历史记录全量回填与幂等校验。
- [x] M5: 回归验证通过并完成本地提交。
- [x] M6: 调用详情移除 `source` 展示与代理名 fallback；号池尝试明细展示从 `proxyBindingKeySnapshot` 解析出的代理显示名，解析失败时使用紧凑 key fallback。
- [x] M7: Settings 页面增加 request/response body logging 双开关，后端 settings 合同、SQLite 单例持久化与 raw capture 链路同步接入。
- [x] M8: 关闭 response body logging 时同步关闭 `raw_response` preview，并让详情/回填链路接受“新记录无 raw body”为正常退化。
- [x] M9: 为 invocation 记录新增 `compactionRequestKind` / `compactionResponseKind` 语义投影，列表与详情按 `Compact` / `远程压缩V2` 的双层合同收口。
- [x] M10: 修复 pool `/v1/responses` 路径里 `compactionRequestKind=remote_v2` 的请求侧落库缺失，并保证在 `requestBodyLoggingEnabled=false` 下仍可观测。
- [x] M11: 将 `imageIntent` 打通到 payload / `/api/invocations` / SSE / Prompt Cache preview / Records / Dashboard，并为 owner-facing 列表补齐独立“图片工具”徽标。
- [x] M12: 将 `requestModel` / `responseModel` 打通到 `/api/invocations`、SSE、Prompt Cache preview 与 Dashboard working conversations，统一主模型显示优先级为 `responseModel ?? model ?? requestModel`。
- [x] M13: 为 Records、InvocationTable、Dashboard working conversations 与详情抽屉补齐 routed-model 差异图标与双模型详情展示，并保留 legacy `model` 记录的降级显示。
