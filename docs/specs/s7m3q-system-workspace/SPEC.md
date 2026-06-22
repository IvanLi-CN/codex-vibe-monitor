# 系统工作区重构（#s7m3q）

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## 背景 / 问题陈述

- 顶层 `设置` 当前同时承载系统级配置、forward proxy 诊断与运行信息，信息架构已经过载。
- 系统运行状态与后台任务执行情况缺少稳定入口，用户只能从零散页面或数据库侧面推断。
- 现有 UI 已有 `AppLayout` 顶层壳子与 `AccountPoolLayout` 子工作区模式，可以复用为新的系统工作区。

## 目标 / 非目标

### Goals

- 把顶层 `设置` 升级为顶层 `系统` 工作区，采用左侧导航、右侧子路由出口的两栏布局。
- 在 `系统` 下稳定提供 `状态 / 任务 / 设置 / 代理` 四个子界面。
- 新增系统状态读接口，展示调用成功数、非成功数、归档与未归档 body 数量/体积、数据库体积、其他文件体积，并按 60 秒轮询刷新。
- 新增系统后台任务记录读接口，至少覆盖 scheduler、retention/archive（含 raw compression 摘要）、startup backfill、forward-proxy subscription refresh。
- 保持现有 `/api/settings*` 写接口契约不变；原设置能力按职责拆到 `系统/设置` 与 `系统/代理`。

### Non-goals

- 不重做 `dashboard / stats / live / records / account-pool` 的顶层结构。
- 不新增手动重跑、暂停/恢复、告警通知、权限系统。
- 不把账号池 maintenance records 合并进系统任务页首版。
- 不改变现有设置保存字段形状。

## 范围（Scope）

### In scope

- Web 路由：`/system/*` 父工作区、旧 `/settings` 兼容跳转、顶层导航文案改名。
- Web 页面：SystemLayout、Status/Tasks/Settings/Proxy 四个子页，以及设置页内容拆分。
- Rust API：`GET /api/system/status`、`GET /api/system/tasks`。
- Rust persistence：新增 `system_task_runs` 表与轻量任务记录写入。
- Storybook / tests / visual evidence：系统工作区页面级 story、导航回归、旧路径跳转与状态轮询验证。

### Out of scope

- 账号池 maintenance 事件模型重构。
- archived 明细在线回放。
- 新的系统级 SSE 频道。

## 信息架构

### 顶层导航

- 原 `设置` 顶层入口改名为 `系统`。
- 顶层入口默认进入 `#/system/status`。
- 旧 `#/settings` 保留兼容入口，但只做重定向到 `#/system/settings`。

### 子导航

- `状态`：系统级汇总指标。
- `任务`：系统后台任务执行记录。
- `设置`：原设置页中的非 forward-proxy 能力。
- `代理`：原设置页中的 forward-proxy 能力。

## 功能与行为规格

### `系统/状态`

- MUST 展示：
  - 调用成功数
  - 调用非成功数
  - 已归档 body 数量
  - 已归档 body 体积
  - 未归档 body 数量
  - 未归档 body 体积
  - 数据库体积
  - 其他文件体积
- MUST 每 60 秒自动刷新一次，并显示“上次刷新时间 / 刷新中”状态。
- “非成功数”按 `status != success` 统计，包含失败与未完成状态；页面文案需明确这是系统口径。
- “已归档 body” 在首版按 `archive_batches.dataset='codex_invocations' AND status='completed'` 的归档调用行数 / 归档文件实际大小统计。
- “未归档 body” 在首版按 live `codex_invocations.response_raw_path IS NOT NULL` 的记录数 / `response_raw_size` 汇总统计。

### `系统/任务`

- MUST 默认展示系统后台任务执行记录，而不是账号池维护事件。
- 首版任务类型至少覆盖：
  - `scheduler_poll`
  - `retention_archive`
  - `startup_backfill`
  - `forward_proxy_subscription_refresh`
- retention 任务摘要必须包含 raw compression / archive / prune 等关键计数，避免再拆单独任务调度器。
- 列表至少支持任务类型、结果状态与时间范围的基础筛选。

### `系统/设置`

- 保留原设置页中的：
  - proxy/hijack 与 websocket runtime 设置
  - pricing 设置
  - external API keys 设置
- 保存语义继续复用 `useSettings` 与现有 `/api/settings*` 写接口。

### `系统/代理`

- 承载原 settings 页中的 forward-proxy 能力：
  - proxy URL / subscription URL 管理
  - 节点表
  - 节点延迟测试
  - 手动刷新订阅

## 接口契约（Interfaces & Contracts）

### 接口清单

| 接口（Name）             | 类型（Kind） | 范围（Scope） | 变更（Change） | 使用方（Consumers） |
| ------------------------ | ------------ | ------------- | -------------- | ------------------- |
| `GET /api/system/status` | HTTP JSON    | internal      | New            | `系统/状态`         |
| `GET /api/system/tasks`  | HTTP JSON    | internal      | New            | `系统/任务`         |

## 验收标准（Acceptance Criteria）

- Given 顶层导航渲染完成，When 用户点击 `系统`，Then 进入 `#/system/status`。
- Given 用户访问 `#/settings`，When 路由解析，Then 重定向到 `#/system/settings`。
- Given `系统/状态` 页面加载，When 数据返回，Then 页面展示八项系统指标并包含刷新时间反馈。
- Given `系统/任务` 页面加载，When 查询返回，Then 页面展示系统后台任务记录且不混入账号池维护事件。
- Given 用户进入 `系统/设置`，When 调整原有常规设置，Then 保存行为与旧设置页一致。
- Given 用户进入 `系统/代理`，When 操作 forward proxy，Then 现有校验、测速、刷新订阅能力保持可用。

## 非功能性验收 / 质量门槛

### Testing

- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run build-storybook`
- shell/layout e2e 更新后通过

### UI / Storybook

- 必须新增系统工作区页面级 stories，覆盖 `状态 / 任务 / 设置 / 代理` 四个子页的 mock 状态。
- 顶层导航 story / test 必须从 `设置` 更新为 `系统`。

## Visual Evidence

- source_type: storybook_canvas
  story_id_or_title: System/SystemWorkspace/Status
  target_program: mock-only
  capture_scope: browser-viewport
  requested_viewport: 1440x1280
  viewport_strategy: storybook-viewport
  sensitive_exclusion: N/A
  submission_gate: owner-approved
- evidence_note: 验证系统工作区左右两栏结构、左侧子导航、状态页指标卡与刷新状态。
  snapshot_path: `/Users/ivan/.codex/user-inline-assets/codex-vibe-monitor__2e728e5d/2026/06/22/20260622T043310Z-status-87ad6f12.png`
- source_type: storybook_canvas
  story_id_or_title: System/SystemWorkspace/Tasks
  target_program: mock-only
  capture_scope: browser-viewport
  requested_viewport: 1440x1280
  viewport_strategy: storybook-viewport
  sensitive_exclusion: N/A
  submission_gate: owner-approved
  evidence_note: 验证系统任务页的列表、筛选器、分页摘要与上一页/下一页控件布局。
  snapshot_path: `/Users/ivan/.codex/user-inline-assets/codex-vibe-monitor__2e728e5d/2026/06/22/20260622T051604Z-system-tasks-pagination-trimmed-3bf3d193.png`
- source_type: storybook_canvas
  story_id_or_title: System/SystemWorkspace/Settings
  target_program: mock-only
  capture_scope: browser-viewport
  requested_viewport: 1440x1280
  viewport_strategy: storybook-viewport
  sensitive_exclusion: N/A
  submission_gate: owner-approved
  evidence_note: 验证系统设置页保留原常规设置分区后的布局与层次。
  snapshot_path: `/Users/ivan/.codex/user-inline-assets/codex-vibe-monitor__2e728e5d/2026/06/22/20260622T043310Z-settings-3aff3f94.png`
- source_type: storybook_canvas
  story_id_or_title: System/SystemWorkspace/Proxy
  target_program: mock-only
  capture_scope: browser-viewport
  requested_viewport: 1440x1280
  viewport_strategy: storybook-viewport
  sensitive_exclusion: N/A
  submission_gate: owner-approved
  evidence_note: 验证 forward-proxy 能力迁移到系统代理页后的布局与信息密度。
  snapshot_path: `/Users/ivan/.codex/user-inline-assets/codex-vibe-monitor__2e728e5d/2026/06/22/20260622T043310Z-proxy-c881c4b7.png`

## 风险 / 开放问题 / 假设

- 风险：`已归档 body` 在现有架构下只能近似映射为 archived invocation rows 与 archive batch file size，页面需明确口径。
- 风险：后台任务已有多种内部子步骤，首版任务记录只保留可读摘要，不扩展成完整事件流。
- 假设：状态页采用前端 60 秒轮询足以满足系统观察需求，不新增 SSE。

## 参考

- `web/src/pages/Settings.tsx`
- `web/src/pages/account-pool/AccountPoolLayout.tsx`
- `src/maintenance/retention.rs`
- `src/maintenance/startup_backfill.rs`
- `src/runtime.rs`
