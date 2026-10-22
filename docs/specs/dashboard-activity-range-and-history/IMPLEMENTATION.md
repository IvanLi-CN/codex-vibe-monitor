# 活动总览时间范围与历史活动归位 实现状态

> 当前需求以 `./SPEC.md` 为准；本文区分已有实现事实与尚未实现的需求。

## Current Status

- Implementation: 已完成应用实现，候选变更进入交付验证。
- Lifecycle: implementation-complete
- Catalog note: 全局与账号范围同步、三档七卡、独立最近时间线、统计页 180 天日历。

## Implementation Coverage

- `REQ-DAH-001`, `REQ-DAH-002`, `REQ-DAH-004`, `REQ-DAH-006`: 已由 `DashboardActivityOverview.tsx`、`TodayStatsOverview.tsx`、`dashboardActivityRange.ts`、`dashboardKpiComparisons.ts` 和快照请求统一为三档范围、七卡、前日/前窗比较及范围跟随图表。
- `REQ-DAH-003`: 已接入昨日活跃分钟均值、完整活动分钟并行均值和真实 TTFT 样本加权均值；实时范围保留现有实时采样主值。
- `REQ-DAH-005`: 时间线固定使用独立最近 24 小时可查询边界和默认 30 分钟视窗；昨日仍接收实时调用，账号切换沿用既有清理和分页状态。
- `REQ-DAH-007`: `UsageCalendar.tsx` 使用 `recent180d` 和本地今日向前 179 日边界，已移动到 `Stats.tsx` 顶部并独立提供次数、金额、Token 指标。
- `REQ-DAH-008`: 组件保留加载、错误、缺样本和已验证零值的区分，并完成中英文及移动布局的 demo 视觉检查。
- HTTP 合同：summary、timeseries、parallel-work 支持成对 `from`/`to` UTC 边界，互斥旧 window/range，最长 48 小时；时间线兼容旧自然日参数并接受 `scopeStart`/`scopeEnd` 别名。
- 账号详情直接复用 `DashboardActivityOverview`，入口位于 `web/src/pages/account-pool/UpstreamAccounts.page-local-shared.tsx`，因此与全局表面同步。

## Verification

- 文档验证：`python3 /Users/ivan/.codex/bin/spec_contract_check.py --path docs/specs/dashboard-activity-range-and-history/SPEC.md` 通过，`git diff --check` 通过。
- Web 验证：`bun run test` 为 179 个文件、1918 个通过、6 个跳过；`bun run test-storybook` 为 34 个通过、2 个既有 account-pool overlay 失败、38 个跳过、173 个测试通过；`bun run typecheck:web`、`bun run build` 通过。全仓 `bun run lint:web` 完成但报告 93 个 warning，其中包含未触碰的 `UpstreamAccountsGroupedRoster.tsx` exhaustive-deps 诊断及改动文件既有 warning。
- Rust 验证：`cargo fmt --all -- --check`、`cargo check --locked --all-targets --all-features`、`cargo clippy --locked --all-targets --all-features -- -D warnings` 通过；定向 `cargo test` 被本机已有的 macOS `sqlx_macros` 动态库 `mis-aligned LINKEDIT string pool` 阻塞，后端资源 profile 尚未执行。首次 Agent VM 准入为 `resource_unverified`，重新准入完成资源分配但在 `guest_readiness_timeout` 终止，当前无有效 lease。
- 视觉验证：ui_demo 已捕获总览桌面/393×852 移动及统计页桌面快照；总览三档范围、独立时间线、统计页顶部 180 日历均可见。

视觉快照：`/Users/ivan/.codex/user-inline-assets/codex-vibe-monitor__c00ce200/2026/10/22/20261022T112550Z-cvm-dashboard-current-721a3330.png`、`/Users/ivan/.codex/user-inline-assets/codex-vibe-monitor__c00ce200/2026/10/22/20261022T112636Z-cvm-dashboard-mobile-current-050edb05.png`、`/Users/ivan/.codex/user-inline-assets/codex-vibe-monitor__c00ce200/2026/10/22/20261022T112551Z-cvm-stats-current-48642f99.png`。

## Remaining Gaps

- Agent VM 恢复后仍需依次执行 backend `lightweight`、`stateful-sqlite`、`archive-file-io` profiles；在此之前不宣称后端资源 profile 完整通过。
- Tier 3 四路只读审查、候选提交后的 current-head CI 和 PR publication 仍属于交付阶段。

## References

- `./SPEC.md`
- `./HISTORY.md`
