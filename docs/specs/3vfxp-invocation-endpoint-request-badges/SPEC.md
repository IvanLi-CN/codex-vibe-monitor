# InvocationTable 请求类型 Badge 化（#3vfxp）

## 状态

- Status: 部分完成（2/3）
- Created: 2026-03-22
- Last: 2026-03-22

## 背景 / 问题陈述

- 当前 `InvocationTable` 摘要区直接显示原始 endpoint 路径，只有 `compact` 路径额外套了一个 `text-info` 着色特判；对 Dashboard / Live 首屏来说，可读性不够稳定。
- 已识别请求实际上只有少数几类，但用户仍需要先读完整路径才能判断是 `Responses`、`Chat` 还是 `Compact`，这让“最近请求”表更像调试原文而不是监控摘要。
- 如果继续沿用 compact-only 特判，后续再补其它已识别请求类型时会继续把 endpoint 语义散落在组件里，测试也会维持一堆路径样式特例。

## 目标 / 非目标

### Goals

- 让 `InvocationTable` 摘要区对已识别请求优先显示人类可读 badge：`Responses`、`Chat`、`远程压缩 / Compact`。
- 仅对精确匹配的 `/v1/responses`、`/v1/chat/completions`、`/v1/responses/compact` 启用 badge，避免误伤未知路径。
- 保持展开详情继续显示原始 endpoint 文本，兼顾首屏可读性与排障信息。
- 统一桌面与移动摘要的 endpoint 渲染逻辑，并补齐 Vitest / Storybook / Playwright 回归。

### Non-goals

- 不修改 Rust 后端采集逻辑、`/api/invocations` 返回字段、SQLite 或 SSE 契约。
- 不扩展 `InvocationRecordsTable`、Records 页筛选建议或其它表格的 endpoint 呈现。
- 不做 endpoint 前缀归类、模糊识别或额外 API 字段补充。

## 范围（Scope）

### In scope

- `web/src/lib/invocation.ts`：新增 endpoint 展示语义 helper。
- `web/src/components/InvocationTable.tsx`：摘要区 recognized badge / raw fallback 与详情 raw endpoint 保留。
- `web/src/i18n/translations.ts`、`web/src/components/InvocationTable.stories.tsx`、`web/src/components/InvocationTable.test.tsx`、`web/tests/e2e/invocation-table-layout.spec.ts`。
- `docs/specs/README.md` 与本 spec 的状态同步。

### Out of scope

- `src/**`、`web/src/lib/api.ts`、`web/src/components/InvocationRecordsTable.tsx`。
- 新增公开接口、契约文档或 asset promotion。
- 其它 endpoint 分类策略与新请求类型命名体系。

## 需求（Requirements）

### MUST

- 摘要区对精确匹配的 `/v1/responses`、`/v1/chat/completions`、`/v1/responses/compact` 渲染人类可读 badge，不再直接显示原始路径。
- 摘要区对未知 endpoint 保持现有 raw path monospace 截断展示。
- 展开详情必须继续显示原始 endpoint 文本值，不得被 badge 替代。
- recognized badge 必须带稳定选择器 `data-testid="invocation-endpoint-badge"`，并用 `data-endpoint-kind` 区分 `responses` / `chat` / `compact`。
- raw fallback 必须继续使用 `data-testid="invocation-endpoint-path"`，并统一输出 `data-endpoint-kind="raw"`。

### SHOULD

- endpoint 识别逻辑集中到单一 helper，避免 `InvocationTable` 内继续散落 compact-only 判断。
- 新增 badge 不能破坏桌面无横向滚动、长代理名省略和详情按钮位置的既有回归约束。
- 中英文文案都通过 i18n key 驱动，不直接在 JSX 硬编码文本。

### COULD

- None

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- Dashboard / Live 摘要区读取现有 `record.endpoint`，经 helper 精确匹配三类已识别 endpoint 后渲染对应 badge。
- `/v1/responses` 显示 `Responses`，`/v1/chat/completions` 显示 `Chat`，`/v1/responses/compact` 显示 `Compact / 远程压缩`。
- 未识别 endpoint（例如带长后缀的 debug/test 路径）继续显示原始路径文本，并沿用现有 `truncate` 行为。
- 展开详情保持原始 endpoint 文本值，作为摘要 badge 的排障兜底。

### Edge cases / errors

- `record.endpoint` 缺失或为空时，摘要区仍按 raw fallback 显示 `—`。
- 只接受修剪后的精确匹配；`/v1/responses/`、`/v1/responses/foo`、`/v1/chat/completions/extra` 都必须留在 raw fallback。
- recognized badge 即使使用人类可读文本，也应保留 `title` 中的原始 endpoint，方便桌面悬浮排障。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `InvocationTable` endpoint summary rendering | UI presentation rule | internal | Modify | None | web | Dashboard / Live | 仅前端摘要语义变化，详情 raw endpoint 保留 |
| `resolveInvocationEndpointDisplay` | TS helper | internal | New | None | web | InvocationTable | 纯展示语义 helper，不改 API 数据流 |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given `/v1/responses`、`/v1/chat/completions`、`/v1/responses/compact` 记录，When 渲染 `InvocationTable` 摘要区，Then 桌面与移动都显示对应 badge，而不是原始路径。
- Given 未识别 endpoint（如 `/v1/responses/very-long-segment-...`），When 渲染摘要区，Then 仍显示原始路径文本且维持截断行为。
- Given 任一已识别 endpoint 记录，When 展开详情，Then 原始 endpoint 文本仍完整可见。
- Given 更新后的 Dashboard / Live 表格，When 运行现有布局回归，Then 不出现新的横向滚动、badge 挤压、展开按钮错位或长代理名回归。
- Given 中英文 locale，When 渲染已识别 endpoint badge，Then 文案来自 i18n key 而不是 JSX 硬编码。

## 实现前置条件（Definition of Ready / Preconditions）

- 只覆盖 `InvocationTable` 摘要区与详情区 endpoint 展示，不扩展到 Records 表：已明确
- recognized endpoint 集合固定为三个精确匹配路径：已明确
- 快车道本轮终点为 merge-ready，不自动 merge：已明确

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `cd web && bunx vitest run src/components/InvocationTable.test.tsx`
- E2E tests (if applicable): `cd web && bun run test:e2e -- invocation-table-layout.spec.ts`

### UI / Storybook (if applicable)

- Stories to add/update: `web/src/components/InvocationTable.stories.tsx`
- Visual regression baseline changes (if any): 复用现有 InvocationTable layout 回归，不新增截图门槛

### Quality checks

- `cd web && bun run build`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 追加本 spec 索引并同步状态

## 计划资产（Plan assets）

- Directory: `docs/specs/3vfxp-invocation-endpoint-request-badges/assets/`
- In-plan references: None
- PR visual evidence source: maintain `## Visual Evidence (PR)` in this spec when PR screenshots are needed.

## Visual Evidence (PR)

None yet.

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新增 endpoint 展示 helper，并让 `InvocationTable` 摘要区按 recognized badge / raw fallback 双路径渲染。
- [x] M2: 补齐 i18n、Storybook、Vitest 与 Playwright 回归，覆盖三类 recognized badge 和一类未知 raw endpoint。
- [ ] M3: 完成本地定向验证与 fast-track 收敛到 merge-ready。

## 方案概述（Approach, high-level）

- 用一个纯 helper 聚合 endpoint 展示语义，组件层只负责按 helper 结果选择 badge 或 raw path。
- recognized badge 仅服务摘要区；详情区继续保持 raw endpoint 原文，避免牺牲排障信息。
- 通过更新既有 `InvocationTable` layout regression，而不是另建一套新测试，保证 badge 改动不会破坏既有布局承诺。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：如果 recognized badge 样式处理不当，可能重新引入桌面列宽抖动或压缩长代理名回归。
- 风险：旧测试大量依赖 compact-only 路径着色，若不整体更新会造成“实现正确但断言语义过时”的噪音失败。
- 需要决策的问题：None
- 假设（需主人确认）：None

## 变更记录（Change log）

- 2026-03-22: 创建 spec，冻结 recognized endpoint 集合、摘要 badge / 详情 raw endpoint 边界与验证口径。
- 2026-03-22: 完成 helper、InvocationTable 摘要区、人类可读 badge、i18n、Storybook、Vitest 与独立租约端口上的 Playwright 布局回归；等待 PR 收敛完成 M3。

## 参考（References）

- `docs/specs/g3amk-codex-remote-compact-observability/SPEC.md`
- `docs/specs/5gqdb-invocation-proxy-name-truncation-hotfix/SPEC.md`
- `docs/specs/7n2ex-invocation-account-latency-drawer/SPEC.md`
