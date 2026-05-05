# 号池分组设置弹窗“绑定代理节点”目录加载与同步热修 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/7gb5w-account-pool-bound-proxy-dialog-freshness/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-12: 初始化 hotfix spec，锁定“未加载不等于空目录”“settings 保存后同会话自动同步”“Storybook 先行验收”三条主约束。
- 2026-04-12: 完成 hook tri-state、dialog loading 占位、弹窗 stale silent refresh、settings 保存失效通知与 targeted Vitest/build 验证。
- 2026-04-12: 根据 review-loop 收敛自动 silent refresh 的失败自旋问题，抽出共享 auto-refresh hook，并补回归测试与 Storybook 视觉证据。
- 2026-04-12: 主人批准截图随分支一起提交；分支已推送并创建 PR #335，当前停在 merge-ready。
