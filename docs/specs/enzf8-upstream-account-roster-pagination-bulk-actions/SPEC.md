# 上游账号列表分页、跨页选择与批量操作（#enzf8）

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-22
- Last: 2026-03-22

## 背景 / 问题陈述

- 上游账号列表已经具备基础筛选与详情能力，但当前仍是一次性加载全部账号，账号数增长后会放大首屏负载，也不利于批量管理。
- 列表目前缺少跨页选择与成组操作，用户无法对几十到上百个账号执行统一启停、同步、删改分组或标签。
- 现有状态展示仍混合原始持久化状态与前端基于 `lastError` 的临时推断，badge、筛选项和 attention 统计容易出现口径漂移。
- 批量同步需要与已有单账号 actor 串行约束兼容，同时保持前端可见的实时进度，避免“点了没反馈”。

## 目标 / 非目标

### Goals

- 将 `GET /api/pool/upstream-accounts` 改为服务端分页，`pageSize` 仅支持 `20 / 50 / 100`，默认 `20`。
- 统一收口 7 个展示状态：`active`、`syncing`、`needs_reauth`、`upstream_unavailable`、`upstream_rejected`、`error_other`、`disabled`。
- 列表支持跨页选择、当前页全选和批量操作，分页切换时保留 `selectedIds`，筛选条件或 `pageSize` 变化时清空选择并回到第 1 页。
- 新增同步外批量动作：`delete`、`enable`、`disable`、`set_group`、`add_tags`、`remove_tags`，返回逐账号结果。
- 新增批量同步后台 job + SSE 进度流，前端可显示 snapshot、逐行结果和终态。

### Non-goals

- 不扩展数据库中原始 `status` 枚举。
- 不提供“选中当前全部筛选结果”的超集选择入口。
- 不把所有批量操作都改成后台 job。
- 不新增批量组备注编辑或新的账号详情能力。

## 范围（Scope）

### In scope

- `src/upstream_accounts/mod.rs`：列表分页、展示状态分类器、批量 mutation、批量同步 job 与 SSE 事件流。
- `src/main.rs`：批量操作与批量同步相关路由接线。
- `web/src/lib/api.ts`、`web/src/hooks/useUpstreamAccounts.ts`：分页查询参数、展示状态、批量操作与批量同步接口对接。
- `web/src/pages/account-pool/UpstreamAccounts.tsx`、`web/src/components/UpstreamAccountsTable.tsx`：状态筛选、跨页选择、批量工具条、分页 footer、批量对话框与同步进度展示。
- `web/src/i18n/translations.ts`、相关 Vitest / Storybook，以及 `docs/specs/README.md`。

### Out of scope

- 账号详情页的非批量交互重构。
- “当前筛选结果全选”服务端游标协议。
- 原始 `status` 的存储结构变更或历史数据迁移。

## 接口契约（Interfaces & Contracts）

### `GET /api/pool/upstream-accounts`

- 新增 query 参数：
  - `status`
  - `page`
  - `pageSize`
- `pageSize` 非法值统一回退到 `20`，有效值仅允许 `20 / 50 / 100`。
- 返回体新增：
  - `total`
  - `page`
  - `pageSize`
  - `metrics`
- 每个账号 summary 新增 `displayStatus`，用于列表 badge、筛选和顶部 attention 统计。

### 展示状态分类

- 服务端统一派生 `displayStatus`，优先级如下：
  - `disabled`
  - `syncing`
  - `needs_reauth`
  - `upstream_unavailable`
  - `upstream_rejected`
  - `error_other`
  - `active`
- 前端不再基于 `lastError` 自行推断筛选键；仅保留 OAuth bridge legacy hint 的文案识别。

### 批量操作

- `POST /api/pool/upstream-accounts`
  - 请求体包含 `action` 与显式 `accountIds[]`。
  - 支持动作：`delete`、`enable`、`disable`、`set_group`、`add_tags`、`remove_tags`。
  - 返回逐账号结果：成功、失败或跳过原因彼此独立，不做整批回滚。

### 批量同步

- `POST /api/pool/upstream-accounts/bulk-sync-jobs`
  - 创建后台同步 job，只接受显式 `accountIds[]`。
  - 同一时间最多存在 1 个运行中的 job；若已有运行中 job，创建请求直接返回该 job 的当前 snapshot，不再新建任务。
- `GET /api/pool/upstream-accounts/bulk-sync-jobs/:jobId`
  - 返回当前 snapshot。
- `GET /api/pool/upstream-accounts/bulk-sync-jobs/:jobId/events`
  - SSE 事件至少包含：`snapshot`、`row`、`completed`、`failed`、`cancelled`。
- `DELETE /api/pool/upstream-accounts/bulk-sync-jobs/:jobId`
  - 取消尚未完成的 job。
- disabled 账号在批量同步中允许被标记为 `failed` 或 `skipped`，其它账号继续执行。

## 验收标准（Acceptance Criteria）

- Given 打开上游账号页，When 首次请求列表，Then 使用 `page=1&pageSize=20`，并能在界面切换为 `50` 或 `100`。
- Given 用户在第 1 页和第 2 页分别勾选账号，When 翻页往返，Then 已选数量持续累计且对应页的勾选状态保持不变。
- Given 已存在跨页选择，When 修改状态、分组、标签筛选或 `pageSize`，Then 选择立即清空且页码回到第 1 页。
- Given 用户切换状态筛选，When 查看列表 badge、筛选结果和顶部 attention 卡，Then 三者都基于同一 `displayStatus` 口径。
- Given 用户执行批量启用、停用、删除、设置分组、加标签或减标签，When 请求完成，Then 返回逐账号成功/失败摘要且成功项不因单账号失败回滚。
- Given 用户发起批量同步，When job 运行中，Then 页面通过 SSE 持续收到 snapshot 与 row 进度，并在完成后刷新列表。
- Given 用户在批量同步创建请求尚未返回时连续点击，When 请求命中前后端限制，Then 只会保留 1 个运行中的同步 job，后续创建请求复用现有 job。
- Given 批量同步中包含 disabled 账号，When job 结束，Then disabled 账号被标记为失败或跳过原因，其余账号仍继续同步。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust: `cargo test upstream_accounts -- --nocapture`
- Web: `cd web && bun run test -- src/components/UpstreamAccountsTable.test.tsx src/pages/account-pool/UpstreamAccounts.test.tsx`

### Quality checks

- Rust format/typecheck: `cargo fmt --check`、`cargo check`
- Web build: `cd web && bun run build`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 spec，冻结分页参数、展示状态、选择清空规则与批量同步 job 契约。
- [x] M2: 后端列表接口补齐 `status/page/pageSize`、`displayStatus` 与分页返回字段。
- [x] M3: 后端新增批量 mutation 与批量同步 job/SSE。
- [x] M4: 前端落地状态筛选、跨页选择、批量工具条、分页 footer 与批量对话框。
- [x] M5: 补齐相关测试、更新 README 索引，并收敛到 merge-ready。

## 风险 / 假设

- 假设：展示状态仅用于 UI 展示与筛选，底层持久化原始 `status` 继续保留现有语义。
- 假设：跨页选择仅在当前筛选集合内有效，任一筛选条件变更即清空。
- 风险：批量同步需要同时维护 actor 串行和 SSE 快照一致性，若 row 终态与 snapshot 聚合不同步，前端进度面板会出现闪烁或错误计数。
- 风险：批量标签增减依赖逐账号读取并更新标签集合，若结果摘要口径不清晰，容易让用户误判部分成功。

## 变更记录（Change log）

- 2026-03-22: 创建 spec，冻结分页、展示状态、跨页选择、批量操作与批量同步的范围和契约。
- 2026-03-22: 完成后端列表分页、`displayStatus` 分类器、批量 mutation、批量同步 job/SSE，以及前端跨页选择、分页 footer、状态筛选和批量交互 UI。
- 2026-03-22: 本地验证通过 `cargo fmt --check`、`cargo check`、`cargo test upstream_accounts -- --nocapture`、`cd web && bun run build` 与定向 Vitest 回归，按 fast-track 收口到 merge-ready。
