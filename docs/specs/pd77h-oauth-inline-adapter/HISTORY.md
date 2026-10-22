# OAuth 数据面内联合并 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/pd77h-oauth-inline-adapter/SPEC.md`

## Key Decisions

- sub2api 导入范围限定为 OpenAI OAuth；API Key 独立中转导入不属于该兼容修复。缺少 ID token 时沿用 Session 已有身份适配方式，不为格式兼容改变持久化模型，也不把合成载体当作上游签发的身份凭据。

## Migrated Change History

## 变更记录（Change log）

- 2026-03-16: 创建替代 spec，定义用单进程 OAuth adapter 取代固定 sidecar。
- 2026-05-15: 明确 OAuth 凭据可无 refresh token；无 RT 账号跳过自动刷新并在账号列表显示 `无 RT`。
- 2026-05-15: 新增 Web Session 导入入口，把 ChatGPT Web session JSON 转换到现有 Codex OAuth 导入队列。
- 2026-05-16: 放开 OAuth JSON 导入的 `type=codex` 限制；单条本地导入校验改为一次性报告多个字段错误。
- 2026-05-24: 服务端导入验证的既有账号匹配查询改为复用完整账号列清单，防止新增账号字段后手写查询漏列导致整批验证失败。
- 2026-10-22: sub2api 导入补充跨字段身份冲突拒绝、错误类型声明容错，以及共享账号下不同成员的安全匹配回归。
