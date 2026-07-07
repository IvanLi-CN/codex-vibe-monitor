# 受控手动发版覆盖 - History

- Canonical spec: `docs/specs/m7q2r-manual-release-override/SPEC.md`

## Decisions

- Manual override snapshot 使用 job-local JSON 文件，不写回 `refs/notes/release-snapshots`，避免污染自动 PR intent 的 immutable snapshot。
- 不带 `version`、`bump`、`reason` 的 `workflow_dispatch` 保留为内部 release queue/backfill 兼容路径；提供任一手动覆盖输入时才启用严格 manual override 校验。
- `channel=rc` 沿用自动发版的 `vX.Y.Z-rc.<sha7>` tag 形态，且永不更新 `latest`。
- 修改受控 workflow contract 时，Label Gate 必须具备同仓 current-branch self-validation 路径；否则 `main` 上的旧 checker 会把新拓扑误判为 drift。
