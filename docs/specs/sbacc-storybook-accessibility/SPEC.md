# Storybook 可访问性门禁收敛

## 状态

- Status: 已完成
- Created: 2026-03-13
- Last: 2026-03-13

## 背景 / 问题陈述

- `web/.storybook/main.ts` 已启用 `@storybook/addon-a11y`，但 `web/.storybook/preview.ts` 仍将 `parameters.a11y.test` 设为 `todo`，导致违规只在 Storybook 面板提示，不会阻断 CI。
- 仓库缺少可重复执行的 Storybook a11y CLI/CI 链路，现有 `Front-end Tests` 与 `Records Overlay E2E` 也不会覆盖 Storybook stories 的 axe 结果。
- 首轮接入可阻断测试后，已暴露真实问题：`Records` tab 语义混用、`Settings` 标题层级跳级、计费表格输入缺少可访问名称、部分组合框与 loading spinner 的 ARIA 语义不完整。

## 目标 / 非目标

### Goals

- 为 `web/` 建立可本地执行、可 CI 阻断的 Storybook accessibility gate。
- 保持现有 Storybook 端口治理方式，统一通过仓库脚本启动 Storybook，而不是直接依赖默认 `6006`。
- 修复首轮被门禁打出的真实可访问性问题，并将短期无法一次性收敛的项约束到 story/meta 级例外。
- 将新 job 纳入主 CI 收敛链路与 quality-gates 声明，保证快车道收敛时可以明确追踪通过状态。

### Non-goals

- 不改动 Rust 后端、HTTP/SSE 接口、数据库结构或 Docker 运行时行为。
- 不引入 Chromatic 或外部 SaaS。
- 不把普通 Vitest 单测或 Playwright E2E 全量改写为 Storybook 测试。
- 不通过全局关闭 a11y 规则来换取 CI 通过。

## 范围（Scope）

### In scope

- `web/package.json`
- `web/.storybook/main.ts`
- `web/.storybook/preview.ts`
- `web/.storybook/vitest.setup.ts`
- `web/vitest.config.ts`
- `web/scripts/run-storybook.mjs`
- `web/src/pages/Records.tsx`
- `web/src/pages/Settings.tsx`
- `web/src/components/**/*stories.tsx` 中被 Storybook a11y 收敛波及的 stories
- `.github/workflows/ci.yml`
- `.github/quality-gates.json`
- `.github/scripts/fixtures/quality-gates-contract/*`
- `docs/specs/README.md`

### Out of scope

- Storybook 视觉回归基建。
- 非 Storybook 的前端测试架构重写。
- 全站主题系统重构。

## 需求（Requirements）

### MUST

- `web/.storybook/preview.ts` 默认使用 `parameters.a11y.test = 'error'`。
- `cd web && bun run test-storybook` 可在未手动预启 Storybook 的情况下直接执行并失败于真实 axe 违规。
- 新增 CI job `Storybook Accessibility`，执行 `cd web && bun run test-storybook`。
- `release-meta` 收敛链路依赖 `storybook-accessibility` job。
- Storybook 例外仅允许缩到 story/meta 级，并在 story 附近或 spec 中说明原因。

### SHOULD

- 复用仓库现有 `bun run storybook` 端口脚本，并新增适用于 CI/headless 的 `bun run storybook:ci`。
- 将 Storybook 测试与普通单元测试拆分为独立 Vitest projects，避免手动先启动 Storybook 或混跑配置。
- 对高风险 story 保持基线覆盖：表格类、设置/表单类、交互型页面类。

## 接口契约（Interfaces & Contracts）

- 新增脚本：
  - `cd web && bun run storybook:ci`
  - `cd web && bun run test-storybook`
- 现有 `cd web && bun run test` 切换为只执行 `unit` project。
- 新增 CI job：`Storybook Accessibility`
- `quality-gates.json` 将 `Storybook Accessibility` 声明为 informational check，并要求其出现在 `CI Pipeline` workflow job 集合中。

## 验收标准（Acceptance Criteria）

- Given 任意 Storybook story 触发 axe 违规，When 在本地运行 `cd web && bun run test-storybook`，Then 命令失败并指向具体 story。
- Given PR 或主线 CI 运行，When `Storybook Accessibility` job 执行，Then job 会安装 Chromium 并跑完 Storybook a11y 测试。
- Given `RecordsPage` stories，When 运行 Storybook a11y，Then tab 组件不再出现 `aria-pressed` / `role=tab` 语义冲突。
- Given `SettingsPage` stories，When 运行 Storybook a11y，Then 标题层级连续，计费输入拥有明确可访问名称。
- Given 当前浅色主题 stories，When 短期仍存在历史配色对比度债务，Then 仅允许在 story/meta 级关闭 `color-contrast`，其它规则继续生效。

## 非功能性验收 / 质量门槛（Quality Gates）

### Quality checks

- `cd /Users/ivan/.codex/worktrees/a71e/codex-vibe-monitor/web && bun run test`
- `cd /Users/ivan/.codex/worktrees/a71e/codex-vibe-monitor/web && bun run build`
- `cd /Users/ivan/.codex/worktrees/a71e/codex-vibe-monitor/web && bun run build-storybook`
- `cd /Users/ivan/.codex/worktrees/a71e/codex-vibe-monitor/web && bun run test-storybook`
- `python3 .github/scripts/check_quality_gates_contract.py --repo-root "$PWD" --declaration ".github/quality-gates.json" --metadata-script ".github/scripts/metadata_gate.py" --profile bootstrap`
- `bash .github/scripts/test-quality-gates-contract.sh`
- `bash .github/scripts/test-live-quality-gates.sh`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 切出 `th/storybook-accessibility` 并建立 spec。
- [x] M2: 接入 `@storybook/addon-vitest`、浏览器运行依赖、`test-storybook` 与 `storybook:ci` 脚本。
- [x] M3: 将 Storybook 默认 a11y 模式从 `todo` 升级到 `error`。
- [x] M4: 修复首轮真实 a11y 失败（Records / Settings / Spinner / Combobox）。
- [x] M5: 新增 `Storybook Accessibility` CI job，并同步 quality-gates 声明。
- [x] M6: 完成全量验证、提交、推送、PR、checks、review-loop 收敛。

## 进度备注

- Storybook Vitest 方案采用 `storybookTest({ configDir, storybookScript }) + Vitest projects`，其中 `storybookScript` 固定为 `bun run storybook:ci`，避免裸跑默认端口。
- `RecordsPage` 已移除 `role="tab"` 按钮上的 `aria-pressed`。
- `SettingsPage` 已把卡片主标题收敛为 `h2`，并为 pricing table 输入补充 `aria-label`；forward proxy 列表标题收敛为 `h3`。
- `Spinner` 在携带 `aria-label` / `aria-labelledby` 时自动回填 `role="status"`，避免 `aria-prohibited-attr`。
- `UpstreamAccountGroupCombobox` 在无外部 `aria-label` 时会回退到当前值 / placeholder / 默认名称，避免 button-name 缺失。
- 已完成本地验证、推送与 PR 收敛：PR `#123` 已创建，`Storybook Accessibility` 与既有 checks 均为 green，当前无额外 review comments 阻塞。

## 临时例外策略（Scoped Exceptions）

- 当前仅对 Storybook 的浅色主题 `color-contrast` 历史债务做 story/meta 级例外，统一通过 `web/src/storybook/a11y.ts` 复用。
- 已挂载该例外的 stories：
  - `web/src/components/InvocationRecordsSummaryCards.stories.tsx`
  - `web/src/components/InvocationRecordsTable.stories.tsx`
  - `web/src/components/InvocationTable.stories.tsx`
  - `web/src/components/PromptCacheConversationTable.stories.tsx`
  - `web/src/components/RecordsPage.stories.tsx`
  - `web/src/components/SettingsPage.stories.tsx`
  - `web/src/components/TodayStatsOverview.stories.tsx`
  - `web/src/components/UpstreamAccountGroupCombobox.stories.tsx`
  - `web/src/components/UpstreamAccountUsageCard.stories.tsx`
  - `web/src/components/UpstreamAccountsPage.stories.tsx`
  - `web/src/components/UpstreamAccountsTable.stories.tsx`
  - `web/src/components/ui/info-tooltip.stories.tsx`
- 清理条件：当浅色主题设计 tokens 调整后，逐个移除 story/meta 例外并保持 `bun run test-storybook` 继续通过。

## 风险 / 假设（Risks / Assumptions）

- 假设：Storybook 10 + Vitest 4 + Playwright Chromium 组合在 GitHub Actions Ubuntu runner 上可稳定执行。
- 风险：浅色主题的颜色 token 仍有历史对比度债务，短期内可能继续影响部分 stories；当前已通过最小范围例外避免扩大豁免面。
- 风险：`storybook dev --ci` 仍会常驻进程，必须通过 Vitest 插件托管其生命周期，不能在 CI 中手动后台残留。

## 参考（References）

- Storybook Accessibility tests
- Storybook Vitest addon
- Storybook Testing in CI
