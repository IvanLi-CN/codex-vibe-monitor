# 请求记录筛选范围控件与诊断维度增强（#f2w7m）

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## 背景 / 问题陈述

`/records` 已经具备稳定快照、分页、聚焦统计与基础排障筛选，但当前筛选抽屉存在两个长期问题：

- 区间条件被拆成多个孤立表单项，尤其是时间范围、总 Tokens、总耗时，导致高频排障时认知切换过多。
- 一部分真实诊断能力已经存在于后端契约或记录详情中，但前端没有形成可直接使用的筛选入口，例如短 `调用 ID / 尝试 ID`、上游账号、上游范围，以及代理/传输/服务层级/推理强度等运行态维度。

这使得 records 页面停留在“参数可配”而不是“筛选高效”的状态，也让列表、详情与抽屉之间的诊断词汇表长期不一致。

## 目标 / 非目标

### Goals

- 保留当前筛选抽屉承载方式，但重构其信息架构，使高频排障字段按范围、请求上下文、路由与上游、结果四组组织。
- 将时间范围、总 Tokens、总耗时统一收口为可复用的单字段范围控件，而不是继续把每个端点暴露为独立表单项。
- 正式接出 records 已有但未完整 owner-facing 化的短 ID 与路由筛选能力：`invokeId`、`attemptId`、`upstreamAccountId`、`upstreamScope`。
- 扩展 records 查询与 suggestions 契约，支持 `proxyDisplayName`、`transport`、`serviceTier`、`reasoningEffort` 等诊断维度。
- 保持 records 页既有 stable snapshot、draft/applied 双态、分页/排序与 new-count 语义不变。

### Non-goals

- 不把筛选从抽屉改成页面常驻表单。
- 不新增 `upstreamRequestId`、原始 `routeMode` 或其它超出本轮约束的新筛选维度。
- 不改变 records 列表、summary cards、详情抽屉与 locate/anchor 路由的现有读模型语义。
- 不引入新的第三方 date picker 或完整表单框架。

## 范围（Scope）

### In scope

- `web/src/pages/Records.tsx` 的筛选 IA、草稿状态、应用 chips 与 inline 校验。
- 共享 UI 复用件 `DateTimeRangeField`、`NumericRangeField` 与 suggestion 选择器能力增强。
- records 前后端查询参数、suggestions buckets、类型定义与 mock/story fixtures 更新。
- 与新筛选抽屉直接相关的 Storybook、Vitest、Rust tests 和 mock-only visual evidence。

### Out of scope

- 记录详情抽屉的新字段展示扩展。
- `Dashboard`、`Live`、账号详情 records tab 或其他页面的筛选 IA 重做。
- 导出筛选方案、持久化筛选模板、跨页面共享 saved filters。

## 需求（Requirements）

### MUST

- 时间范围在 UI 中只表现为一个字段；同一控件内必须同时承载 preset 与 custom range。
- `总 Tokens` 与 `总耗时（ms）` 在 UI 中也各自只表现为一个范围字段；内部虽可有两个端点输入，但外层只能算一个 form item。
- 若 `from >= to`、`min > max` 或输入无法归一化，抽屉必须给出 inline error，并禁用“应用筛选”。
- 范围控件的错误状态必须和实际可交互元素建立可访问性语义关联，至少覆盖 `aria-invalid` 与错误说明关联。
- records 抽屉必须直接支持：`invokeId`、`attemptId`、`model`、`endpoint`、`failureClass`、`failureKind`、`promptCacheKey`、`requesterIp`、`keyword`、`upstreamScope`、`upstreamAccount`、`proxyDisplayName`、`transport`、`serviceTier`、`reasoningEffort`、`time range`、`totalTokens range`、`totalMs range`。
- `invokeId` 与 `attemptId` 都必须按界面展示用的短 ID 做精确匹配，不能与 `keyword` 共用模糊语义。
- `upstreamAccount` 的最终查询键必须是 `upstreamAccountId`，但 owner-facing 文案必须显示“账号名 (#ID)”而不是只显示裸数字。
- `upstreamScope` 用 owner-facing 高层语义表达“内部号池 / 外部路径”，不再把旧的笼统“上游”作为主文案。

### SHOULD

- 静态维度使用统一 `SelectField` 语义；高基数字段继续使用 suggestion combobox，并支持 label/value 分离。
- 应用 chips 应使用人类可读标签，尤其是 `upstreamAccount`、`transport`、`serviceTier`、`reasoningEffort` 与范围摘要。
- 数值范围控件应直接显示当前选中区间摘要，而不是只显示滑块端点位置。
- 现有移动端抽屉与桌面抽屉宽度行为保持稳定，不因新字段引入浮层裁剪或滚动冲突。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 用户打开 `/records` 筛选抽屉后，先看到分组明确的字段块；修改任意字段仅更新草稿，点击“应用筛选”后才刷新 stable snapshot。
- 用户在时间范围字段内切换 `today / 1d / 7d / 30d / custom` 时，当前摘要、隐藏的 `from/to` 草稿值与应用 chips 一并更新。
- 用户操作 Tokens 或耗时 slider 时，字段内部需同步显示当前区间摘要，并允许通过轨道区域直接调整最近端点，而不要求必须精确抓住 thumb。
- 用户在 `upstreamAccount` 输入账号名或 ID 时，suggestions 返回可读 label，但应用后查询统一走 `upstreamAccountId`。
- 用户切换 `transport`、`upstreamScope` 等静态维度时，不触发 suggestions 请求；高基数字段则继续受当前 snapshot 与 draft filters 约束。

### Edge cases / errors

- 无效范围不得 silently coerce 成空值；必须明确反馈并阻止应用。
- 若某个 suggestion bucket 在当前 snapshot 下为空，字段仍需保持可聚焦、可输入和可清空。
- 旧记录缺失 `serviceTier`、`reasoningEffort`、`proxyDisplayName` 等字段时，筛选和 suggestions 必须按 `NULL` 兼容，不产生 `undefined` 文案或崩溃。

## 接口契约（Interfaces & Contracts）

### `InvocationRecordsQuery`

- 保留既有 `rangePreset`、`from`、`to`、`status`、`model`、`endpoint`、`failureClass`、`failureKind`、`promptCacheKey`、`requesterIp`、`keyword`、`minTotalTokens`、`maxTotalTokens`、`minTotalMs`、`maxTotalMs`。
- records canonical query owner-facing 使用的短 ID 与扩展维度为：`invokeId`、`attemptId`、`upstreamScope`、`upstreamAccountId`、`proxyDisplayName`、`transport`、`serviceTier`、`reasoningEffort`。

### `InvocationSuggestionField` / `InvocationSuggestionsResponse`

- 新增 buckets：`proxyDisplayName`、`upstreamAccount`、`serviceTier`、`reasoningEffort`。
- `InvocationSuggestionItem` 允许可选 `label`，用于“显示值 != 查询值”的 suggestions；本轮至少 `upstreamAccount` 需要该能力。

### Shared UI fields

- `DateTimeRangeField` 对外暴露 `preset`、`from`、`to`、`onChange`、`disabled`、`error` 与 owner-facing range summary。
- `NumericRangeField` 对外暴露 `min`、`max`、`step`、`unitLabel`、`onChange`、`disabled`、`error` 与 owner-facing range summary。
- `NumericRangeField` 在分组容器内使用时必须支持嵌入态，不额外渲染独立卡片式外壳，避免在筛选抽屉里形成嵌套卡片。
- 两个控件都必须适配现有 `field` / `field-label` / focus ring / dark-light theme 词汇表。

## 验收标准（Acceptance Criteria）

- Given 用户打开 records 抽屉，When 查看时间范围区域，Then 页面不再渲染独立的“开始时间 / 结束时间”表单项，而是一个可同时切 preset/custom 的单字段范围控件。
- Given 用户查看 Tokens 或耗时区间，When 操作任一范围条件，Then 各自只表现为一个字段，并在无效区间时显示 inline error 且禁用应用按钮。
- Given 用户需要按调用 ID、尝试 ID、上游账号、代理节点、传输、服务层级、推理强度排障，When 操作抽屉，Then records 列表、summary 与 new-count 共享同一过滤口径。
- Given 用户在 `upstreamAccount` 中输入账号名或数字，When suggestions 返回并应用，Then chip 展示账号名和 ID，底层查询只发送 `upstreamAccountId`。
- Given 用户使用短 `invokeId`、短 `attemptId` 与 `keyword`，When 分别搜索，Then 前两者保持精确匹配，`keyword` 仍是模糊全文匹配，三者语义不混淆。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests 覆盖新增过滤和 suggestions buckets。
- `src/pages/Records.test.tsx`、新范围组件 tests、相关 API/query tests 覆盖抽屉结构、无效范围阻断、label/value suggestions 与 chips 摘要。

### UI / Storybook (if applicable)

- 新增或更新范围控件 stories。
- Records 页 Storybook fallback / canvas 需要覆盖新抽屉结构与关键交互。
- 最终 owner-facing 视觉证据需来自 mock-only 稳定渲染面。

## Visual Evidence

- 最终 owner-facing 视觉证据以本 spec `./visual-evidence/` 中的 mock-only captures 为准。
- `./visual-evidence/records-range-diagnostics-drawer.png` - PR: include
- `./visual-evidence/records-model-filter-nested-selector.png` - PR: include
- `./visual-evidence/date-time-range-field.png` - PR: none
- `./visual-evidence/numeric-range-field.png` - PR: none
- `./assets/invocation-model-filter-field-evidence-request-configured.png` - PR: none
- `./assets/invocation-model-filter-field-evidence-response-rerouted.png` - PR: include
- `./assets/invocation-model-filter-field-evidence-response-not-rerouted.png` - PR: none

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- `upstreamScope` 采用 owner-facing“内部 / 外部”抽象，而不是同时暴露底层 `routeMode`：已锁定。
- `upstreamAccount` suggestions 需要兼顾 label 和精确 value，属于本轮共享 combobox 能力增强的一部分：已锁定。
- 这轮不做 saved filters，因此新 IA 需要在不引入额外持久化的前提下足够高效：已锁定。

## 参考（References）

- `docs/archive/specs/6whgx-records-stable-snapshot-analytics/SPEC.md`
- `docs/archive/specs/3gvtt-records-request-id-response-details/SPEC.md`
- `docs/archive/specs/8pjnh-records-filter-dropdown-overlap-fix/SPEC.md`
- `docs/specs/hnu7b-mobile-first-navigation-and-overlays/SPEC.md`
- `docs/specs/ykhfu-web-demo/SPEC.md`
