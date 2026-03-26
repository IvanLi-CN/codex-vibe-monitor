# Live Prompt Cache 调用记录展开与历史抽屉（#3vm5e）

## 状态

- Status: 已完成（5/5，PR #246）
- Created: 2026-03-26
- Last: 2026-03-26

## 背景 / 问题陈述

- Live 页的 `Prompt Cache Key 对话` 表当前只能看到对话聚合 totals 与 24 小时趋势，无法直接回看某个对话最近落下来的原始调用。
- 运维排查时既需要在表内快速扫一眼“最近 5 条调用记录”，也需要在不跳页的前提下展开完整历史，确认模型、账号、接口与状态分布。
- 当前一键查看多个对话时，如果每一行都单独请求 preview，会让“全部展开”退化成 N+1 请求模式，响应抖动明显。

## 目标 / 非目标

### Goals

- 为 Prompt Cache 对话表新增两类行级动作：
  - 表内展开最近 5 条调用记录；
  - 右侧抽屉查看该对话的全部保留调用记录。
- 将 `recentInvocations[<=5]` 直接并入 `GET /api/stats/prompt-cache-conversations` 主响应，避免“一键展开全部”触发按行补请求。
- 在 Live 页对话筛选控件左侧新增“展开所有记录 / 收起所有记录”按钮，只作用于当前可见结果集。
- 保持 Prompt Cache 对话筛选、24h 趋势图、上游账号抽屉、SSE 刷新与本地筛选记忆语义不变。

### Non-goals

- 不新增独立的 Prompt Cache 历史后端路由。
- 不尝试从 hourly rollup 反推出已删除 raw row 的逐条调用详情。
- 不改 `Records` 页筛选模型、调用详情字段或 Sticky Key 对话表行为。

## 范围（Scope）

### In scope

- `src/api/mod.rs` 与 `src/tests/mod.rs`：Prompt Cache 对话接口新增 `recentInvocations[]` 与回归测试。
- `web/src/lib/api.ts` 与 `web/src/lib/api.test.ts`：新增 preview 类型与 normalize。
- `web/src/pages/Live.tsx`：批量展开状态上提、对话筛选头部新增“一键展开/收起”按钮。
- `web/src/components/PromptCacheConversationTable.tsx`、测试与 Storybook：表内 preview、完整历史抽屉、交互与空态。
- `web/src/i18n/translations.ts`、本 spec 与 `docs/specs/README.md`。

### Out of scope

- Prompt Cache 对话筛选选项、localStorage key、图表时间轴与累计 totals 口径。
- 已归档但不再存在 raw row 的逐条调用恢复。
- 其他页面的调用记录展示方式。

## 接口契约（Interfaces & Contracts）

- `GET /api/stats/prompt-cache-conversations`
  - 每个 `conversation` 新增：
    - `recentInvocations: Array<{ id: number; invokeId: string; occurredAt: string; status: string; model: string | null; totalTokens: number; cost: number | null; proxyDisplayName: string | null; upstreamAccountId: number | null; upstreamAccountName: string | null; endpoint: string | null }>`
  - `recentInvocations` 固定按 `occurredAt DESC, id DESC` 排序，最多返回 5 条。
  - preview 仅覆盖当前仍存在 raw row 的记录；聚合 totals 继续使用全历史统计。
- `GET /api/invocations`
  - 抽屉复用现有 `promptCacheKey + page/pageSize + sortBy=occurredAt + sortOrder=desc` 查询。
  - 抽屉会在前端自动翻页，直到 `loadedCount === total` 或遇到分页错误。

## 验收标准（Acceptance Criteria）

- Given 任一 Prompt Cache 对话行可见，When 点击 preview icon，Then 仅该行在表内展开最近 5 条调用记录，再次点击可收起。
- Given 最近 5 条调用记录或历史抽屉可见，When 调用记录渲染完成，Then 必须复用 `Records` 页同款调用记录表格，而不是独立卡片样式。
- Given 当前筛选结果中存在多个对话，When 点击头部“展开所有记录 / 收起所有记录”，Then 仅当前可见结果集一起展开或收起，不写入本地持久化。
- Given 点击历史抽屉 icon，When 抽屉打开，Then 以时间倒序展示该 Prompt Cache Key 的全部保留调用记录，并自动续拉后续页直到拉满。
- Given 某对话 totals 仍存在但 raw rows 已被清理，When 表内 preview 与抽屉渲染，Then 显示“暂无调用记录”，同时 totals / 图表 / 上游账号摘要保持正常。
- Given Live 数据因 SSE 或轮询刷新，When 对话仍在当前结果集中，Then 已展开 preview 保持展开；若对话被筛掉，其展开状态会被清理。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust: `cargo test prompt_cache_conversations -- --nocapture`
- Web: `cd web && bun run test -- src/components/PromptCacheConversationTable.test.tsx src/lib/api.test.ts src/pages/Live.test.tsx`
- Storybook: `cd web && bun run build-storybook`

### Quality checks

- Rust format: `cargo fmt`
- Front-end type/build: `cd web && bun run test --runInBand` 或等效 Vitest 子集 + Storybook build

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 spec，冻结 Prompt Cache 调用记录 preview 与历史抽屉的接口/交互口径。
- [x] M2: 后端对话响应补齐 `recentInvocations[]`，并通过 limit/source-scope/empty-preview 回归测试。
- [x] M3: Live 页与 Prompt Cache 表格完成批量展开、行内 preview 与历史抽屉交互。
- [x] M4: 前端 normalize、Vitest 与 Storybook 覆盖齐备。
- [x] M5: 视觉证据、review-loop 与 PR 收敛到 merge-ready。

## Visual Evidence

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: Monitoring/PromptCacheConversationTable / Populated
  state: collapsed table
  evidence_note: 验证默认折叠态下的 Prompt Cache 对话表仍保持现有双列摘要、时间列与 24 小时累计趋势不变。
  image:
  ![Prompt Cache 调话表折叠态](./assets/prompt-cache-records-collapsed.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: Monitoring/PromptCacheConversationTable / SingleExpanded
  state: single row expanded
  evidence_note: 验证单行展开时只显示该对话最近 5 条调用记录，并直接复用 `Records` 页同款调用记录表格。
  PR: include
  image:
  ![Prompt Cache 对话单行展开态](./assets/prompt-cache-records-single-expanded.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: Monitoring/PromptCacheConversationTable / ExpandAll
  state: all rows expanded
  evidence_note: 验证所有对话同时展开时，每个对话都追加同款调用记录表格，而不是退化成多套卡片区块。
  image:
  ![Prompt Cache 对话全部展开态](./assets/prompt-cache-records-expand-all.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: browser-viewport
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: Monitoring/PromptCacheConversationTable / DrawerOpen
  state: retained history drawer open
  evidence_note: 验证“全部调用记录”抽屉按时间倒序展示保留 raw records，并复用同款调用记录表格与数量统计。
  PR: include
  image:
  ![Prompt Cache 对话历史抽屉](./assets/prompt-cache-records-drawer-open.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: Monitoring/Live Prompt Cache Section / InteractiveFilters
  state: live section bulk expand control
  evidence_note: 验证 Live 页对话筛选左侧新增“一键展开所有记录”按钮，并且点击后当前可见结果集统一展开。
  PR: include
  image:
  ![Live Prompt Cache 一键展开](./assets/prompt-cache-live-bulk-expand.png)
