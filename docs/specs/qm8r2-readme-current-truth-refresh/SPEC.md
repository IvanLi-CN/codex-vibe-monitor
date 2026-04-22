# README 当前真相重写与展示刷新（#qm8r2）

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-23
- Last: 2026-04-23

## 背景 / 问题陈述

- 根 `README.md` 已经落后于当前项目真实能力，仍残留旧的特性口径、环境变量堆砌与过期截图，不能正确回答“项目现在是什么、能做什么、如何开始”。
- 当前仓库已经形成更完整的产品面与文档面：Dashboard / Live / Stats / Records / Settings / Account Pool、OAuth inline adapter、docs-site、Storybook、SQLite retention / archive，但 README 没有把这些 current truth 收敛成一个面向协作者和自部署用户的入口。
- README 展示图也缺少与当前 UI 相匹配的稳定截图，无法承担 GitHub 仓库首页的产品级展示职责。

## 目标 / 非目标

### Goals

- 基于当前代码、前端路由、后端 API、docs-site 与 Storybook，重写根 `README.md`，让它重新成为仓库的人类入口文档。
- 用稳定 Storybook mock 场景生成一组 README 展示图，并替换旧截图来源。
- 让 README 明确回答：项目定位、核心能力、页面地图、最短启动路径、关键配置入口、文档入口、技术栈与常用命令。

### Non-goals

- 不重写 `docs-site/docs/**` 的信息架构，只复用它们作为 README 的事实来源。
- 不改变后端 API、前端产品逻辑或部署语义。
- 不把 README 写成完整配置手册；更深的部署与运行细节继续留在 `docs-site/` 与仓库内部文档。

## 范围（Scope）

### In scope

- 根 `README.md` 的整页重写
- README 展示图的生成、落盘与引用更新
- `web/src/components/DashboardPage.stories.tsx` 的 README 专用 dense mock 场景补充
- `docs/specs/README.md` 与当前 spec 的状态同步

### Out of scope

- `docs-site/docs/**` 的正文重写
- 生产页面信息架构调整
- 新增后端功能或前端业务交互

## 需求（Requirements）

### MUST

- README 必须以当前代码真相源为依据，覆盖当前存在的页面与能力边界。
- README 必须替换旧截图，改用稳定 Storybook mock 生成的展示图。
- Dashboard 主展示图必须使用更真实的 dense mock，并包含 12 个工作中对话，避免首页观感过空。
- README 必须提供清晰的快速开始入口，区分镜像启动与本地开发路径。
- README 必须把更深入的配置 / 部署 / 开发内容导向 `docs-site/` 与仓库内部文档，而不是在首页堆满长环境变量列表。

### SHOULD

- README 语言优先面向“第一次认识项目的人”，而不是只面向已经熟悉仓库结构的维护者。
- README 的页面地图、能力地图与截图顺序应相互呼应。
- README 里的命令应与当前仓库脚本真实存在的入口保持一致。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 打开仓库首页时，读者先看到项目定位与当前 UI 展示，再看到能力地图与最短启动路径，而不是旧的环境变量清单或失真的历史口径。
- README 通过 4 张稳定展示图说明当前最核心的产品面：Dashboard、Live、Records、Account Pool。
- README 把深入文档分流到 `docs-site/docs/index.md`、`product.md`、`quick-start.md`、`config.md`、`deployment.md`、`development.md` 与 `storybook.mdx`。
- README 的“页面地图”与当前前端路由一致，能与 `web/src/App.tsx` 对上。

### Edge cases / errors

- README 不得继续暗示项目仍以历史 XYAI 抓取为主要入口。
- README 不得声称当前并不存在的页面、命令或端口约定。
- README 展示图不得依赖真实私有数据页面，而应来自稳定 mock / Storybook 入口。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `README.md` | doc | public | Modify | None | repo root docs | GitHub 仓库访客 / 协作者 | 改为 current truth 入口 |
| `docs/readme-assets/final/*` | asset | public | New | None | README assets | README / 仓库首页 | 稳定展示图资产 |
| `DashboardPage.stories.tsx#ReadmeDense` | storybook story | internal | New | None | web Storybook | README 截图生成 / UI 评审 | 12 条 working conversations |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given 打开根 `README.md`，When 读者浏览首页，Then 能快速理解项目定位、核心能力、关键页面与启动路径，而不再被旧口径误导。
- Given README 的“页面地图”，When 对照 `web/src/App.tsx`，Then 页面列表与路径一致。
- Given README 的“核心能力”说明，When 对照后端路由与 docs-site，Then 不出现已经过时或不存在的产品面描述。
- Given README 的展示图，When 查看仓库首页，Then 能看到 Dashboard / Live / Records / Account Pool 四个当前主要界面，其中 Dashboard 图包含 12 个工作中对话。
- Given `web/src/components/DashboardPage.stories.tsx` 的 `ReadmeDense` story，When 渲染 Storybook canvas，Then 能稳定产出 README 所需的 dense Dashboard 截图。

## 实现前置条件（Definition of Ready / Preconditions）

- 当前项目页面与路由已由 `web/src/App.tsx` 明确存在。
- docs-site 已提供相对新鲜的 public docs，可作为 README 的事实来源补充。
- Storybook 已具备稳定页面级截图能力，可承载 README 展示图来源。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cd /Users/ivan/Projects/Ivan/codex-vibe-monitor/web && bun run lint`
- `cd /Users/ivan/Projects/Ivan/codex-vibe-monitor/web && bun run build`
- `cd /Users/ivan/Projects/Ivan/codex-vibe-monitor/web && bun run build-storybook`

### UI / Storybook (if applicable)

- Stories to add/update: `web/src/components/DashboardPage.stories.tsx`
- Visual evidence source: Storybook canvas screenshots stored under `docs/readme-assets/final/`

### Quality checks

- README 命令与路径必须与当前 `package.json` / `web/package.json` / `docs-site/package.json` 对齐。
- README 的页面与接口口径必须与 `web/src/App.tsx`、`src/maintenance/hourly_rollups.rs`、`src/oauth_bridge.rs` 对齐。

## 文档更新（Docs to Update）

- `README.md`: 以 current truth 重写仓库首页入口
- `docs/specs/README.md`: 新增本 spec 索引并同步当前状态
- `docs/specs/qm8r2-readme-current-truth-refresh/SPEC.md`: 记录本轮 README 重写范围、验收与视觉证据

## 计划资产（Plan assets）

- Directory: `docs/readme-assets/final/`
- In-plan references:
  - `dashboard-overview-1680-readme-dense.png`
  - `live-monitoring-1680.png`
  - `records-analysis-1680.png`
  - `account-pool-grouped-1680.png`
- Visual evidence source: Storybook canvas

## Visual Evidence

- Dashboard dense README 截图，基于 Storybook canvas `pages-dashboardpage--readme-dense`，`1680px` 宽视口，包含 12 条工作中对话：

  ![Dashboard dense README](../../readme-assets/final/dashboard-overview-1680-readme-dense.png)

- Live 页面 README 截图：

  ![Live README](../../readme-assets/final/live-monitoring-1680.png)

- Records 页面 README 截图：

  ![Records README](../../readme-assets/final/records-analysis-1680.png)

- Account Pool 页面 README 截图：

  ![Account Pool README](../../readme-assets/final/account-pool-grouped-1680.png)

## 资产晋升（Asset promotion）

- README 展示图作为稳定项目文档资产，固定保留在 `docs/readme-assets/final/`，由根 `README.md` 引用。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 基于当前代码与文档真相源盘点 README 需要覆盖的 current truth。
- [x] M2: 为 README 生成稳定 Storybook 展示图，并补齐 Dashboard dense mock story。
- [x] M3: 重写根 `README.md`，重组为项目定位、展示图、核心能力、页面地图、快速开始与文档入口。
- [x] M4: 校对 README 命令、路径、页面与截图引用，推进到 fast-track PR 收敛。

## 方案概述（Approach, high-level）

- 先用当前代码、路由与 docs-site 内容建立 README 真相源，再用 Storybook 生成稳定截图，最后把 README 重写成面向人类阅读的 current truth 入口。
- 避免把 README 当成完整配置手册，而是让它承担“快速理解项目 + 快速开始 + 正确分流到更深文档”的职责。
- 用 README 专用 dense mock 提升首页展示密度，让 Dashboard 截图更像真实运行中的工作台。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：如果未来页面或 API 能力再发生较大变化，README 仍可能再次过时；因此本轮同时把 README 口径锚定到 docs-site 与代码真相源。
- 风险：README 展示图若后续继续沿用旧 mock，会重新出现“界面过空”的问题；当前通过 `ReadmeDense` story 固化了一条更适合仓库首页的截图来源。
- 假设：当前 docs-site 与前端路由已经足够反映项目的稳定产品面，不需要在本轮新增 public docs 页面。

## 变更记录（Change log）

- 2026-04-23: 新建 spec，冻结 README current truth 重写范围、展示图来源与验收口径。
- 2026-04-23: 为 Dashboard 补齐 `ReadmeDense` Storybook 场景，使用 12 条工作中对话重做 README 主展示图。
- 2026-04-23: 重写根 `README.md`，把仓库首页从旧口径与长配置列表改为当前产品入口、能力地图、页面地图与快速开始。
- 2026-04-23: 本地收口阶段修正 README dense Storybook mock 的类型漂移，并重新通过 `web` 的 `lint`（existing warnings only）、`build` 与 `build-storybook`，同时刷新当前 head 的 README 展示图资产。

## 参考（References）

- `README.md`
- `web/src/App.tsx`
- `src/maintenance/hourly_rollups.rs`
- `src/oauth_bridge.rs`
- `docs-site/docs/index.md`
- `docs-site/docs/product.md`
- `docs-site/docs/quick-start.md`
- `web/src/components/DashboardPage.stories.tsx`
