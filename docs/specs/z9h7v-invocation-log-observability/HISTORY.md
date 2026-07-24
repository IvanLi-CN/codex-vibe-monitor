# 请求日志可观测性增强（IP / Cache Tokens / 分阶段耗时 / Prompt Cache Key / Body Logging Toggles） - History

## Account upstream attempt observability

- 账号详情从最终调用记录切换为 7 天主库尝试请求表，修复失败账号事件链接到最终成功账号调用而无法定位的问题；每行只显示本次尝试请求的请求/响应模型、结果、代理、三段延迟和错误，不显示 endpoint，也不混入重试序号或最终调用 usage。
- 账号尝试请求列表将 HTTP 明确为上游结果，代理优先显示可读节点名，并将完整错误、上游请求 ID、路由键及不一致的下游 HTTP 收入对应摘要卡的详情面板；桌面与移动端统一为同一套卡片交互。
- 账号尝试请求列表的旧“不得显示最终 invocation tokens/费用”限制已被 workflow parity 合同取代：账号 attempts API 返回 `workflowEntry` / `invocationRecord`，前端复用调用详情 attempt 卡；Token/成本只显示在最终成功 attempt，不复制到失败重试。
- 诊断证据改为当前尝试摘要卡下方的详情面板，避免将关键排障信息压缩在局部入口中。
- 路由调用事件新增可空 `attempt_id` 精确关联；旧事件保持可见但不可跳转。
- 尝试列表的模型投影保持从 invocation payload 读取，兼容未包含独立 request/response model 列的既有 SQLite 数据库；健康事件以 `attempt_id` 作为可见定位标识，不再显示空的最终请求 ID。
- owner-facing attempt 标识收口为持久化短 `attemptId`：live / archive `pool_upstream_request_attempts` 统一新增 `attempt_public_id`，新写入即生成，启动期顺序回填历史主库与 archive batch；账号详情、健康事件与 Records 新入口不再暴露纯数字 attempt 主键。

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
- 2026-06-28: 将共享 invocation preview 的 `promptCacheKey` 明确打通到 Dashboard 上游账号活动 recent 行，修复详情抽屉 selection 误把 `invokeId` 当对话键的问题。
- 2026-06-30: 重组共享调用详情组件的信息架构，按快速排障路径分组展示请求身份、路由与模型、失败信号、细节保留和阶段耗时，并补齐 Storybook 视觉证据。
- 2026-07-02: 明确 101 SQLite 止血不截断、不跳过、不丢弃 raw payload；新增 raw 文件写入耗时、codec、文件字节数与 terminal raw metadata 写入路径证据，用于区分 DB 核心写慢、batch flush 慢与 raw IO/gzip 慢。
- 2026-07-03: 新 HTTP proxy invocation 改用 10 位 NanoID 风格 `invokeId`，移除 owner-facing `proxy-...` 前缀、内部 counter 与时间戳；历史长 ID 继续兼容查询、展示与 reservation recovery。
- 2026-07-07: 运行态号池调用在已有上游账号时显示当前账号，并以 text-only 蓝色呼吸状态表达“正在路由中”；缺账号仍使用“号池路由中”fallback，终态账号保持普通显示。
- 2026-07-10: 账号详情请求 ID 升级为可定位入口；新增账号作用域锚点分页接口，由后端直接返回目标所在窗口，前端不再为定位遍历或预加载历史记录。
- 2026-07-10: 锚点分页增加短生命周期 `anchorId`，使后续相邻页复用定位时冻结的 runtime overlay，避免运行态记录令页边界漂移。
- 2026-07-10: 账号详情请求 ID 固化为单行完整展示，并通过表面专属列宽与单层非布局高亮避免截断、换行和焦点轮廓叠加。
- 2026-07-19: 上游账号详情 owner-facing 术语统一改为“请求 / 尝试请求 / 请求 ID”；页签显示“请求”，尝试列表继续以 `attemptId` 为主入口；调用侧次级标识改为调用短 ID，不再裸露原始 `invokeId`。
- 2026-07-13: Dashboard 活动快照新增成功已计费调用的响应模型/思考程度性能聚合，并在总览和账号卡提供桌面浮层与窄屏抽屉入口；后续实时 KPI 合同已迁回 `z6ysw` 的最近完整 1 分钟 bucket，本 spec 只保留完整范围模型性能明细语义。
- 2026-07-15: 将 Dashboard 模型性能时长合同从含混的单字段 `usageDurationMs` 改为显式的 `wallClockUsageDurationMs`、`cumulativeUsageDurationMs` 与 `parallelism`，并统一覆盖全局、账号、模型与账号+模型四级聚合。
- 2026-07-15: Attempt owner-facing 合同改为持久化 8 位短 `attemptId`；账号详情、健康与事件、Records 新跳转统一改用 `attemptId`，并新增启动期 live/archive backfill 补齐历史 `attempt_public_id`。
- 2026-07-16: 修复 terminal failure payload summary 在 pool route / pre-upstream 失败分支丢失 `requestModel` 的合同漂移，确保 `/api/invocations`、SSE records 与账号尝试列表在失败记录上继续拿到真实请求模型。
- 2026-07-18: 新增 direct/pool 上游请求压缩字节事实与 HTTP 近似真值聚合字段，统一在调用详情和账号尝试诊断中展示 `压缩比 + 前/后字节`、`近似上传` 与 `近似下载`，并补充对应 Storybook 视觉证据。
- 2026-07-20: Records 首屏模型摘要收口为响应模型主值 + reroute / reasoningEffort / imageIntent 信号；`聚焦摘要` 统一更名为 `Token 与成本`，并在列表成本列、展开摘要与详情卡之间复用同一套 mismatch warning 语义。
- 2026-07-20: `/api/invocations` 记录对象新增 advisory `costAudit`，以持久化 `cost` 为真值、按当前 catalog 本地重算为对照，采用 `0.000001 USD` 容差并区分 `price_version_changed`、`total_mismatch` 与不可比较原因。
- 2026-07-20: workflow detail 只为最终成功 attempt 注入 `responseSummary.usage` Token/成本审计对象，补齐未命中缓存输入、命中缓存输入、输出与金额四项指标，并修正 `reasoningTokens` 缺失不得伪造成 `0`。
- 2026-07-21: 修复健康与事件 `attemptId` 定位回归：账号详情只保留“请求 tab -> target attempt”路径，删除 records tab 中隐藏的 `InvocationTable` / anchored locate dead path，并将 owner-facing 焦点反馈收口为滚动入视区、展开诊断、下一次抽屉内交互后 1.5 秒延迟消退的高亮合同。
- 2026-07-21: 修复 Records 响应体面板打开后把整条详情布局顶宽的问题；展开详情、workflow detail 与结构化 payload viewer 统一限制外层宽度，并将 SSE/NDJSON 超宽内容的横向滚动下沉到单个事件/行卡片内部，补充对应 Storybook 回归证据。
- 2026-07-22: 账号详情 attempts API 改为返回 workflow-compatible `workflowEntry` 与 `invocationRecord`，前端直接复用调用详情 attempt 卡；旧账号详情不显示 Token/成本的限制被“仅最终成功 attempt 显示 usage”规则 supersede。

- 2026-07-24: Added Responses Lite image-tool rewrite audit fields to invocation payload and workflow attempt request detail, making skipped client-owned tools visible next to the effective policy.
