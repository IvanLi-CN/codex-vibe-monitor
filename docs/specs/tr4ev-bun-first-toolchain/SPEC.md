# Bun-first 工具链收敛（#tr4ev）

## 状态

- Status: 已完成
- Created: 2026-03-12
- Last: 2026-03-12

## 背景 / 问题陈述

- 仓库当前把 `npm` / `npx` / Node 20 作为前端与交付链路默认入口：`README.md`、`AGENTS.md`、`lefthook.yml`、`Dockerfile`、`.github/workflows/ci.yml`、`web/package.json` 都直接依赖这些命令。
- 仓库内没有 `bun` 配置、版本钉住文件或 lockfile，导致“应该使用 Bun”只停留在口头约定，没有落到可执行的工程面。
- 需要把直接执行面统一收敛到 Bun-first，同时保持 Rust 后端、业务 API、SQLite schema 与前端运行行为不变。

## 目标 / 非目标

### Goals

- 将仓库根与 `web/` 两侧的安装、脚本执行、hooks、CI、Docker builder、开发文档统一切换为 Bun/Bunx/Bun image。
- 为仓库根与 `web/` 分别生成并提交文本 `bun.lock`，删除对应 `package-lock.json`，避免继续把 npm lockfile 当作唯一事实源。
- 固定 Bun 版本入口（`.bun-version`），让本地与 CI 共享同一版本基线。
- 新增一条只扫描运营面的 Bun-first 守门检查，阻止重新引入 `npm` / `npx` / `setup-node` / `package-lock` / 直接 `node <script>`。
- 在 PR 阶段补上 Docker smoke，确保 Bun 版前端 builder 不会等到 main 分支发布时才失败。

### Non-goals

- 不把仓库根与 `web/` 合并成 workspace。
- 不改 Rust 后端实现、HTTP API、数据库 schema、迁移或运行时业务逻辑。
- 不批量改写历史 `docs/specs/**` 与 `docs/plan/**` 中已经记录为历史事实的 `npm` 命令。
- 不为“看起来更纯”删除仍被工具链合理使用的 `node:` 标准库导入、`@types/node`、`tsconfig.node.json`。

## 范围（Scope）

### In scope

- `/Users/ivan/.codex/worktrees/41dd/codex-vibe-monitor/.bun-version`
- `/Users/ivan/.codex/worktrees/41dd/codex-vibe-monitor/package.json`
- `/Users/ivan/.codex/worktrees/41dd/codex-vibe-monitor/web/package.json`
- `/Users/ivan/.codex/worktrees/41dd/codex-vibe-monitor/bun.lock`
- `/Users/ivan/.codex/worktrees/41dd/codex-vibe-monitor/web/bun.lock`
- `/Users/ivan/.codex/worktrees/41dd/codex-vibe-monitor/README.md`
- `/Users/ivan/.codex/worktrees/41dd/codex-vibe-monitor/AGENTS.md`
- `/Users/ivan/.codex/worktrees/41dd/codex-vibe-monitor/lefthook.yml`
- `/Users/ivan/.codex/worktrees/41dd/codex-vibe-monitor/Dockerfile`
- `/Users/ivan/.codex/worktrees/41dd/codex-vibe-monitor/.github/workflows/ci.yml`
- `/Users/ivan/.codex/worktrees/41dd/codex-vibe-monitor/.github/scripts/check-bun-first.sh`
- `/Users/ivan/.codex/worktrees/41dd/codex-vibe-monitor/docs/specs/README.md`

### Out of scope

- `src/**`
- `web/src/**`
- 数据库迁移或 schema 文件
- 历史已完成 spec/plan 内容批量替换

## 接口契约（Interfaces & Contracts）

- 对外业务接口保持不变：不新增、不删除、不重命名任何 HTTP API、SSE 事件、SQLite 表结构或前端业务类型。
- 对开发/交付接口做以下收敛：
  - 安装命令：`bun install` / `cd web && bun install`
  - 前端脚本：`bun run dev|build|test|build-storybook`
  - hooks 内可执行文件：`bun eslint`、`bun tsc`、`bun dprint`、`bun commitlint`
  - CI 安装器：`oven-sh/setup-bun@v2`
  - Docker web builder：官方 `oven/bun` 镜像 + `bun install --frozen-lockfile` + `bun run build`
- Bun-first guard 只检查运营面文件；允许以下残留：
  - `docs/specs/**/SPEC.md`
  - `docs/plan/**`
  - lockfile 内容
  - `node:` import
  - `@types/node`
  - `tsconfig.node.json`

## 验收标准（Acceptance Criteria）

- Given 在仓库根与 `web/` 执行 `bun install --frozen-lockfile`，When 安装完成，Then 两侧都能使用各自 `bun.lock` 成功解析依赖，且 `package-lock.json` 已不存在。
- Given 执行本地开发与验证命令，When 运行 `bun run lint`、`bun run test`、`bun run build`、`bun run build-storybook`，Then 不再需要 `npm` / `npx` / 直接 `node <script>`。
- Given GitHub PR CI 运行，When 进入 `Lint & Format Check` 与 `Build Artifacts`，Then workflow 使用 `setup-bun` 和 Bun 安装流程，并在 `Build Artifacts` 内完成 Docker smoke。
- Given Docker build 使用新的 web builder，When 构建镜像并运行 `/.github/scripts/smoke-test-image.sh`，Then 镜像可以成功产出前端静态资源并通过 `/health` smoke。
- Given 运行 Bun-first guard，When 扫描运营面文件，Then 不再命中禁止项；若重新引入禁止项则 guard 失败。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `bun install --frozen-lockfile`
- `cd web && bun install --frozen-lockfile`
- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo test --locked --all-features`
- `cd web && bun run lint`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run build-storybook`
- `bun run check:bun-first`
- `docker build -t codex-vibe-monitor:bun-smoke --build-arg APP_EFFECTIVE_VERSION=dev .`
- `./.github/scripts/smoke-test-image.sh codex-vibe-monitor:bun-smoke`

### Quality checks

- `codex --sandbox read-only -a never review --base origin/main`
- PR required checks 保持为 `Validate PR labels`、`Lint & Format Check`、`Backend Tests`、`Build Artifacts`、`Review Policy Gate`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 建立 Bun-first spec，冻结“只改直接执行面、不动业务接口”的范围。
- [x] M2: 完成仓库根与 `web/` 的 Bun lockfile、脚本、hooks、Docker、CI、文档迁移。
- [x] M3: 跑通本地验证、Docker smoke、PR checks 与 review-loop 收敛。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：Bun 与 Vite/Storybook/Playwright 的兼容性若有边角差异，可能首先暴露在 build 或 storybook 阶段；必须以实际命令验证收敛。
- 风险：Docker builder 从 Node 切到 Bun 后，若 lockfile / install 逻辑不一致，可能只在容器构建阶段暴露。
- 风险：CI 切换到 Bun 后，若 required check 名称意外变化会影响分支保护，因此只能替换 step/命令，不能改 job name。
- 开放问题：None。
- 假设：Bun `1.3.10` 可兼容当前仓库的 React/Vite/Storybook/Playwright 依赖组合。

## 变更记录（Change log）

- 2026-03-12: 创建 spec，冻结“Bun-first direct execution surface”定义、允许残留项与 PR 阶段 Docker smoke 要求。
- 2026-03-12: 仓库根与 `web/` 已迁移到文本 `bun.lock`，`package-lock.json` 删除，`README.md`、`AGENTS.md`、`lefthook.yml`、`Dockerfile`、`.github/workflows/ci.yml` 全部切换到 Bun-first 入口。
- 2026-03-12: 新增 `/.github/scripts/check-bun-first.sh` 作为运营面守门；本地已通过 `bun install --frozen-lockfile`（root + web）、`cargo fmt --all -- --check`、`cargo check --locked --all-targets --all-features`、`cargo test --locked --all-features`、`cd web && bun run lint`、`cd web && bun run test`、`cd web && bun run build`、`cd web && bun run build-storybook`，并在 shared testbox 完成 Docker smoke。
- 2026-03-12: PR #115 已创建并打上 `type:skip` / `channel:stable`；GitHub required checks 通过，`spec_drift_check.sh --base-ref origin/main --spec-path docs/specs/tr4ev-bun-first-toolchain/SPEC.md` 返回 `Spec同步状态=通过` / `Spec漂移=不存在`，`codex review --base origin/main` 未发现阻塞项。
