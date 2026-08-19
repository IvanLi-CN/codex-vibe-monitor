# 上游账号模型映射 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实。

## Current Status

- Implementation: complete
- Lifecycle: ready for PR merge
- Catalog note: account-local request-side model mapping

## Coverage / rollout summary

- 覆盖账号存储、路由候选、请求改写、尝试观测与路由页签编辑器。
- 不需要外部迁移工具；启动 schema maintenance 负责旧数据库默认列。

## Delivered Coverage

- 已完成后端迁移、匹配与路由缓存、三类上游请求改写、尝试观测字段、详情 API、独立映射保存和路由页签编辑器。
- 已接入 `@dnd-kit/core`、`@dnd-kit/sortable` 和 `@dnd-kit/utilities`，指针拖拽与键盘排序均在 Chrome 演示页验证。
- `cargo test` 通过；前端 Storybook 测试、生产构建和 Storybook 构建通过。全量 Vitest 存在一个既有 `PromptCacheConversationTable` 时序失败，单独复跑该文件为 45/45 通过。
- 桌面与移动视觉证据已获主人确认并登记在 `SPEC.md`。

## Related Changes

- None

## References

- `./SPEC.md`
- `./HISTORY.md`
