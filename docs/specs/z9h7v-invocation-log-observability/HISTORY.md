# 请求日志可观测性增强（IP / Cache Tokens / 分阶段耗时 / Prompt Cache Key / Body Logging Toggles） - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/z9h7v-invocation-log-observability/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-02-25: 初始化规格，冻结实现边界与验收口径。
- 2026-02-25: 完成后端字段采集、`/api/invocations` 投影扩展与前端表格升级，并通过 `cargo test`、`cargo check`、`web npm run build` 验证。
- 2026-02-25: 修复 SSE `records` 广播回查 SQL 投影不全问题，确保 `endpoint/requesterIp/promptCacheKey/failureKind` 与 `/api/invocations` 一致，并补充回归测试。
- 2026-02-25: 将对外字段从 `codexSessionId` 切换为 `promptCacheKey`，新增启动期历史数据全量回填与旧键清理，并补充回填幂等/异常分支测试。
- 2026-02-25: 修复启动回填对历史相对路径 raw 文件的兼容性（新增 `database_path` 父目录兜底），避免因工作目录变化导致 `skipped_missing_file` 异常偏高。
- 2026-05-11: 修正代理诊断展示口径：调用详情隐藏 `source` 且不再作为代理名 fallback，号池尝试明细展示实际落库的 `proxyBindingKeySnapshot`。
- 2026-05-11: 将号池 `budget_exhausted_final` 合成终态从真实重试明细中拆出，明确展示未发起新的上游请求，避免误读为同账号 429 后再次重试。
- 2026-05-12: 修正号池尝试代理字段可读性：前端使用 `proxyBindingKeySnapshot` 查询绑定节点并展示代理显示名，未解析时才显示紧凑 key。
- 2026-06-22: 扩展 proxy settings 合同与 SQLite 单例，新增 `requestBodyLoggingEnabled` / `responseBodyLoggingEnabled` 双开关，默认值均为 `true`。
- 2026-06-22: raw capture 链路按新开关裁剪 request raw、response raw 与 `raw_response` preview；关闭后仅停止新的 body 留存，保留结构化 payload、usage、timing 与路由元数据。
- 2026-06-22: Settings 页面新增请求/响应 body 记录开关，并补充前后端回归测试与 Storybook 视觉证据。
- 2026-06-23: 新增 `compactionRequestKind` / `compactionResponseKind` 语义层，稳定识别 `/v1/responses` 内的 remote compaction V2，并保持旧 `/v1/responses/compact` 继续显示为 `Compact`。
- 2026-06-23: 调整 invocation 列表与详情展示规则：列表按运行态请求信号与终态响应信号区分 `Responses` / `远程压缩V2`，详情始终保留原始 endpoint 并单列展示“压缩请求 / 压缩响应”。
- 2026-06-24: 修复 pool `/v1/responses` 请求侧 `compactionRequestKind` 在 prebuffer/replay 路径丢失的问题，确保 `requestBodyLoggingEnabled=false` 时仍可稳定落库 `remote_v2`。
- 2026-06-24: 将 `imageIntent` 升级为公开 invocation 可观测合同，打通 `/api/invocations`、SSE `records`、Prompt Cache preview、Records 与 Dashboard，并新增独立“图片工具”徽标。
- 2026-06-26: 将 `requestModel` / `responseModel` 扩展到 `/api/invocations`、SSE `records`、Prompt Cache preview 与 Dashboard working conversations，并统一主模型显示为 `responseModel ?? model ?? requestModel`。
- 2026-06-26: 调用详情拆分为“请求模型 / 响应模型”双字段；当规范化后的请求/响应模型不一致时，仅响应模型 badge 显示上游路由差异图标，旧 `model` 记录继续作为响应模型回填。
