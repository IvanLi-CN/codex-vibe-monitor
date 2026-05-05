# 47ran Implementation

- 恢复 `GET /api/settings` 的 `proxy` 返回与 `PUT /api/settings/proxy` 路由接线。
- 在当前 pool `/v1/models` 分支内重新接入 hijack / merge / fallback；未恢复任何非 pool 直连路径。
- 默认 preset 扩展到 `gpt-5.5` / `gpt-5.5-pro`，默认 pricing 升级到 `openai-standard-2026-04-25` 并补齐 `gpt-5.4-mini`。
- Settings 页、API client、hook、Storybook mock 与文案一起恢复到当前 truth。
