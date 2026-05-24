# OAuth 数据面内联合并 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/pd77h-oauth-inline-adapter/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-16: 创建替代 spec，定义用单进程 OAuth adapter 取代固定 sidecar。
- 2026-05-15: 明确 OAuth 凭据可无 refresh token；无 RT 账号跳过自动刷新并在账号列表显示 `无 RT`。
- 2026-05-15: 新增 Web Session 导入入口，把 ChatGPT Web session JSON 转换到现有 Codex OAuth 导入队列。
- 2026-05-16: 放开 OAuth JSON 导入的 `type=codex` 限制；单条本地导入校验改为一次性报告多个字段错误。
- 2026-05-24: 服务端导入验证的既有账号匹配查询改为复用完整账号列清单，防止新增账号字段后手写查询漏列导致整批验证失败。
