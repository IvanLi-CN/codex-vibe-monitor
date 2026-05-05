# 后端 prompt-cache conversations 结构收敛 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/n78zb-backend-prompt-cache-conversations-structure/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-12: 创建 prompt-cache conversations 结构收敛 follow-up spec，冻结 fast-flow / merge+cleanup / prompt-cache-only 范围。
- 2026-04-12: 完成 `prompt_cache_conversations` 真模块拆分；本地 `cargo fmt/check/test` 通过，期间命中过一次既有代理热路径时间敏感单测 `proxy_openai_v1_chunked_json_without_header_sticky_uses_live_first_attempt`，单测复跑与整套复跑均通过；shared-testbox `api-read-smoke` 全绿。
- 2026-04-12: PR #339 review proof clear，GitHub checks 全绿，merge + cleanup 完成，本地回到最新 `main`。
