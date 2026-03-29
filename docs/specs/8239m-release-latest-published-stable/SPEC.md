# Release `latest` 仅指向最新已发布 stable（#8239m）

## 状态

- Status: 已实现，待 PR / CI 收敛
- Created: 2026-03-29
- Last: 2026-03-29

## 背景 / 问题陈述

- 当前 release snapshot 在写入 git notes 时，会把 stable 的 `tags_csv` 预写成 `vX.Y.Z + latest`。
- 真正发布时，`Release` workflow 又会调用 `release_snapshot.py export --resolve-publication-tags` 重新计算 tags。
- 现有重算逻辑把“主干后面存在更高 stable snapshot”直接等同于“存在更高已发布 stable”，导致手动 backfill、rerun 旧 stable 或 stable backlog 场景下，`latest` 可能被未发布的 pending stable 提前压掉。

## 目标 / 非目标

### Goals

- 明确并落地唯一语义：`latest` 只指向最新已发布 stable。
- pending stable snapshot 不能压掉当前已发布 stable 的 `latest`。
- stable 新 snapshot 的 immutable `tags_csv` 只保存版本 tag；`latest` 只在发布阶段动态解析。
- README、脚本帮助文案与回归测试统一到同一规则。

### Non-goals

- 不调整 semver bump、release queue、手动 backfill 入口或 PR label 规则。
- 不引入 registry API / GitHub Release API 作为新的发布状态来源。
- 不迁移历史 release snapshot note schema。

## 范围（Scope）

### In scope

- `.github/scripts/release_snapshot.py`：重构 immutable tags 与 publish-time tags 的职责边界。
- `.github/scripts/test-release-snapshot.sh`：补齐 pending stable、旧 stable rerun/backfill、rc 不更新 latest 的回归。
- `README.md`：更新 stable / rc 与 `latest` 的准确语义。

### Out of scope

- `.github/workflows/release.yml` 的工作流拓扑与权限模型。
- 应用运行时代码、HTTP API、数据库或前端界面。

## 需求（Requirements）

### MUST

- stable snapshot 写入 notes 时，`tags_csv` 只能包含不可变版本 tag。
- `publication_tags()` 追加 `latest` 时，只能根据“是否存在更高已发布 stable”判定；未发布 snapshot 不得参与压制。
- `release_tag_points_to_target()` 继续作为“该 snapshot 是否已发布”的权威信号。
- 历史 snapshot 即使仍带 `latest`，在 `export --resolve-publication-tags` 时也必须按新规则重新计算，避免旧 note 影响结果。

### SHOULD

- helper 命名直接体现 immutable tags 与 publish-time tags 的分工，避免再次出现双重真相源。
- README 明确写出：`rc` 永不更新 `latest`；stable 仅在不存在更高已发布 stable 时更新 `latest`。

## 验收标准（Acceptance Criteria）

- Given 某个 stable 已发布，且主干后面只有更高但未发布的 stable snapshot
  When 对该已发布 stable 运行 `publication_tags()`
  Then 结果仍包含 `${image}:latest`。
- Given 较新的 stable 已发布
  When rerun 或 backfill 较旧 stable
  Then 较旧 stable 只能导出版本 tag，不能把 `latest` 抢回去。
- Given `channel:rc`
  When 生成 snapshot 或解析 publication tags
  Then 输出只包含 rc 版本 tag，不包含 `latest`。
- Given 新写入的 stable snapshot
  When 查看 immutable note 中的 `tags_csv`
  Then 只包含 `${image}:vX.Y.Z`。

## 非功能性验收 / 质量门槛（Quality Gates）

- `bash .github/scripts/test-release-snapshot.sh`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 拆分 immutable tags 与 publish-time tags 责任边界
- [x] M2: 让 `latest` 只受更高已发布 stable 影响
- [x] M3: README 与脚本级回归测试对齐新语义
- [ ] M4: fast-track 推进到 PR / CI / review 收敛

## 风险 / 假设（Risks / Assumptions）

- 风险：历史 note 仍可能保存 `latest`，所以 release workflow 必须继续通过 `--resolve-publication-tags` 动态重算。
- 假设：当前 workflow 的发布顺序保持“manifest push/verify -> git tag -> GitHub Release”，因此 `release_tag_points_to_target()` 足以代表“已发布 stable”。

## 变更记录（Change log）

- 2026-03-29：落地 immutable tag / publish-time `latest` 分离，补齐 pending stable、旧 stable rerun/backfill 与 rc 语义回归，并完成本地 `test-release-snapshot` 验证。

## 参考（References）

- `.github/scripts/release_snapshot.py`
- `.github/scripts/test-release-snapshot.sh`
- `docs/specs/f6f6e-gh-actions-release-anti-cancel/SPEC.md`
