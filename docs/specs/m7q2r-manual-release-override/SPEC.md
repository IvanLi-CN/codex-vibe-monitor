# 受控手动发版覆盖（#m7q2r）

## 背景 / 问题陈述

现有 `Release` workflow 的 `workflow_dispatch(commit_sha)` 只能重放已冻结的 release snapshot。若目标 commit 的原始 PR release intent 是 `type:skip` 或 `type:docs`，手动 backfill 仍会导出 `release_enabled=false`，无法对已经进入 `main` 且通过质量门禁的 commit 做维护者显式发版。

## 目标 / 非目标

### Goals

- 允许维护者对 `origin/main` 上的 40 位 commit SHA 发起受控手动发版。
- 手动发版必须显式选择 `version` 或 `bump`，并提供 reason。
- 手动发版必须复用现有多架构 build、smoke、manifest、Git tag、GitHub Release 与 PR comment 发布链路。
- 手动覆盖信息必须可审计，至少出现在 GitHub Release body 与 workflow log 中。
- 自动 PR label 发版路径继续只消费 immutable release snapshot，不受手动覆盖影响。

### Non-goals

- 不覆盖、迁移或删除 `refs/notes/release-snapshots` 中的自动 snapshot。
- 不改变 `type:*` / `channel:*` PR label gate。
- 不新增真实发版以外的长期审计存储。
- 不改变 Docker 平台矩阵、runner 拓扑或 publish 顺序。

## 范围（Scope）

- `.github/workflows/release.yml`：增加手动覆盖输入、dispatch 分流与 Release body 审计输出。
- `.github/scripts/release_snapshot.py`：增加 job-local `manual-release-override` snapshot 构造与校验。
- `.github/scripts/test-release-snapshot.sh` 与 quality-gates contract fixtures：覆盖手动发版输入和 workflow 拓扑。

## 需求（Requirements）

### MUST

- `commit_sha` 必须是 40 位 SHA，且必须包含在 `origin/main`。
- 手动覆盖必须满足 `version` 与 `bump` 严格二选一；`bump` 仅允许 `patch|minor|major`。
- 手动覆盖必须提供非空 `reason`，并记录 actor 与触发时间。
- 目标 commit 必须通过 `CI Main`，或满足既有 snapshot-only CI Main failure 例外。
- `version` 可接受 `X.Y.Z` 或 `vX.Y.Z`，规范化后必须大于当前最新 stable semver tag。
- `bump` 基于当前最新 stable semver tag 推算；无 stable tag 时使用目标 commit 的 `Cargo.toml` version 作为基线。
- 目标 Git tag 不存在时才新建；若已存在且指向目标 commit，允许幂等恢复；若指向其它 commit，必须失败。
- `channel=stable` 发布时更新 `latest`；`channel=rc` 生成 `vX.Y.Z-rc.<sha7>` 且不更新 `latest`。

### SHOULD

- 只在 `workflow_dispatch` 提供手动覆盖输入时启用 manual override snapshot。
- 不带 `version`、`bump`、`reason` 的内部 release queue dispatch 继续按 immutable snapshot backfill 行为运行。
- Release body 中直接展示 source、actor、triggered_at、version/bump 与 reason。

## 验收标准（Acceptance Criteria）

- Given `type:skip` merged commit 已通过 `CI Main`
  When 维护者用 `workflow_dispatch` 提供 `bump=patch` 与 `reason`
  Then workflow 生成 `snapshot_source=manual-release-override` 且进入现有 publish jobs。
- Given 维护者提供 `version=v2.19.1`
  When 当前最新 stable tag 小于 `v2.19.1`
  Then 发布 tag 为 `v2.19.1`。
- Given 维护者提供 `version=2.19.1` 且 `channel=rc`
  When 触发手动发版
  Then 发布 tag 为 `v2.19.1-rc.<sha7>` 且不推送 `latest`。
- Given `version` 与 `bump` 同时为空或同时提供
  When 触发手动覆盖
  Then workflow 在 release meta 阶段失败。
- Given 目标 tag 已存在并指向其它 commit
  When 触发手动覆盖
  Then workflow 失败且不进入 publish jobs。

## 参考（References）

- `.github/workflows/release.yml`
- `.github/scripts/release_snapshot.py`
- `docs/specs/8239m-release-latest-published-stable/SPEC.md`
- `docs/solutions/workflow/release-queue-ci-main-eligibility.md`
