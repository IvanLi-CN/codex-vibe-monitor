# 绑定代理节点改为“本组真实流量统计”（#3np57）

## 状态

- Status: 进行中
- Created: 2026-04-13
- Last: 2026-04-13

## 背景 / 问题陈述

- 号池分组设置弹窗里的“绑定代理节点”`24H` 成功/失败，当前实际读取的是全局 `forward_proxy_attempt_hourly`。
- 这份统计既没有分组作用域，也会混入 forward-proxy probe / penalized 探测结果，因此会把“当前组真实请求量”误显示成“全局节点尝试数”。
- 用户期望是：弹窗只展示当前组真实 pool 请求在过去 24 小时内实际落到各绑定节点的成功/失败次数，并且 `Direct` 也要作为普通绑定节点参与统计。

## 目标 / 非目标

### Goals

- 将分组设置弹窗中的节点 `24H` 成功/失败改成“当前组真实请求尝试数”。
- 在真实 `pool_upstream_request_attempts` 落库时快照：
  - `group_name_snapshot`
  - `proxy_binding_key_snapshot`
- `GET /api/pool/forward-proxy-binding-nodes` 新增可选 `groupName` 参数；传入后返回该组真实 24 小时节点桶，未传时维持现有全局统计语义。
- `Direct` 使用 canonical key `__direct__` 参与分组统计。
- 为后端、前端、Storybook 与视觉证据补齐回归覆盖。

### Non-goals

- 不尝试把修复前的历史 24 小时数据做 best-effort 回填。
- 不修改其他页面的全局 forward-proxy 统计语义。
- 不引入新的 group id / immutable group identity 体系。

## 范围（Scope）

### In scope

- `src/schema.rs`
- `src/proxy/usage_persistence.rs`
- `src/proxy/failover.rs`
- `src/forward_proxy/slices/storage_and_hourly_stats.rs`
- `src/upstream_accounts/core_runtime_types.rs`
- `src/upstream_accounts/crud_group_notes.rs`
- `src/maintenance/retention.rs`
- `src/maintenance/hourly_rollups.rs`
- `web/src/lib/api/core-upstream.ts`
- `web/src/hooks/useForwardProxyBindingNodes.ts`
- `web/src/pages/account-pool/UpstreamAccounts.page-local-shared.tsx`
- `web/src/pages/account-pool/UpstreamAccountCreate.page-impl.tsx`
- 相关 Rust / Vitest / Storybook 回归与视觉证据

### Out of scope

- 全局 live/settings forward-proxy 统计
- 修复前历史窗口的精确归属迁移
- 绑定节点显示名、协议名或选择交互改版

## 数据契约

### Pool attempt snapshot fields

- live + archive `pool_upstream_request_attempts` 新增：
  - `group_name_snapshot TEXT`
  - `proxy_binding_key_snapshot TEXT`
- 快照只在真实 pool 请求尝试写入时记录；老数据无值时保持 `NULL`。

### Binding nodes API

- `GET /api/pool/forward-proxy-binding-nodes`
  - 新增可选 query: `groupName`
  - `groupName` 缺省：保留现有全局 `forward_proxy_attempt_hourly` 统计
  - `groupName` 存在：返回该组来自终态 `pool_upstream_request_attempts` 的 24 个小时桶

### Frontend contract

- `fetchForwardProxyBindingNodes(keys?, { includeCurrent?, groupName? })`
- `useForwardProxyBindingNodes(keys?, { enabled?, groupName? })`
- 两个组设置入口都必须把 `groupNoteEditor.groupName` 透传进去

## 聚合规则

- 只统计 `finished_at IS NOT NULL` 的真实 pool 尝试记录。
- 分组归属固定以 `group_name_snapshot` 为准。
- 代理归属固定以 canonical `proxy_binding_key_snapshot` 为准，不再依赖 display name 猜测。
- `status == success` 记入成功；其余终态记入失败。
- 没有快照字段的历史记录直接忽略，不做模糊归因。
- 被惩罚节点的 probe 结果不会进入该视图，因为数据源已切换为 pool 真实请求。

## 验收标准（Acceptance Criteria）

- Given 同一代理节点同时被 A / B 两组使用，When 分别打开两组弹窗，Then 每组只看到自己的真实请求数。
- Given penalized 节点发生 probe 成功/失败，When 打开组弹窗，Then 该 probe 不影响节点 `24H` 计数。
- Given 组内绑定多个节点且 node shunt 关闭，When 真实请求分别落到不同节点，Then 统计按实际选中的 canonical key 拆分。
- Given 本组真实请求走 `Direct`，When 打开组弹窗，Then `Direct` 节点出现对应 `24H` 计数。
- Given 调用方未传 `groupName`，When 请求 binding nodes endpoint，Then 返回值继续保持当前全局统计语义。
- Given 历史 24 小时内只有旧数据且没有新快照，When 打开组弹窗，Then 节点显示空桶/零值而不是“近似归因”。

## 质量门槛（Quality Gates）

- `cargo test --lib upstream_accounts::tests_part_1::forward_proxy_binding_nodes_query_keeps_repeated_keys_and_include_current`
- `cargo test forward_proxy_binding_nodes_ -- --nocapture`
- `cargo test begin_pool_upstream_request_attempt_with_scope_ -- --nocapture`
- `cd web && bunx vitest run src/lib/api.test.ts src/hooks/useForwardProxyBindingNodes.test.tsx src/pages/account-pool/UpstreamAccounts.test.tsx src/pages/account-pool/UpstreamAccountCreate.test.tsx`
- `cd web && bun run build`
- `cd web && bun run build-storybook`

## 文档更新（Docs to Update）

- `docs/specs/README.md`

## Plan assets

- Directory: `docs/specs/3np57-group-bound-proxy-real-traffic-stats/assets/`

## Visual Evidence

- source_type: storybook_canvas
  story_id_or_title: Account Pool/Components/Upstream Account Group Settings Dialog/Group Scoped Real Traffic
  state: group-scoped real traffic
  evidence_note: 已使用稳定 Storybook 场景完成视觉验收；弹窗 24H 节点统计已切到当前组真实请求，`Direct` 与手动节点分别显示真实计数，penalized 节点在无组内流量时保持 0。本次按主人要求不提交截图文件。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 为 pool attempt live/archive schema 增加分组与 canonical binding key 快照
- [x] M2: 分组 binding nodes API 切换到真实 pool 请求 24h 桶
- [x] M3: 前端两个组设置入口透传 `groupName`
- [x] M4: 补齐 Rust / 前端回归测试
- [x] M5: Storybook 视觉证据、review-loop 与 PR-ready 收口

## 风险 / 假设

- 假设：`PoolResolvedAccount.group_name` 与实际组归属一致，可作为快照真相源。
- 假设：`selected_proxy.key` 能通过 forward-proxy manager canonicalize 成稳定 binding key；`Direct` 固定为 `__direct__`。
- 风险：组名在 24 小时内被重命名时，旧名称流量不会迁移；这是有意保持“快照即历史真相”的结果。
