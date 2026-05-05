# OAuth 邮件多语言验证码与邀请识别 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/g4e6a-oauth-mailbox-multilingual-recognition/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-24: 新建 follow-up spec，冻结 OAuth 邮件多语言验证码与邀请识别的后端解析边界、误判门禁与质量门槛。
- 2026-03-24: 完成后端解析升级与 Rust 回归测试补齐，等待 fast-track 的本地全量验证、PR 收敛与 merge cleanup。
- 2026-03-24: 本地 `cargo check`、`cargo test`、`cd web && bun run test` 与 `cd web && bun run build` 已通过，进入 fast-track 的 PR / review / merge 收口阶段。
- 2026-03-24: 创建 PR #215，并根据 review 收紧 invite CTA 链接判定，避免把普通 workspace 页面误存成邀请链接。
- 2026-03-24: 根据后续 review 继续收紧 body-only invite 判定，要求 workspace 语义与真实邀请 CTA 同时成立，并排除帮助文档类链接。
- 2026-03-24: 根据后续 review 恢复 query 型邀请 CTA 识别（如 `?invite=` / `?accept=`），避免误伤合法邀请链接。
- 2026-03-24: 根据后续 review 收紧验证码候选方向性，只接受位于验证码语义之后的数字候选，并放宽 body-only invite 对 `workspace` 文本的额外依赖。
- 2026-03-24: 根据最终 review 收紧 subject 弱验证码匹配；subject 路径不再借用正文全局品牌词，仅接受 subject 本地 OpenAI/ChatGPT 上下文。
- 2026-03-24: 为 batch OAuth 页面补齐慢测超时，确保 `cd web && bun run test` 在完整套件下稳定通过，作为 fast-track 合并门槛的一部分。
- 2026-03-24: 根据终轮 review 进一步收紧 subject-only 验证码门槛为“subject 自带品牌语义”，并支持从 redirect CTA 中解析出真实 ChatGPT/OpenAI invite 目标链接。
