# Dashboard TPM 整数显示热修（#8shg4)

## 状态

- Status: 已完成
- Created: 2026-04-11
- Last: 2026-04-11

## 背景 / 问题陈述

- `TPM (5m avg)` 已替换进入今日 KPI，但当前仍沿用通用 number 格式，窗口均值出现小数时会直接显示小数点。
- 主人明确要求 TPM 不要出现小数点；继续显示小数会让该 KPI 看起来像成本或比率，而不是吞吐量。

## 目标 / 非目标

### Goals

- 让 Dashboard 今日 KPI 的 `TPM (5m avg)` 始终以整数显示。
- 保持 `Cost/min (5m avg)`、总成本和其它非 TPM 指标的现有精度。
- 通过现有 Storybook / Vitest 覆盖锁定“TPM 无小数、Cost/min 仍保留货币格式”。

### Non-goals

- 不修改 5m 均值算法、窗口口径或后端接口。
- 不改变 `Cost/min`、总成本或 Tokens 总量的显示规则。

## 范围（Scope）

### In scope

- `web/src/components/AdaptiveMetricValue.tsx`：增加整数显示模式。
- `web/src/components/TodayStatsOverview.tsx`：把 TPM tile 切到整数显示。
- `web/src/components/TodayStatsOverview.test.tsx`、`web/src/components/TodayStatsOverview.stories.tsx`：补齐/更新小数 TPM 的 UI 覆盖。

### Out of scope

- `dashboardTodayRateSnapshot` 计算逻辑。
- Rust 后端与 `/api/stats/*` 契约。

## 需求（Requirements）

### MUST

- TPM 在任何情况下都不得显示小数点。
- TPM 的整数显示只影响该 tile，不影响 `Cost/min` 的货币格式。

### SHOULD

- 继续复用 `AdaptiveMetricValue` 的溢出压缩能力，不引入新的独立文本组件。

### COULD

- Storybook 示例可直接使用 fractional TPM，方便后续回归肉眼验证。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 当 `TodayStatsOverview` 收到 fractional `tokensPerMinute`（例如 `1000.6`）时，TPM tile 显示四舍五入后的整数。
- `Cost/min` 仍按货币规则显示，例如 `$0.10`。

### Edge cases / errors

- rate unavailable / loading 的 `—` 与 skeleton 行为保持不变。
- compact overflow fallback 仍可对 TPM 生效，但 compact 文本本身也不得带小数。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- | --- | --- |
| TodayStatsOverview TPM display format | ui-component-prop | internal | Modify | None | web/dashboard | Dashboard today KPI | 仅 UI 格式变更，不改数据口径 |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given `tokensPerMinute=1000.6`，When 渲染今日 KPI，Then TPM 显示 `1,001` 而不是 `1000.6`。
- Given fractional TPM 与 fractional cost/min 同时存在，When 渲染今日 KPI，Then `Cost/min` 仍保留 `$0.10` 这类货币格式。
- Given desktop single row Storybook，When 查看 TPM tile，Then 不出现小数点。

## 实现前置条件（Definition of Ready / Preconditions）

- 保持本次变更仅限前端格式层，不触及窗口算法与后端契约。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `TodayStatsOverview.test.tsx` 覆盖 fractional TPM 整数显示。
- Integration tests: `DashboardActivityOverview.test.tsx` 保持 today rate flow 回归不破。

### UI / Storybook (if applicable)

- Stories to add/update: `TodayStatsOverview.stories.tsx`
- Docs pages / state galleries to add/update: 复用现有 today KPI story
- `play` / interaction coverage to add/update: None
- Visual regression baseline changes (if any): 本次视觉证据使用本地 Storybook 渲染，不向 PR 提交截图资产。

### Quality checks

- `cd web && bun run test -- src/components/TodayStatsOverview.test.tsx src/components/DashboardActivityOverview.test.tsx`
- `cd web && bun run build`
- `cd web && bun run build-storybook`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增 follow-up spec 索引并同步状态。
- `docs/specs/8shg4-dashboard-tpm-whole-number-hotfix/SPEC.md`: 记录本次格式热修的范围、验收与验证。

## 计划资产（Plan assets）

- Directory: `docs/specs/8shg4-dashboard-tpm-whole-number-hotfix/assets/`
- In-plan references: `![...](./assets/<file>.png)`
- Visual evidence source: local Storybook render shown in chat only for this hotfix.

## Visual Evidence

- local-only evidence will be captured from Storybook and shown in chat before merge; no screenshot assets are promoted into this spec for this hotfix.

## 资产晋升（Asset promotion）

- None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: TPM tile 改为整数显示。
- [x] M2: Vitest / Storybook 覆盖 fractional TPM。
- [x] M3: fast-flow PR 合并并完成收尾。

## 方案概述（Approach, high-level）

- 在 `AdaptiveMetricValue` 中补一个 integer 模式，让 TPM 复用现有布局、自适应压缩和动画逻辑，而不是引入分叉组件。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：若 integer 模式误伤其它 number 场景，会让本应保留小数的数值被截断；因此只在 TPM tile 上启用。
- 假设：TPM 使用标准四舍五入即可满足“不显示小数点”的要求。

## 变更记录（Change log）

- 2026-04-11: 新建 hotfix spec，冻结“TPM 不显示小数点，仅影响今日 KPI TPM tile”的范围。
- 2026-04-11: 完成整数格式实现，local `vitest + build + build-storybook` 通过，Storybook 本地证据已在聊天回传。

## 参考（References）

- `docs/specs/2qsev-dashboard-tpm-cost-per-minute-kpi/SPEC.md`
