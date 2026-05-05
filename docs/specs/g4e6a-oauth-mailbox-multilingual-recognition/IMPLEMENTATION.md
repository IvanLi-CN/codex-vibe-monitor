# OAuth 邮件多语言验证码与邀请识别 - Implementation

## Current State

- Canonical spec: `docs/specs/g4e6a-oauth-mailbox-multilingual-recognition/SPEC.md`
- Implementation summary: 进行中

## Migrated Implementation Notes

## 状态

- Status: 进行中
- Created: 2026-03-24
- Last: 2026-03-24

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test parse_mailbox -- --nocapture`
- `cargo test normalize_mailbox_text -- --nocapture`
- `cargo check`
- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增索引并在 PR / merge 事实确定后同步状态与备注
