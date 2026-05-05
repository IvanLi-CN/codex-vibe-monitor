# Release PR 评论权限补齐（#q1m9k）

## 状态

- Status: 进行中
- Created: 2026-03-17
- Last: 2026-03-17

## 背景 / 问题陈述

- `Release` workflow 已经执行到 `Upsert PR release version comment`，但线上真实 run 对 `PR #161` 返回 `Resource not accessible by integration`。
- 同一个 job 内 tag、GitHub Release 与镜像发布均成功，说明故障集中在 PR 评论所需的权限声明。

## 目标 / 非目标

### Goals

- 补齐 release 评论步骤的 GitHub token 权限，使已完成的 release 能把版本评论写回对应 PR。
- 用 contract / fixture 锁定新增权限，避免后续漂移。

### Non-goals

- 不改变 release 版本计算、tag / GitHub Release 创建或 release queue 串行逻辑。
- 不把 PR 评论失败升级为发布失败。

## 范围（Scope）

### In scope

- 更新 `.github/workflows/release.yml` 的 `release-publish.permissions`。
- 更新 quality-gates contract 与 fixtures。

### Out of scope

- 重新设计评论正文。
- 修改 branch protection 或仓库级 Actions policy。

## 需求（Requirements）

### MUST

- `release-publish` 必须显式声明足够的 PR 评论权限。
- contract checker 必须校验新增权限。
- 本地 contract/self-test 必须通过。

## 验收标准（Acceptance Criteria）

- Given 一个 `type:{patch|minor|major}` 的 release
  When `Release Publish` 执行 PR 评论步骤
  Then workflow 不再因权限不足返回 `Resource not accessible by integration`。
- Given 运行 `python3 .github/scripts/check_quality_gates_contract.py --repo-root "$PWD" --profile final` 与 `bash .github/scripts/test-quality-gates-contract.sh`
  When 本地校验执行
  Then 二者都通过。

## 质量门槛（Quality Gates）

- `python3 .github/scripts/check_quality_gates_contract.py --repo-root "$PWD" --profile final`
- `bash .github/scripts/test-quality-gates-contract.sh`

## 里程碑（Milestones）

- [ ] M1: 为 release-publish 补齐 PR 评论权限
- [ ] M2: 更新 contract / fixture 并完成本地验证
- [ ] M3: fast-flow 推进到 PR checks 收敛

## 参考（References）

- `.github/workflows/release.yml`
- `docs/specs/sq8gw-release-pr-version-comment/SPEC.md`
