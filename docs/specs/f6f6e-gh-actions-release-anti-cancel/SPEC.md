# GH Actions 防取消发布链路全面对齐（#f6f6e）

## 状态

- Status: 部分完成（3/4）
- Created: 2026-03-14
- Last: 2026-03-15

## 背景 / 问题陈述

- 当前仓库把 PR 检查、`main` 校验与 release 发布都耦合在 `.github/workflows/ci.yml`，只能做到“运行中的 `main` run 不被新 push 取消”，不能从工作流拓扑上明确区分 PR 与发布职责。
- style playbook 的 `PR label release` 约定要求：PR 检查可抢占、`main` CI 与 release run 非抢占、并为 burst merges 下可能丢失的 pending release 提供明确的 backfill 路径；同时不能让 `CI Main` 自己因为共享 pending 队列静默漏掉 merged commit。
- 仓库内置 `quality-gates` 当前仍处于 `bootstrap` profile；若直接拆 workflow 而不升级 contract/fixtures/self-tests，仓库自己的 CI 契约会先漂移失效。

## 目标 / 非目标

### Goals

- 拆分为 `CI PR`、`CI Main`、`Release` 三段式链路，明确并发语义与职责边界。
- 保留现有 PR label 驱动的 release intent、版本/tag 规则、多架构 smoke 与发布幂等行为。
- 为 release 增加 `workflow_dispatch(commit_sha)` 手动补发入口，作为需要显式重放历史 commit 时的人工 backfill 通道；入口接受已经成功通过 `CI Main` 的 `main` commit，以及仅在 `Release Snapshot` 失败、其余 `CI Main` 校验均成功的历史 commit；手动 backfill 只补齐并发布目标 commit。
- 将 `quality-gates`、trusted metadata gate、contract fixtures/self-tests 升级到 `final` profile。

### Non-goals

- 不启用 merge queue、定时 reconciliation 或自动给 PR 打 label。
- 不修改应用运行时代码、HTTP/DB 契约或 Docker 镜像内容。
- 不改变 required check 名称与 branch protection 语义。

## 范围（Scope）

### In scope

- 重构 `.github/workflows/ci.yml` 为 `ci-pr.yml`、`ci-main.yml` 与 `release.yml`。
- 升级 `.github/workflows/label-gate.yml`、`.github/workflows/review-policy.yml` 到 trusted-source final 版本。
- 调整 `.github/quality-gates.json`、`.github/scripts/check_quality_gates_contract.py`、fixtures 与自测脚本，适配 split workflow topology。
- 更新 README 与 spec 文档，说明严格防取消与手动 backfill 的操作口径。

### Out of scope

- GitHub 仓库线上 branch protection 配置本身的人工修改。
- 新增 release 定时器、外部调度器或长期运行的补发巡检任务。

## 需求（Requirements）

### MUST

- PR 侧 required checks 继续保持 `Validate PR labels`、`Lint & Format Check`、`Backend Tests`、`Build Artifacts`、`Review Policy Gate`。
- `CI PR` 对同一 PR 必须保持可抢占；`CI Main` 与 `Release` 对运行中的 main/release run 必须保持非抢占，并使用固定并发组做全局串行。
- `Label Gate` 必须在 trusted base 上校验 merged PR 进入主线前的 `type:*` / `channel:*` 标签合法性，但不再负责冻结或传递发布意图元数据。
- `CI Main` 必须为 mainline 上尚未持久化的 merged commits 写入 immutable release snapshot，冻结当前 PR labels、版本分配与镜像/tag 元数据；后续成功的 `CI Main` run 必须能够 catch up 之前因 pending 替换而漏掉的 commits。
- `CI Main` 写 snapshot 时，自动发布路径必须直接读取 merged commit 关联 PR 的当前 labels；不得依赖 artifact、timeline label 回放或历史 rollout 分支。
- `Release` 必须同时支持 `workflow_run(CI Main success)` 与 `workflow_dispatch(commit_sha)` 两种入口，并复用同一套 publish 逻辑；自动入口每次只发布 mainline 上最早一个尚未发布的 snapshot，成功后继续串行排下一个；自动与手动入口都只能消费 immutable release snapshot，禁止重新读取 PR labels 或重算版本。
- `workflow_dispatch` 只接受 `commit_sha`，且对非法/不可解析输入、既未通过 `CI Main` 也不满足“仅 `Release Snapshot` 失败”的目标 SHA 一律 fail closed。
- merge 后的 `type:*` / `channel:*` labels 视为发布输入的一部分；若合并后人为改动标签，后续 backfill 将按改动后的标签重建 snapshot，仓库规则必须禁止这种操作。
- `quality-gates` contract、fixtures 与自测必须升级到 `final` profile，并校验新的 workflow 家族。

### SHOULD

- 将 release 相关 job 从 PR 可见的 informational checks 中解耦，避免把不在 PR 触发的 job 继续伪装成 PR checks。
- README 明确说明 GitHub concurrency 不能保证 FIFO，但 mainline catch-up 会把被替换的 pending snapshot / release 重新排回队列；同时写清何时使用手动 backfill。

## 接口契约（Interfaces & Contracts）

- 无应用接口变更。
- GitHub Actions 内部契约变更：
  - 新增 workflow `CI PR`、`CI Main`、`Release`。
- `Release` 新增 `workflow_dispatch` 输入 `commit_sha`（40 位 commit SHA）。
- `Label Gate` 不再生成额外 artifact，发布意图接口收敛为 merged PR 当前 labels。
- `quality-gates.json` 新增 split-topology workflow 声明，作为 contract checker 的源事实。

## 验收标准（Acceptance Criteria）

- Given 一个新的 PR
  When GitHub Actions 触发检查
  Then `CI PR` 运行 required checks，且同一 PR 的旧 run 会被新提交取消。
- Given `main` 上连续合入多个 PR
  When 新的 `push main` 到来
  Then 当前运行中的 `CI Main` 不会被取消，较早 merged commit 即使曾因 pending 替换错过单独 run，也会在后续成功的 `CI Main` 中补齐 snapshot；`Release` 运行中的发布不会被新 release run 打断。
- Given 某个 merged commit 的自动 release pending run 被更晚的 pending run 替换
  When 后续仍有新的 `CI Main` / `Release` 成功运行
  Then `Release` 会按最早未发布 snapshot 继续排队，最终仍会把该 commit 的 release 补齐，且 stable 版本号保持单调递增。
  When 维护者手动触发 `Release` 并传入该 commit SHA
  Then workflow 仅在该 commit 已成功通过 `CI Main` 且已有 immutable snapshot 时继续执行，随后只复用 snapshot 中冻结的 labels/version/tag 元数据来完成或跳过发布。
- Given 本仓库执行 `python3 .github/scripts/check_quality_gates_contract.py --repo-root "$PWD" --profile final`
  When 校验 split topology
  Then contract、fixtures 与 trusted metadata gate 全部通过。

## 非功能性验收 / 质量门槛（Quality Gates）

### Quality checks

- `python3 .github/scripts/check_quality_gates_contract.py --repo-root "$PWD" --profile final`
- `bash .github/scripts/test-quality-gates-contract.sh`
- `bash .github/scripts/test-inline-metadata-workflows.sh`
- `bash .github/scripts/test-live-quality-gates.sh`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 拆分 `CI PR` / `CI Main`，固定并发语义并保留 required check 名称
- [x] M2: 升级 `label-gate` / `review-policy` / `quality-gates` 到 final trusted-source contract
- [x] M3: 抽离独立 `Release` workflow，并支持 `workflow_run` + `workflow_dispatch(commit_sha)`
- [ ] M4: README、fixtures、自测、快车道 PR 收敛全部完成

## 方案概述（Approach, high-level）

- PR 路径只保留 PR/merge_group 相关 job；release job 从 PR workflow 中完全拆出。
- `CI Main` 复用现有 lint/test/build 逻辑，但不再承担发布；发布只由 `Release` workflow 负责。
- `CI Main` 通过 git notes 写入 immutable release snapshot，把 PR labels、版本分配与镜像/tag 元数据冻结到 merge commit。
- `Label Gate` 在 trusted source 上校验标签；`CI Main` 再把当前 PR labels 提升为 immutable git-notes snapshot。
- `Release` 通过统一的 target SHA 解析层兼容自动与手动入口，但只加载 snapshot 并复用现有 smoke / manifest / tag / GitHub Release 步骤。
- `quality-gates` contract 扩展为显式声明 PR/main/release workflow 家族，contract checker 与 fixtures 一起升级，避免只改 workflow 不改自证体系。

## 风险 / 假设（Risks / Assumptions）

- 风险：workflow 拆分后，contract checker、fixtures 与 live-quality-gates 之间容易出现声明不一致。
- 风险：`workflow_dispatch` backfill 若未验证 SHA 所属分支，可能误对非 main commit 执行发布。
- 风险：若合并后有人手改 release labels，后续手动 backfill 会跟随漂移，因此必须把“merge 后不得改 release labels”写成仓库操作规约。
- 假设：仓库权限允许 `workflow_run` 触发 release 并继续推 tag / 建 GitHub Release。

## 变更记录（Change log）

- 2026-03-14: 创建 strict anti-cancel release topology spec，冻结三段式 workflow + final quality-gates 升级范围。
- 2026-03-14: 完成 workflow split、final quality-gates contract、release backfill 入口与本地 contract/self-tests。
- 2026-03-15: 将发布链路进一步收敛为“PR 标签校验 → 全局串行 `CI Main` 写/补 snapshot → 全局串行 `Release` 按最早未发布 snapshot 排队发布”，删除 artifact、rollout 与 legacy fallback 复杂度。

## 参考（References）

- `docs/plan/0002:pr-label-release/PLAN.md`
- `/Users/ivan/.style-playbook-skills/skills/style-playbook/references/tags/pr-label-release.md`
