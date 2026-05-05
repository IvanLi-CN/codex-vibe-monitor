# Release 工作流 PR 版本评论（#sq8gw）

## 状态

- Status: 进行中
- Created: 2026-03-17
- Last: 2026-03-17

## 背景 / 问题陈述

- 现有 `Release` workflow 会创建 tag、GitHub Release 与多架构镜像，但 merged PR 本身不会收到“本次实际发布了哪个版本”的直接反馈。
- 当 release queue 串行补发历史 main commit，或者维护者通过 `workflow_dispatch(commit_sha)` 做 backfill 时，PR 页面缺少一个稳定位置来确认最终发布版本号。

## 目标 / 非目标

### Goals

- 在 `Release Publish` 成功完成后，把本次发布的版本信息同步到对应 PR 评论区。
- 评论内容至少包含 release tag、应用版本号与目标 commit，并链接到对应 GitHub Release。
- 对 rerun / backfill 保持幂等：同一个 PR 只维护一条机器评论，后续发布更新该评论而不是重复刷屏。
- 不影响现有 release queue、tag/Release 幂等与多架构发布流程。

### Non-goals

- 不修改 PR label 规则、版本分配规则或 release queue 调度逻辑。
- 不为 `type:docs` / `type:skip` 增加额外评论。
- 不新增应用运行时代码、数据库结构或前端行为。

## 范围（Scope）

### In scope

- 更新 `.github/workflows/release.yml`，在 release 发布后 upsert PR 版本评论。
- 为该评论步骤补齐最小权限与 quality-gates contract / fixture 自证。
- 更新 spec 索引，记录这次 workflow 增量。

### Out of scope

- 变更 GitHub branch protection、仓库 rulesets 或标签管理策略。
- 变更 GitHub Release body 模板。

## 需求（Requirements）

### MUST

- 仅在 `release_enabled == true` 的发布路径上执行 PR 评论。
- 评论必须基于 `release-meta` 已冻结的输出（`pr_number`、`release_tag`、`app_effective_version`、`target_sha`），不得重新推导版本。
- 评论必须带固定 marker，便于后续 rerun / backfill 更新同一条评论。
- 若目标 PR 不存在、`pr_number` 为空或评论 API 调用失败，workflow 应记录 notice / warning，但不能破坏已完成的发布结果或阻断后续 release queue。
- `release-publish` 必须显式声明完成评论所需的最小权限。
- `.github/scripts/check_quality_gates_contract.py` 与 fixtures 必须校验新权限和评论步骤，保证 contract 不漂移。

### SHOULD

- 评论正文简洁可扫描，直接给出 release tag、版本号、channel、commit 与 release 链接。
- rerun / backfill 时覆盖旧评论内容，始终保持 PR 上的版本注释是最新一次成功发布结果。

## 验收标准（Acceptance Criteria）

- Given 一个正常的 stable / rc 发布
  When `Release Publish` 完成
  Then 对应 merged PR 下会出现一条带固定 marker 的评论，展示本次发布版本与 release 链接。
- Given 同一个 commit 的 release workflow 被 rerun，或同一个 PR 对应的 backfill 重放
  When 评论步骤再次执行
  Then workflow 会更新已有机器评论，而不是新增第二条相同用途的评论。
- Given 仓库执行 `python3 .github/scripts/check_quality_gates_contract.py --repo-root "$PWD" --profile final`
  When 校验 release workflow contract
  Then 新增的评论权限与步骤约束会被验证并通过。

## 非功能性验收 / 质量门槛（Quality Gates）

### Quality checks

- `python3 .github/scripts/check_quality_gates_contract.py --repo-root "$PWD" --profile final`
- `bash .github/scripts/test-quality-gates-contract.sh`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 为 release-publish 增加 PR 版本评论步骤与最小权限
- [x] M2: 让评论步骤支持 marker-based upsert，兼容 rerun / backfill
- [x] M3: 更新 contract checker 与 fixtures
- [ ] M4: fast-flow 推进到 PR checks / review proof 收敛

## 风险 / 假设（Risks / Assumptions）

- 风险：GitHub comments API 短暂失败时，PR 版本评论可能缺席；本次选择 best-effort，不让评论失败回滚已完成发布。
- 假设：发布 commit 总能关联到唯一 merged PR，且 `release-meta.outputs.pr_number` 可用。

## 参考（References）

- `.github/workflows/release.yml`
- `docs/specs/f6f6e-gh-actions-release-anti-cancel/SPEC.md`
