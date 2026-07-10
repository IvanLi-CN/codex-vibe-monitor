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
- 新 HTTP proxy invocation 的 `invokeId` 由 `nanoid` 使用固定 10 位大写可读 alphabet 生成，格式为 `^[ABCDEFGHJKMNPQRSTUVWXYZ23456789]{10}$`；历史 `proxy-...` ID 不迁移，旧 reservation recovery 解析只对历史格式生效。
- 调用详情现在固定展示“请求模型 / 响应模型”两个 badge；旧记录仅有历史 `model` 时，响应模型回填旧值，请求模型显示 `—`。
- `InvocationExpandedDetails` 现已按快速排障路径重组为请求身份、路由与模型、失败信号、细节保留、阶段耗时分组；Live 展开区与 Dashboard 调用详情抽屉继续复用同一共享组件，长 ID、endpoint、IPv6 与错误文本在桌面和窄屏内换行或截断。
- 共享 invocation display view-model 现已区分 `accountRoutingInProgress`：当号池调用处于 `running` / `pending` 且已有上游账号名或账号 ID 时，Live、Records、Dashboard working conversations 与 Dashboard 调用详情抽屉显示当前账号，并使用 text-only primary 蓝色呼吸动画；缺账号仍显示“号池路由中”，终态账号不启用呼吸。
- 本次修复是 future-only：不改 SQLite schema，不对历史 invocation 回填 `imageIntent` 或 `compactionRequestKind`。
- 运行态 V2 识别来自 request body 的 `context_management[type=compaction][compact_threshold]`，终态识别来自响应内实际出现的 compaction item；两者独立写入 payload，不回填历史记录。
- raw request/response payload 的完整保留合同不作为 SQLite 止血牺牲项；本轮只补充 raw file write 的 `raw_kind`、codec、file bytes、observed bytes、truncated、path 与 elapsed 证据，并继续同步持久化 terminal usage/status/failure/raw metadata。
- 账号详情调用记录现显示可选择的 `invokeId`；健康与事件的调用 ID 通过 `/api/invocations/locate` 获取目标所在 retained/runtime 分页窗口及短生命周期 `anchorId`，后续页复用冻结 runtime overlay，再由共享虚拟表格定位目标行。
- 账号详情调用列表使用表面专属列宽完整单行展示 `invokeId`；桌面与移动定位态均采用单层非布局型高亮并抑制默认 outline，其他共享 `InvocationTable` 使用方保持原布局。

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
- `cargo test payload_utils_tests`
- `cargo test capture_target_pool_route_persists_attempt_rows_and_summary_fields`
- `cargo test capture_target_pool_route_no_content_success_finalizes_pending_attempt`
- `cargo test capture_target_pool_route_stops_after_three_distinct_accounts`
- `cargo test pool_route_compact_502_returns_cvm_id_and_attempt_observations`
- `cargo test send_pool_request_with_failover_keeps_early_phase_guard_armed_when_streaming_phase_was_not_persisted`
- `cargo test prepare_target_request_body_detects_remote_v2_compaction_requests`
- `cargo test parse_target_response_payload_detects_remote_v2_compaction_stream_events`
- `cargo test parse_target_response_payload_detects_response_compaction_json_shape`
- `cargo test proxy_openai_v1_responses_pool_`
- `cargo test proxy_openai_v1_direct_image_pool_persists_direct_image_intent`
- `cd web && bun run test -- src/lib/api.test.ts src/hooks/useSettings.test.tsx src/pages/Settings.test.tsx src/hooks/useAvailableModelOptions.test.ts`
- `cd web && bun run test InvocationTable.test.tsx DashboardWorkingConversationsSection.test.tsx`
- `cd web && bun run test -- --run InvocationTable.test.tsx InvocationRecordsTable.test.tsx DashboardWorkingConversationsSection.test.tsx DashboardInvocationDetailDrawer.test.tsx promptCacheLive.test.ts invocationLiveMerge.test.ts`
- `cd web && bun run test -- InvocationTable.test.tsx InvocationRecordsTable.test.tsx DashboardInvocationDetailDrawer.test.tsx`
- `cd web && bun run test -- InvocationTable.test.tsx InvocationRecordsTable.test.tsx DashboardWorkingConversationsSection.test.tsx`
- `cd web && bun run test`
- `cd web && bun run test-storybook`
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
- [x] M14: 重组共享调用详情组件的信息架构与视觉层级，补齐成功、运行中、异常、号池终态、长字段、light/dark 与窄屏 Storybook 证据。
- [x] M15: 保留完整 raw payload 合同，为 raw 文件写入与 terminal raw metadata 写入补齐低开销耗时证据。
- [x] M16: 为运行态号池调用补齐当前上游账号呼吸提示，并覆盖 Live、Records、Dashboard working conversations 与 Dashboard 调用详情抽屉的共享账号展示路径。
- [x] M17: 为账号详情补齐调用 ID 展示、账号作用域锚点分页、虚拟滚动定位与结构化未找到反馈。
