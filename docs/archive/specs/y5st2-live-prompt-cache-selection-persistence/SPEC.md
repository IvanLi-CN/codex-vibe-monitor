# Live 页 Prompt Cache 对话筛选本地记忆（#y5st2）

## 状态

- Status: 已完成
- Created: 2026-03-23
- Last: 2026-03-23

## 背景 / 问题陈述

- Live 页 `Prompt Cache Key 对话` 区块已经支持数量模式与最近活动时间模式筛选，但当前选择只保存在页面内存里。
- 用户刷新 `#/live` 或重新打开页面时，筛选会回退到默认的 `50 个对话`，导致重复操作，也让线上使用时的状态延续性很差。
- 这个需求只要求“记住浏览器里上次选过什么”，不需要后端账户级同步，也不需要改 URL。

## 目标 / 非目标

### Goals

- 让 `/live` 页 Prompt Cache 对话筛选优先恢复浏览器本地保存的上次选择。
- 保持现有筛选选项集合与后端请求参数语义不变。
- 当本地存储不可用、值非法或缺失时，稳定回退到默认 `count:50`，不影响页面可用性。
- 补齐前端回归测试，覆盖默认、非法值、写回与两类模式恢复。

### Non-goals

- 不修改 `GET /api/stats/prompt-cache-conversations` 接口或 `PromptCacheConversationSelection` 类型。
- 不把该筛选同步到 URL hash/query，也不做跨设备或账号级持久化。
- 不顺带让 Live 页的 `summaryWindow` 或 `limit` 也记忆化。
- 不改 Prompt Cache 对话筛选选项、隐含过滤规则、图表时间轴或表格布局。

## 范围（Scope）

### In scope

- `web/src/pages/Live.tsx`：读取/写入本地持久化的 Prompt Cache 筛选值。
- `web/src/pages/Live.test.tsx`：补持久化与回退回归测试。
- `docs/specs/README.md` 与当前 spec：记录范围、进度与 fast-track 交付状态。

### Out of scope

- 后端 API、SSE、缓存 key、查询参数与图表组件。
- 其它页面或其它筛选器的持久化。
- Storybook 视觉资产与 PR 截图。

## 需求（Requirements）

### MUST

- 仅使用浏览器 `localStorage` 记住 Prompt Cache 筛选值。
- 本地持久化格式必须复用现有 option id，例如 `count:50`、`activityWindow:6`。
- 首次渲染时必须先恢复存储值，再发出首个 `usePromptCacheConversations(...)` 请求。
- 非法值、旧值、读写异常都必须静默降级到默认 `count:50`。

### SHOULD

- 持久化逻辑只影响 Prompt Cache 对话筛选，不把状态管理扩散到其它 Live 控件。
- 读取/写入 helper 保持轻量，沿用项目现有本地存储容错模式。

### COULD

- None

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 页面首次打开时：
  - 若 `localStorage` 中存在合法的 Prompt Cache 筛选 option id，则恢复该值。
  - 否则使用默认 `count:50`。
- 用户切换 Prompt Cache 筛选时：
  - 立即更新页面选择值。
  - 同步把同一 option id 写回 `localStorage`。
- 用户刷新页面或重新进入 `#/live` 时：
  - 使用最近一次成功写回的 option id 恢复筛选。

### Edge cases / errors

- 若浏览器拒绝访问 `localStorage`，页面继续用默认值工作，不抛错。
- 若存储值不在现有选项集合内，忽略该值并回退到默认 `count:50`。
- 若切换筛选时写存储失败，UI 与接口请求仍按当前用户选择继续工作。

## 接口契约（Interfaces & Contracts）

None

## 验收标准（Acceptance Criteria）

- Given 浏览器本地没有保存值，When 打开 `/live`，Then Prompt Cache 筛选显示 `50 个对话`，且首个请求发送 `limit=50`。
- Given 本地保存值是无效字符串，When 打开 `/live`，Then 页面回退到 `50 个对话`，且不会因为非法值报错。
- Given 用户把筛选切换到 `20 个对话`，When 选择完成，Then 同一 option id 被写入 `localStorage`。
- Given 本地保存值是 `count:20`，When 刷新或重新打开 `/live`，Then 首个请求发送 `limit=20`。
- Given 本地保存值是 `activityWindow:6`，When 打开 `/live`，Then 首个请求只发送 `activityHours=6`，不会误发默认 `limit=50`。

## 实现前置条件（Definition of Ready / Preconditions）

- Prompt Cache 筛选选项集合与默认值已明确
- 本地持久化范围限定为 Prompt Cache 对话筛选，不扩展到其它控件
- 验收标准覆盖默认、非法值、写回、count/activityWindow 恢复
- 不涉及后端或跨端契约变更

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `cd web && bunx vitest run src/pages/Live.test.tsx`

### Quality checks

- Web build: `cd web && bun run build`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增索引行并在完成后同步状态

## 计划资产（Plan assets）

- None

## Visual Evidence (PR)

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: Live 页 Prompt Cache 筛选在首屏渲染前恢复本地保存值，并对非法/异常场景回退到 `count:50`。
- [x] M2: 用户切换 Prompt Cache 筛选时会把现有 option id 写回本地存储，且不影响其它 Live 控件。
- [x] M3: `Live.test.tsx` 覆盖默认、非法值、写回以及 count/activityWindow 恢复场景。
- [x] M4: fast-track 完成提交、PR 创建、review-loop 收敛与 spec-sync，同步到可合并前状态。

## 方案概述（Approach, high-level）

- 复用 `Live.tsx` 现有的选项常量，把 selection state 的真相源改成“受保护的 option id 字符串”。
- 通过静态 lookup 在 option id 与 `PromptCacheConversationSelection` 之间映射，保证首个 hook 调用就使用恢复后的选择。
- 沿用项目已有的 `localStorage` try/catch 模式，避免私密模式或浏览器限制导致页面异常。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：未来若筛选选项集合调整，旧存储值可能失效；本次通过“非法值回退默认”兜底。
- 需要决策的问题：None
- 假设（需主人确认）：浏览器本地记忆仅针对当前设备/当前浏览器，不需要 URL 或服务端同步。

## 变更记录（Change log）

- 2026-03-23: 新建 follow-up spec，冻结 Live 页 Prompt Cache 对话筛选的前端本地记忆边界与验收标准。
- 2026-03-23: 完成 Live 页 Prompt Cache 筛选的本地持久化实现与页面回归测试，本地 `vitest + build` 已通过，等待快车道 PR 收口。
- 2026-03-23: PR #207 完成 spec-sync，并在 `codex review --base origin/main` 下确认无新增待修项。

## 参考（References）

- `docs/specs/m4c2q-prompt-cache-conversation-filter-window/SPEC.md`
- `web/src/pages/Live.tsx`
