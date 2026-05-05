# 反向代理 Fast 模式请求改写（三态设置，`requestedServiceTier`=上游实际请求值） - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/dvwja-proxy-fast-mode-request-rewrite/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-09: 创建规格，冻结三态 Fast rewrite 语义、双接口范围与 `requestedServiceTier` 最终请求值口径。
- 2026-03-09: 完成 SQLite 设置迁移、双接口 tier 改写与 `requestedServiceTier` 最终值回写，补齐 Settings UI、Storybook mock、Vitest 与 Playwright 覆盖，并通过 `cargo test`、`cargo check`、`cd web && npm run test`、`cd web && npm run build`。
- 2026-03-09: 根据 review 调整 `disabled` 模式为真正透明透传；仅在 `fill_missing` / `force_priority` 生效时才标准化 `service_tier` 字段形状，并补充对应回归测试。
- 2026-03-09: 创建 PR #102，补齐 release labels，并在变基到最新 `main` 后确认本地验证与 GitHub Actions checks 全部通过。
- 2026-04-05: 补充 pool fast hotfix 不变量：body rewrite 后必须丢弃 stale `Content-Length`，且 send-stage transport failure 仍需保留最终出站 request raw 与 `requestedServiceTier`。
