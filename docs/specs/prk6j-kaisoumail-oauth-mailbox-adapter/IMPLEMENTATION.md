# KaisouMail OAuth 邮箱适配 - Implementation

## Current State

- Canonical spec: `docs/specs/prk6j-kaisoumail-oauth-mailbox-adapter/SPEC.md`
- Status: 已实现并通过本地验证

## Implementation Notes

- 后端 mailbox client 已切换到 KaisouMail Bearer API。
- 项目内 OAuth mailbox session API 保持兼容。
- 手动地址仍走非阻塞降级：非法格式、unsupported domain 或不可读时不阻断 OAuth 主流程。
- `generated` 会话保存 KaisouMail mailbox `id` 并执行远端删除；`attached` 会话只清理本地记录。

## Quality Gates

- `cargo fmt`
- `cargo check`
- `cargo test mailbox`
- `cd web && bun run test -- UpstreamAccountCreate`

## Visual Evidence

Not applicable. 本次不改变可见 UI 结构，仅同步服务端适配与文案。
