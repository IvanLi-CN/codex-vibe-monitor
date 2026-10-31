# Forward Proxy 节点延迟与订阅刷新（#kmr5z）

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## Context and Scope

- Forward proxy 设置页已有节点历史成功率、历史均值延迟和订阅配置，但缺少用户主动确认“当前节点是否可用、当前延迟是多少”的操作入口。
- 订阅节点可能在后台周期刷新前发生变化，用户需要在设置页手动强制刷新订阅并立刻看到新增节点。
- 单节点测速与批量测速需要渐进反馈，避免用户等待全部节点、全部轮次结束后才知道可用节点。

### Goals and Non-goals

### Goals

- 在 forward proxy 节点表新增可点击的延迟列，显示当前手动测速状态与均值延迟。
- 支持单节点重测：最多 5 轮，每轮请求预算 5 秒，单节点总预算 15 秒。
- 支持批量测试：按广度优先调度节点，所有节点先跑第 1 轮，再进入第 2 轮，最多 5 轮。
- 任一节点只要出现一轮有效耗时就立即显示当前均值，后续有效样本继续刷新均值。
- 支持手动刷新订阅，并返回刷新后的节点设置与新增节点数量。

### Non-goals

- 不改变 forward proxy 路由选择、权重计算或惩罚策略。
- 不做后台常驻巡检；测速只由用户点击触发。
- 不把失败或超时样本折算成固定大延迟。

### Scope

### In scope

- Rust API：新增订阅刷新、单节点测速、批量测速进度接口。
- Rust runtime：复用现有 proxy client、egress IP provider、OAuth upstream base、attempt/metadata 记录能力。
- Web UI：Settings 页新增延迟列、单节点按钮、批量测试入口、订阅刷新按钮与 i18n。
- Storybook：Settings 页覆盖刷新、测速中、成功、部分失败、全失败状态。

### Out of scope

- 代理节点导入验证流程的轮次数与并发策略。
- 账号池绑定节点选择器的展示逻辑。
- 自定义测速站点配置。

## Requirements

### REQ-KMR5Z-001 Latency Feedback

- `REQ-KMR5Z-LATENCY-TEST`: 节点测速 MUST 保持既有预算、目标可达性和渐进结果契约。
- `REQ-KMR5Z-SUBSCRIPTION-RECOVERY`: 订阅节点快照 MUST 在匹配源配置时于启动恢复，并在完整刷新成功前保留上次成功结果。
- `REQ-KMR5Z-REFRESH-CONTRACT`: 订阅刷新 MUST 保持既有 HTTP 响应字段契约；部分失败只能保留旧结果并标记内部任务为 `skipped`，完整空结果 MUST 清空节点。

- 节点表必须新增“延迟”列，内容为可点击 badge/button。
- 未测状态显示 `测试`；测试中显示当前轮次进度；有结果显示整数毫秒；五轮全失败显示 `--` 或“超时”。
- 单节点测速最多执行 5 轮，每轮预算 5 秒，整体预算 15 秒。
- 批量测试必须广度优先，不能让单个节点连续跑完 5 轮后才测试下一个节点。
- 均值只使用成功样本，算法为 `round(sum(successLatencyMs) / successCount)`。

### REQ-KMR5Z-002 Probe Health

- 每轮测试包含出站 IP、OAuth `/models` 上游和 Codex `/responses` 上游三个目标；三个目标全部可达才算该轮健康。
- 出站 IP 请求使用 `https://api.ipify.org?format=json`。
- OAuth 上游请求使用 `GET {oauth_codex_upstream_base_url()}/models`。
- Codex responses 上游请求使用 `GET {oauth_codex_upstream_base_url()}/responses`；`405`、`401`、`403`、`404` 和其他 `<500` HTTP 状态均视为 endpoint reachable，transport error、超时和 `>=500` 视为不可达。
- 任一目标不可达时，节点进度必须报告异常目标与错误原因；成功延迟样本只能用于展示统计，不能把节点最终状态改成正常。

### REQ-KMR5Z-003 Subscription Refresh

- 手动刷新订阅必须强制绕过订阅刷新间隔。
- 启动时必须优先恢复与当前订阅源配置匹配的上次成功节点快照，并在接收请求前同步这些节点的代理路由。
- 订阅刷新只有在所有已配置来源都成功获取后才可替换节点快照；全部或部分来源失败时，保留上次成功节点。
- 所有来源成功但解析结果为空时，必须立即清空活动节点和持久化快照。
- 修改或删除订阅源时，必须清除旧配置对应的快照，不能在之后重新添加时复用旧节点。

### REQ-KMR5Z-004 Restart Recovery

- 启动时必须优先恢复与当前订阅源配置匹配的上次成功节点快照，并在接收请求前同步这些节点的代理路由。
- 启动恢复快照时，若其订阅源列表与当前设置不一致，则忽略快照并继续使用当前手动代理配置。

### REQ-KMR5Z-005 Refresh Result Semantics

- 订阅刷新只有在所有已配置来源都成功获取后才可替换节点快照；全部或部分来源失败时，保留上次成功节点。
- 所有来源成功但解析结果为空时，必须立即清空活动节点和持久化快照。

### REQ-KMR5Z-006 Subscription Source Invalidation

- 修改或删除订阅源时，必须清除旧配置对应的快照，不能在之后重新添加时复用旧节点。

### REQ-KMR5Z-007 Interrupted Settings Rollback

- 设置更新后的路由同步被停机中断且数据库回滚成功时，必须在同一事务中恢复旧设置与匹配的旧快照；若回滚事务失败，manager 必须保持与数据库中最后提交的设置一致并返回内部错误。

### REQ-KMR5Z-008 Subscription Settings Presentation

- 订阅刷新成功后，必须返回刷新后的节点设置与新增节点数量；Settings 页应使用该设置更新节点列表。

- 延迟 badge 的 tooltip 应显示有效样本数、已完成轮次、出站 IP 状态、OAuth `/models` 状态、Codex `/responses` 状态和失败摘要。
- 测速成功时应刷新出站 IP 元数据；OAuth 上游探测应写入 probe attempt。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 用户点击某节点延迟 badge 后，UI 建立测速进度流并只更新该节点。
- 用户点击“测试全部”后，UI 对当前表格内全部节点建立批量测速进度流。
- 后端每完成一个节点的一轮测试，就推送该节点当前聚合结果。
- 订阅刷新按钮调用强制刷新 API，成功后用响应中的设置替换当前 forward proxy 设置草稿。

### Edge cases / errors

- 节点不存在时返回 404。
- 单轮任一目标失败时，该轮不写入成功 probe attempt，并在进度中保留失败目标；已成功目标的耗时仍可作为展示样本。
- 五轮后任一目标存在失败时，节点进入失败/异常展示状态，即使 `averageLatencyMs` 存在也不能显示为正常延迟。
- 单节点整体 15 秒预算耗尽后，返回已完成样本；无有效样本时显示超时。
- 批量测试中单个节点失败不影响其他节点继续测试。
- 启动恢复快照时，若其订阅源列表与当前设置不一致，则忽略快照并继续使用当前手动代理配置。
- 启动恢复快照时，若 JSON 无法解析、订阅源不匹配，或快照包含非法/非规范化的订阅节点 URL，则整体忽略快照，不恢复其中的部分节点。
- 订阅刷新出现任一来源错误时，本轮结果不完整，不得清除或部分替换上次成功的节点快照。
- 手动刷新遇到部分来源失败时，继续沿用既有 HTTP 状态和响应字段契约；只在内部任务观测中标记为 `skipped`，并保留上次成功结果。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                                                  | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）                                   |
| ------------------------------------------------------------- | ------------ | ------------- | -------------- | ------------------------ | --------------- | ------------------- | ----------------------------------------------- |
| `POST /api/settings/forward-proxy/refresh-subscriptions`      | HTTP JSON    | internal      | New            | None                     | backend         | Settings page       | 强制刷新订阅                                    |
| `GET /api/settings/forward-proxy/nodes/:proxyKey/test-stream` | SSE          | internal      | New            | None                     | backend         | Settings page       | 单节点测速进度                                  |
| `GET /api/settings/forward-proxy/nodes/test-stream`           | SSE          | internal      | New            | None                     | backend         | Settings page       | 批量测速进度，使用重复 `key` query 参数传入节点 |

### 契约文档（按 Kind 拆分）

- None

## Verification

- Given Settings 页有多个 forward proxy 节点，When 用户点击“测试全部”，Then 每个节点先完成第 1 轮后才进入第 2 轮。
- Given 某节点第 1 轮有有效耗时，When 该轮结束，Then UI 立即显示当前均值。
- Given 某节点后续轮次继续成功，When 新样本到达，Then UI 显示更新后的整数毫秒均值。
- Given 某节点 5 轮全部失败，When 测试结束，Then UI 显示 `--` 或“超时”。
- Given 某节点 `egressIp` 与 OAuth `/models` 成功但 Codex `/responses` 超时，When 测试进度到达前端，Then UI 显示异常并展示 `codexResponses` 失败原因，而不是显示正常延迟。
- Given Codex `/responses` 返回 `405`，When 后端记录该目标结果，Then 该目标视为可达并可贡献成功样本。
- Given 用户点击刷新订阅，When 后端成功解析订阅，Then Settings 页节点列表包含刷新后的节点。
- Given 上次成功刷新已保存节点且订阅源配置未变，When 服务重启但订阅源暂时不可用，Then 服务开始处理请求前恢复并路由上次成功的节点。
- Given 多个订阅源中任一来源刷新失败，When 本轮刷新结束，Then 活动节点与持久化快照均保留本轮前的上次成功结果。
- Given 手动刷新中部分订阅源失败，When 请求完成，Then 响应仍使用既有状态和字段，内部刷新任务标记为 `skipped`，且不报告替换出的新节点。
- Given 启动快照 JSON 可解析但包含非法或非规范化节点 URL，When 服务尝试恢复，Then 整个快照被忽略，不恢复剩余合法节点。
- Given 所有订阅源均成功返回但解析结果为空，When 本轮刷新结束，Then 活动节点和持久化快照立即变为空。
- Given 用户删除或修改订阅源，When 设置保存成功，Then 旧订阅源的节点快照被清除，不会因重新添加相同地址而恢复。

### VER-KMR5Z-BACKEND

- Method: backend unit and integration regressions for latency probing, OAuth proxy precedence, subscription restart recovery, refresh retention, source invalidation, and schema compatibility.
- covers: `REQ-KMR5Z-LATENCY-TEST`, `REQ-KMR5Z-SUBSCRIPTION-RECOVERY`, `REQ-KMR5Z-REFRESH-CONTRACT`

### VER-KMR5Z-FRONTEND

- Method: Settings page unit, Storybook, and build checks for latency and subscription refresh states.
- covers: `REQ-KMR5Z-LATENCY-TEST`, `REQ-KMR5Z-REFRESH-CONTRACT`

### VER-KMR5Z-002 Durable Subscription Recovery

- Method: SQLite-backed restart, refresh, source invalidation, and interrupted-settings rollback regression tests.
- covers: REQ-KMR5Z-004, REQ-KMR5Z-005, REQ-KMR5Z-006, REQ-KMR5Z-007
- Pass condition: startup restores only a source-matched snapshot before requests; incomplete refresh retains the last success; successful empty refresh clears it; source changes invalidate it; shutdown rollback restores old settings and snapshot atomically or leaves runtime and database aligned with the last committed settings.

### VER-KMR5Z-003 Repository Quality

- Method: Rust formatting and focused Rust regression tests; web unit tests and build for Settings page changes.
- covers: REQ-KMR5Z-001, REQ-KMR5Z-002, REQ-KMR5Z-003, REQ-KMR5Z-004, REQ-KMR5Z-005, REQ-KMR5Z-006, REQ-KMR5Z-007, REQ-KMR5Z-008
- Pass condition: applicable formatting, backend, web unit, and web build checks pass.

## Related ADRs

None

## Visual Evidence

- source_type: storybook_canvas
  story_id_or_title: Settings/SettingsPage/Forward Proxy Latency And Refresh
  target_program: mock-only
  capture_scope: browser-viewport
  requested_viewport: desktop1660
  viewport_strategy: storybook-viewport
  sensitive_exclusion: N/A
  submission_gate: owner-approved
  evidence_note: 验证 Settings 页 forward proxy 卡片新增延迟列、测试全部、刷新订阅反馈与新增订阅节点展示。

![Forward proxy latency and refresh evidence](./assets/settings-forward-proxy-latency-refresh.png)

- source_type: storybook_canvas
  story_id_or_title: Settings/SettingsPage/Forward Proxy Latency And Refresh
  target_program: mock-only
  capture_scope: browser-viewport
  requested_viewport: 1440x900
  viewport_strategy: devtools-emulate
  sensitive_exclusion: N/A
  submission_gate: owner-approved
  evidence_note: 验证 `codexResponses` 失败时延迟按钮显示异常，并在 hover/focus 浮层中展示 `Codex /responses` 失败详情。

![Forward proxy latency tooltip evidence](./assets/settings-forward-proxy-latency-tooltip-evidence.png)

- source_type: storybook_canvas
  story_id_or_title: Monitoring/ForwardProxyLiveTable/Populated
  target_program: mock-only
  capture_scope: element
  requested_viewport: 1660x960
  viewport_strategy: devtools-emulate
  sensitive_exclusion: N/A
  submission_gate: pending-owner-approval
  evidence_note: 验证 Live 页代理运行态的五个窗口统计列已按 `7 天 / 1 天 / 1 小时 / 15 分钟 / 1 分钟` 展示。

![Live forward proxy window order](./assets/forward-proxy-live-window-order.png)

- source_type: storybook_canvas
  story_id_or_title: Settings/SettingsPage/Forward Proxy Latency And Refresh
  target_program: mock-only
  capture_scope: element
  requested_viewport: 1660x960
  viewport_strategy: devtools-emulate
  sensitive_exclusion: N/A
  submission_gate: pending-owner-approval
  evidence_note: 验证 Settings 页 forward proxy 桌面表格的五个窗口统计列已按 `7 天 / 1 天 / 1 小时 / 15 分钟 / 1 分钟` 展示。

![Settings forward proxy desktop window order](./assets/settings-forward-proxy-window-order.png)

- source_type: storybook_canvas
  story_id_or_title: Settings/SettingsPage/Forward Proxy Latency And Refresh
  target_program: mock-only
  capture_scope: element
  requested_viewport: 430x932
  viewport_strategy: devtools-emulate
  sensitive_exclusion: N/A
  submission_gate: pending-owner-approval
  evidence_note: 验证 Settings 页 forward proxy 移动卡片的窗口小格已按 `7 天 / 1 天 / 1 小时 / 15 分钟 / 1 分钟` 展示。

![Settings forward proxy mobile window order](./assets/settings-forward-proxy-window-order-mobile.png)

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：批量测速节点很多时会维持较长 SSE 连接；实现必须在每轮预算内释放已超时请求。
- 假设：Direct 节点参与测试，作为当前机器直连基线。
- 假设：OAuth 上游无凭据响应仍可用于网络可达性与延迟诊断，实际 HTTP 状态会返回给 UI。

## 参考（References）

- `src/forward_proxy/`
- `web/src/pages/Settings.tsx`
