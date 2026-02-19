# PR 标签驱动发版（#0002）

## 状态

- Status: 已完成
- Created: 2026-02-19
- Last: 2026-02-19

## 问题陈述

当前仓库在 `main` 每次 push 时都会构建并推送 GHCR 镜像，并尝试创建 `v${APP_EFFECTIVE_VERSION}` tag。现状存在几类问题：

- “是否发版 / 发哪个版本”缺少明确、可审计、可复现的决策来源（容易被误触发或产生歧义）。
- 预发行（RC）与稳定版（stable）缺少统一规范（tag 命名、是否更新 `latest`、是否创建 GitHub Release）。
- tag/Release 幂等性与失败策略不够严格（当前 `|| true` 会吞掉错误，可能掩盖问题）。

需要引入“PR 标签驱动”的发版意图（release intent）作为源事实，并在 CI 中做确定性 gate。

## 目标 / 非目标

### Goals

- 引入 PR 标签作为发版意图源事实（type/channel），合并前强校验。
- 合并到 `main` 后，由 CI 解析关联 PR 的 labels，决定是否发版、发版渠道（stable/rc）与 semver bump（major/minor/patch）。
- 统一版本与 tag 规则：
  - stable：`vX.Y.Z`
  - rc：`vX.Y.Z-rc.<sha7>`
- 产物规则：
  - stable：推 `${image}:vX.Y.Z` 与 `${image}:latest`，并创建 GitHub Release（非 prerelease）
  - rc：仅推 `${image}:vX.Y.Z-rc.<sha7>`，并创建 GitHub Release（prerelease）
  - type:docs/type:skip：不推镜像、不打 tag、不创建 Release
- 幂等与安全：
  - tag/Release 已存在且指向同一 SHA 时 rerun 不失败
  - tag 已存在但指向不同 SHA 时失败（保护漂移）

### Non-goals

- 不切换为 `workflow_run` 触发（本计划固定沿用 `push main` 触发）。
- 不引入自动生成 changelog 的体系（Release body 仅做最小信息记录）。
- 不新增多架构构建策略与额外平台矩阵（仍按仓库现有 `PLATFORMS`）。

## 范围（Scope）

### In scope

- 创建/维护发版相关的 GitHub labels（`type:*` 与 `channel:*`）。
- 新增 PR label gate workflow，保证合并前标签规则确定且可见。
- 改造 `push main` 的 docker job：
  - 通过 commit SHA 反查关联 PR
  - 解析 labels 得出 release intent
  - 计算版本、推镜像、创建 tag、创建 GitHub Release
- 更新 README 文档口径（如何打标签、stable/rc 与产物规则）。

### Out of scope

- 自动给 PR 打标签或自动选择 bump（由提交者显式设置 label）。
- 自动推送非发版的 sha 镜像（本计划明确“不发版就不推镜像”）。

## 需求（Requirements）

### MUST

- PR 必须且只能有 1 个 `type:*` 与 1 个 `channel:*`：
  - `type:patch` | `type:minor` | `type:major` -> 发版
  - `type:docs` | `type:skip` -> 不发版
  - `channel:stable` -> stable
  - `channel:rc` -> prerelease
- 未知或缺失 labels 必须在 PR 阶段失败（label gate）。
- `main` 的发版必须通过 release-intent gate 决定；若无法唯一定位 PR（0 或 >1），默认跳过发版（更安全）。
- 版本计算必须以 “最大 stable tag（vX.Y.Z）”为基线做 bump，并按 channel 生成 stable/rc tag。
- 创建 tag 与 GitHub Release 必须是幂等的（rerun 不应失败）。

### SHOULD

- CI 失败信息清晰可读（说明缺失/冲突 labels、无法解析 PR、tag 漂移等原因）。
- README 说明要点与例子保持简洁（尽量避免重复描述 CI 内部细节）。

## 验收标准（Acceptance Criteria）

- Given PR 打了 `type:docs` + `channel:stable`
  When 合并到 `main`
  Then 不推镜像、不创建 tag、不创建 GitHub Release，CI 仍通过。
- Given PR 打了 `type:patch` + `channel:stable`
  When 合并到 `main`
  Then 推 `${image}:vX.Y.(Z+1)` 与 `${image}:latest`，创建 `vX.Y.(Z+1)` tag 与 GitHub Release（非 prerelease）。
- Given PR 打了 `type:minor` + `channel:rc`
  When 合并到 `main`
  Then 推 `${image}:vX.(Y+1).0-rc.<sha7>`，不更新 `latest`，创建对应 tag 与 GitHub Release（prerelease）。
- Given 发生 rerun（同一 commit）
  When tag/Release 已存在且指向同一 SHA
  Then rerun 不失败。

## 非功能性验收 / 质量门槛（Quality Gates）

- 本地至少执行 1 条与改动相关的自动化验证：
  - `bash -n .github/scripts/compute-version.sh`
- PR 的 CI（lint/unit-tests/build）应保持通过；label gate 应通过。

## 里程碑（Milestones）

- [x] M1: 创建 labels + 新增 label gate workflow
- [x] M2: release-intent gate + 版本计算（stable/rc）落地
- [x] M3: tag 创建幂等 + GitHub Release 创建幂等 + README 更新

## 参考（References）

- Style playbook: `PR label release`（`/Users/ivan/.codex/skills/style-playbook/references/tags/pr-label-release.md`）

## 变更记录（Change log）

- 2026-02-19: 落地 PR 标签驱动发版（label gate + CI release gate + 版本/tag/Release 规则）。PR #36。
