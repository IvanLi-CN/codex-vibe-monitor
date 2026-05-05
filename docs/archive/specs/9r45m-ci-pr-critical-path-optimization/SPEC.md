# PR CI 关键路径提速（#9r45m）

## 状态

- Status: 已完成
- Created: 2026-03-13

## 背景 / 问题陈述

- PR run `23019610982` 虽然整体通过，但关键路径接近 12 分钟；`Build Artifacts` 必须等待 `lint`、`frontend-tests`、`records-overlay-e2e`、`unit-tests` 全部结束后才会启动。
- `Build Artifacts` job 内还重复执行前端构建与 Rust release build，而 Dockerfile 本身已经会完成这些构建，导致同一 PR 在 runner 上重复消耗数分钟。

## 目标 / 非目标

### Goals

- 缩短 PR 工作流关键路径，让 `Build Artifacts` 在 `pull_request` 场景下与其他检查并行启动。
- 保留现有 required check 名称与 `quality-gates` 契约，避免影响分支保护与 Review Policy。
- 将 PR smoke build 收敛到 Docker Buildx 单一路径，并为该路径启用 GitHub Actions 缓存。

### Non-goals

- 不修改 `push main` 的 release 流程与发布语义。
- 不调整 `Label Gate`、`Review Policy` 或 `quality-gates.json` 中的 required/informational checks 集合。
- 不变更应用代码、HTTP 接口、数据库或 Docker 运行时行为。

## 范围（Scope）

### In scope

- 修改 `.github/workflows/ci.yml` 中 `Build Artifacts` 的 PR 执行图与构建步骤。
- 新增/更新文档记录这次 PR CI 提速的基线、方案与验收口径。

### Out of scope

- release 多架构镜像构建链路。
- 本地 Docker 验证策略与共享测试机策略。

## 需求（Requirements）

### MUST

- `Build Artifacts` 在 PR 场景下不再依赖其他测试 job 完成后才启动。
- `Build Artifacts` 名称保持不变，继续作为 required check 暴露给 GitHub branch protection。
- PR smoke build 继续产出可被 `smoke-test-image.sh` 验证的本地镜像。
- 现有 `quality-gates` 合约校验与自测脚本继续通过。

### SHOULD

- 通过 Buildx 的 GitHub Actions cache 减少重复 Docker 构建成本。
- 删除 runner 上重复的 Bun/Rust 本地构建步骤，避免与 Dockerfile 中的构建流程重复。

## 接口契约（Interfaces & Contracts）

- 无应用接口变更。
- GitHub Actions 对外可见的 check 名称保持不变：`Build Artifacts` 仍为 required check。

## 验收标准（Acceptance Criteria）

- Given 一个新的 PR run
  When workflow 启动
  Then `Build Artifacts` 会与 `Lint & Format Check`、`Backend Tests`、`Front-end Tests` 等 job 并行排队/启动，而不是等待它们全部完成。
- Given `Build Artifacts` 运行
  When Docker image build 完成
  Then job 继续执行现有 `smoke-test-image.sh` 并通过 `/health` 验证。
- Given 本仓库的 `quality-gates` 校验脚本
  When 以 bootstrap profile 校验当前仓库
  Then 校验通过，且 required/informational checks 覆盖关系不变。

## 非功能性验收 / 质量门槛（Quality Gates）

### Quality checks

- `bash .github/scripts/test-quality-gates-contract.sh`
- `bash .github/scripts/test-live-quality-gates.sh`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 记录基线 run 与关键路径瓶颈
- [x] M2: 让 PR `Build Artifacts` 改为并行启动
- [x] M3: 用 Buildx cache 替换重复的 runner 本地构建步骤
- [x] M4: 跑通 quality-gates 契约自测

## 风险 / 假设（Risks / Assumptions）

- 风险：PR 更早启动 Docker build 会增加并发 runner 资源占用，但不会改变 required checks 语义。
- 假设：`docker/build-push-action@v6` 的 `load: true` 与 `type=gha` cache 可以兼容当前 `smoke-test-image.sh` 工作流。

## 参考（References）

- PR run: `23019610982`
- Job: `66852869055` (`Build Artifacts`)
