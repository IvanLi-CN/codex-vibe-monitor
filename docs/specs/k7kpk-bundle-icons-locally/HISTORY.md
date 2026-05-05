# 前端运行时图标内置打包 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/k7kpk-bundle-icons-locally/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录

- 2026-03-14: 创建 spec，冻结“运行时图标必须内置打包、禁止第三方拉取”的范围与验收口径。
- 2026-03-14: 新增 `AppIcon` 本地图标注册层，按图标导入 `@iconify-icons/mdi` 并替换全部运行时代码中的 `mdi:*` 字符串调用。
- 2026-03-14: 完成 `cd web && bun run lint`、`cd web && bun run test`、`cd web && bun run build`，并通过 Playwright 预览页检查 `#/dashboard`、`#/records`、`#/settings`、`#/account-pool` 无第三方图标请求。
- 2026-03-14: 已推送分支 `th/k7kpk-bundle-icons-locally`；PR 创建受阻于 GitHub MCP 握手失败，暂未完成 M4。
