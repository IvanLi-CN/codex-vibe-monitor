# 上游账号列表分页、跨页选择与批量操作 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/enzf8-upstream-account-roster-pagination-bulk-actions/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-22: 创建 spec，冻结分页、展示状态、跨页选择、批量操作与批量同步的范围和契约。
- 2026-03-22: 完成后端列表分页、`displayStatus` 分类器、批量 mutation、批量同步 job/SSE，以及前端跨页选择、分页 footer、状态筛选和批量交互 UI。
- 2026-03-22: 本地验证通过 `cargo fmt --check`、`cargo check`、`cargo test upstream_accounts -- --nocapture`、`cd web && bun run build` 与定向 Vitest 回归，按 fast-track 收口到 merge-ready。
- 2026-03-26: 修正批量同步终态事件仍携带 `running snapshot.status` 的回归问题，前端改为按事件类型强制收敛终态、立即解锁批量工具条，并区分“全成功自动收起”与“非成功终态手动收起”。
- 2026-03-26: 补齐后端终态 SSE / snapshot 一致性测试、前端 EventSource 回归测试与 Storybook 终态场景；本地验证通过 `cargo test finish_bulk_sync_job_ -- --nocapture`、`cargo test bulk_upstream_account_sync_job -- --nocapture`、`cd web && bun run test -- src/pages/account-pool/UpstreamAccounts.test.tsx` 与 `cd web && bun run build-storybook`。
- 2026-03-26: 已生成并获主人确认的批量同步终态失败 mock-only Storybook 视觉证据，证据路径收敛为稳定资产文件并进入 spec `## Visual Evidence`。
- 2026-03-26: 根据最新交互反馈，将批量同步终态面板改为右下角悬浮气泡展示，避免占用账号列表正文文档流，并进一步把“收起”动作改成图标式按钮，补充对应的 Storybook / Vitest 断言。
- 2026-03-31: 为 `useUpstreamAccounts` 引入 query-key freshness / loading 状态机；筛选、分页、`pageSize`、手动 refresh 与外部 `upstream-accounts:changed` 全部绑定到当前 query key，旧 query 的 success / error 不再回写当前列表。
- 2026-03-31: 号池列表在 query 切换后新增 `600ms` stale grace；超时后列表区、分页摘要与页码按钮统一切到 blocking loading，当前 query 失败时改为 inline error + retry，不再保留上一 query 的旧表格内容。
- 2026-03-31: 补齐 Storybook 慢筛选切换 / 慢分页切换 / 当前 query 失败三组稳定场景，以及 `useUpstreamAccounts` / `UpstreamAccountsPage` / `UpstreamAccountsTable` 定向回归；本地验证通过 `cd web && bun x vitest run src/hooks/useUpstreamAccounts.test.tsx src/pages/account-pool/UpstreamAccounts.test.tsx src/components/UpstreamAccountsTable.test.tsx`、`cd web && bun run build` 与 `cd web && bun run build-storybook`。
- 2026-04-01: 根据最新交互反馈，阻塞 loading 期间冻结列表区上一稳定高度，待当前 query 加载完成后再恢复自适应内容高度，并补齐对应 Vitest 断言与慢筛选视觉证据更新。
- 2026-04-02: 根据最新长列表交互反馈，阻塞 loading 改为“冻结旧 rows + 置灰遮罩 + 表格内部居中半透明磨砂 loading 卡片 + 页脚分页控件原位禁用”组合反馈，避免长列表切页时视口空白或滚动跳动；同步更新 spec 契约与三张 Storybook 视觉证据。
