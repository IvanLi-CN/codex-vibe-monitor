# 文档站与 Storybook GitHub Pages 同构发布（#j9frr）

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-19
- Last: 2026-03-19

## 背景 / 问题陈述

- 当前仓库已经有根 README、`docs/ui/**` 规范文档、`docs/deployment.md` 与一组可运行的 Storybook stories，但缺少一个面向协作者和评审的公开文档壳层。
- Storybook 目前只作为本地开发与视觉证据入口存在，没有与公共文档站统一发布，也没有 GitHub Pages 装配链路。
- 现有 CI 会校验前后端与镜像产物，但不会在 PR 阶段对 docs base-path、`/storybook/` 子路径或文档装配结果做 smoke。

## 目标 / 非目标

### Goals

- 为仓库新增独立的 `docs-site/` 文档站，承载公开的首页、项目介绍、快速开始、配置与运行、自部署、排障、开发与 Storybook 入口。
- 保留 `web/` 下的 Storybook 作为页面/组件证据面，并以 `/storybook/` 子路径并入 GitHub Pages 站点。
- 为 docs-site、Storybook 与装配产物补齐本地开发约定、CI smoke 与独立 `Docs Pages` 发布工作流。
- 保持现有 required checks 拓扑不扩张，新增文档站验证仍通过既有 `Lint & Format Check` 主链完成。

### Non-goals

- 不把 `docs/specs/**`、`docs/plan/**` 或全部内部运维文档完整迁入 public docs 导航。
- 不重构现有前端页面、Storybook 主题系统或故事分层。
- 不改写 release 语义、branch protection 或 quality-gates required check 名称。

## 范围（Scope）

### In scope

- 新建 `docs-site/`，包含 `package.json`、`rspress.config.ts`、`bun.lock` 与 `docs/index.md`、`quick-start.md`、`config.md`、`deployment.md`、`troubleshooting.md`、`development.md`、`product.md`、`storybook.mdx`、`404.md`
- 更新 `web/package.json` 与 `web/scripts/run-storybook.mjs`，固定 Storybook 默认开发端口并引入 `storybook:build`
- 新建 `.github/scripts/assemble-pages-site.sh`
- 新建 `.github/workflows/docs-pages.yml`
- 更新 `.github/workflows/ci-pr.yml` 与 `.github/workflows/ci-main.yml`，把 docs-site build、Storybook static build 与 assembled-site smoke 纳入现有 lint 主链
- 更新 `README.md`、`web/README.md`、`docs/ui/README.md`、`docs/ui/storybook.md`、`docs/specs/README.md`

### Out of scope

- 新增 Storybook story 文件或大规模补写 story prose
- 修改 `src/**` 的运行时逻辑与 API 契约
- 为 PR 正文引入截图资产或视觉证据图片

## 需求（Requirements）

### MUST

- docs-site 必须使用 Rspress，并支持 `DOCS_BASE` 子路径部署
- docs-site 首页导航必须覆盖 Home、Product、Quick Start、Self-Hosting、Development、Storybook、GitHub
- `storybook.html` 必须在本地 docs-site 场景跳转到 Storybook dev server，在装配后的静态站点跳转到 `./storybook/index.html`
- Storybook 默认开发端口必须固定为 `60082`，docs-site 默认开发端口必须固定为 `60081`，并允许通过 env 覆盖
- `web/storybook-static/` 必须继续作为 Storybook 静态产物目录
- `assemble-pages-site.sh` 必须把 docs-site 构建产物放到站点根目录，把 Storybook 放到 `/storybook/`，并对关键入口文件做失败即中断的 smoke 断言
- `ci-pr.yml` 与 `ci-main.yml` 不得新增 job 名称，docs-site / Storybook / assembled-site smoke 只能接入现有 `Lint & Format Check`

### SHOULD

- public docs 应保留单一 Storybook 入口，而不是额外维护独立导览页，避免 docs 与 stories 深链重复维护
- `web/README.md` 与根 README 应给出稳定的本地 URL 合同与 Pages 说明
- `docs/ui/**` 应明确“public docs 在 `docs-site/`，内部 UI 规范继续在 `docs/ui/`”

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 协作者访问 GitHub Pages 根路径时，先看到 docs-site 首页与公共导航，而不是裸 Storybook 或仓库 README。
- docs-site 的 public IA 必须优先服务“自部署用户”，其次服务“项目开发者”，并补充独立排障入口让首次部署后的常见问题可快速收口。
- 协作者点击 Storybook 入口时：
  - 若当前是 docs-site 本地开发服务器（localhost/127.0.0.1 且当前页面是 `storybook.html`），跳转到本地 Storybook dev origin。
  - 其他场景跳转到同站点下的 `./storybook/index.html`。
- 维护者在 PR 中修改 `docs-site/**`、`web/**` 或装配脚本时，现有 `Lint & Format Check` 会同时验证 docs-site build、storybook build 与 assembled-site smoke。
- `Docs Pages` workflow 在 PR 上生成 docs-site、storybook-static 与 assembled pages artifacts；在 `main` 上继续部署到 GitHub Pages。

### Edge cases / errors

- 若 docs-site 或 Storybook 产物目录不存在，装配脚本必须报错退出，而不是生成不完整站点。
- 若 `storybook.html` 缺失关键跳转文案，CI smoke 必须失败。
- 若 Storybook 端口被显式设置为 `6006`，本地脚本必须拒绝启动。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `DOCS_PORT` | env | internal | New | None | docs-site | 本地 docs-site dev/preview | 默认 `60081` |
| `DOCS_BASE` | env | internal | New | None | docs-site / CI | GitHub Pages build | 用于子路径部署 |
| `VITE_STORYBOOK_DEV_ORIGIN` | env | internal | New | None | docs-site | 本地 docs-site `storybook.html` | 默认回退 `http://127.0.0.1:60082` |
| `STORYBOOK_PORT` | env | internal | Modify | None | `web/scripts/run-storybook.mjs` | 本地 Storybook dev | 默认 `60082` |
| `Docs Pages` | workflow | internal | New | None | `.github/workflows/docs-pages.yml` | PR artifacts / GitHub Pages | 不加入 required checks |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given `DOCS_BASE=/<repo>/`
  When 构建 docs-site
  Then 生成的导航与资源路径可在 GitHub Pages 子路径下正常解析，并能直接访问 `quick-start`、`config`、`deployment`、`troubleshooting`、`development`、`product` 与 `storybook.html`。
- Given `cd web && bun run storybook`
  When 不传 `STORYBOOK_PORT`
  Then Storybook 在 `http://127.0.0.1:60082` 启动，并拒绝使用 `6006`。
- Given docs-site build 与 Storybook static build 都成功
  When 执行 `.github/scripts/assemble-pages-site.sh`
  Then 输出站点同时包含根 docs、`storybook.html` 与 `/storybook/index.html`。
- Given PR 修改了 docs-site / Storybook / 装配脚本
  When `CI PR` 运行
  Then `Lint & Format Check` 内的 docs build / storybook build / assembled-site smoke 全部通过。
- Given `main` 分支收到 docs-site 或 Storybook 相关变更
  When `Docs Pages` workflow 运行
  Then 会先产出 docs、storybook 与 assembled artifacts，再部署 GitHub Pages。

## 实现前置条件（Definition of Ready / Preconditions）

- 公开文档页集合与导航项已冻结；其中允许在不改变 Storybook / Pages 组装契约的前提下继续优化 public IA
- 本地端口合同已冻结：app dev `60080`、docs-site `60081`、Storybook `60082`
- docs-site 只承载 public docs，内部实现规范继续留在 `docs/ui/**`
- 现有 quality-gates required check 名称保持不变

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cd docs-site && bun run build`
- `cd web && bun run storybook:build`
- `bash .github/scripts/assemble-pages-site.sh docs-site/doc_build web/storybook-static .tmp/pages-site`
- 浏览器验收：`docs-site` 首页、`storybook.html` 重定向、一个 Storybook docs 深链，以及按 `DOCS_BASE` 子路径提供的 assembled `/storybook/` 访问

### UI / Storybook (if applicable)

- Docs pages / state galleries to add/update: `docs-site/docs/storybook.mdx`
- Stories to reference: `TodayStatsOverview`, `ForwardProxyLiveTable`, `InvocationTable`, `RecordsPage`, `SettingsPage`, `UpstreamAccountsPage`, `TagsPage`

### Quality checks

- `bun install --cwd docs-site`
- `bun install --cwd web`
- `bun run check:bun-first`
- `python3 .github/scripts/check_quality_gates_contract.py --repo-root "$PWD"`

## 文档更新（Docs to Update）

- `README.md`: 增加 public docs / docs-site / Storybook / Pages 入口与本地 URL 合同
- `web/README.md`: 替换模板内容，明确 app / Storybook / docs-site 协作方式
- `docs/ui/README.md`: 明确 public docs 与内部 UI 规范的边界
- `docs/ui/storybook.md`: 增加 public docs/storybook 回链说明

## 计划资产（Plan assets）

- Directory: `docs/specs/j9frr-docs-site-storybook-pages/assets/`
- In-plan references: None
- PR visual evidence source: maintain `## Visual Evidence (PR)` only if screenshots become necessary during PR convergence

## Visual Evidence (PR)

None.

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 建立 `docs-site/` Rspress 文档站与 public docs 页面骨架
- [x] M2: 固定 Storybook 开发/构建接口并完成与 docs-site 的入口对接
- [x] M3: 接入 assembled-site 脚本、`CI PR` / `CI Main` smoke 与 `Docs Pages` workflow
- [x] M4: 更新 README / UI docs 并完成本地验证、浏览器验收与 fast-track PR 收敛

## 方案概述（Approach, high-level）

- 直接复用 `octo-rill` 的“docs-site + Storybook + assembled Pages”骨架，但内容与端口合同按本仓库当前页面与 CI 约束收敛。
- public docs 聚焦项目介绍、自部署 onboarding、配置与运行、排障、开发入口与 Storybook 导航；更深的内部规范和 spec 仍保留在 repo docs。
- GitHub Pages 发布保持独立 workflow，PR 阶段的质量门由现有 lint 主链代管，避免 branch protection 契约扩张。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：Storybook 入口若携带固定 docs path 参数，会在 story id 变化时形成死链；当前方案避免单独维护导览页来降低这类漂移。
- 风险：Rspress / Storybook 的子路径资源引用若处理不当，最容易在 Pages 部署后暴露。
- 开放问题：无。
- 假设：仓库设置允许 GitHub Actions 部署 Pages。

## 变更记录（Change log）

- 2026-03-19: 创建 spec，冻结 docs-site、Storybook、Pages 与 CI smoke 的首版交付范围。
- 2026-03-19: 完成 docs-site / Storybook / Pages 装配实现、targeted validation 与浏览器验收，进入 fast-track PR 收敛阶段。
- 2026-03-19: 参考 `tavily-hikari` 的 task-based IA，把 public docs 重构为“项目介绍 + 快速开始 + 配置与运行 + 自部署 + 排障 + 开发 + Storybook”分工，并强化自部署读者的最短路径。
- 2026-03-19: 删除独立 `storybook-guide` 页面，改为只保留 `storybook.html` 作为 public docs 的 Storybook 入口。

## 参考（References）

- `/Users/ivan/.codex/tmp/project-catalog/checkouts/octo-rill/docs-site/`
- `/Users/ivan/.codex/tmp/project-catalog/checkouts/octo-rill/.github/workflows/docs-pages.yml`
- `README.md`
- `docs/deployment.md`
- `docs/ui/README.md`
- `docs/ui/storybook.md`
