# 号池分层同步高级设置与前 100 溢出低频更新 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/jpvwj-account-pool-tiered-maintenance/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-23: 创建 spec，冻结号池分层维护配置、队列排序和 UI 入口范围。
- 2026-03-23: 已完成 `pool_routing_settings` 维护字段持久化、部分更新 API、批量 tier resolver、固定短 tick 维护调度，以及 routing UI/文案/Storybook 同步。
- 2026-03-23: 本地验证已通过 `cargo check`、3 个定向 Rust 测试、`cd web && bun x vitest run src/lib/api.test.ts src/hooks/useUpstreamAccounts.test.tsx src/pages/account-pool/UpstreamAccounts.test.tsx`、`cd web && bun run build`。
- 2026-03-23: 已补上 review-loop 修复：`refresh-due` 账号继续遵守主频节奏，queued maintenance 在执行前会重验计划是否仍然到期；新增 Rust 回归覆盖这两类场景。
- 2026-03-23: PR #211 当前已对齐最新 `origin/main`，GitHub PR checks 全绿、`mergeable_state=clean`；fresh `codex review --base origin/main` 未再产出新的代码 finding，期间暴露的一次前端测试超时已本地复跑 `cd web && bun x vitest run src/lib/api.test.ts src/pages/account-pool/UpstreamAccounts.test.tsx` 通过。
- 2026-03-24: 回滚 routing 卡片里误加的 3 项 maintenance 与 4 项 timeout 只读摘要 tiles，恢复为仅显示当前号池 API Key 与编辑入口；同步补上列表页 Storybook 回归与 summary-only 视觉证据。
- 2026-04-04: maintenance 策略扩展为“高频 + 原主层 / 次层 + reset 跨点补同步”；`working` / `degraded` 账号固定 `60s` 高频，主 / 次窗口跨过 `resetsAt` 后会在下个 tick 尽快补同步，并补齐对应 Rust 回归与 UI 文案说明。
