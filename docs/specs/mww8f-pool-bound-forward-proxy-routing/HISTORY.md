# 移除直连反向代理并为号池接入分组绑定正向代理 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/mww8f-pool-bound-forward-proxy-routing/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录

- 2026-03-26: 创建 spec，冻结 `/v1/*` 新语义、分组绑定 forward proxy 的运行时规则、接口契约与视觉证据目标。
- 2026-03-27: 视觉证据完成主人确认，spec 状态切换为已完成，并标记 PR 可复用截图。
- 2026-03-27: 增补线上 follow-up：分组绑定弹窗改为协议标签展示 + 独立滚动布局，并在分组绑定路径恢复显式 `Direct` 选项；补齐桌面宽度约束与高密度卡片布局后，重新生成并批准复用 Storybook 证据。
- 2026-03-28: 补充稳定节点身份键、非 ASCII 展示恢复与“历史旧 key 不自动迁移”的接口契约，并为分组设置弹窗追加对应测试与 Storybook 场景。
- 2026-03-29: 补充 stable-key 语义修正后的 runtime/history alias 兼容要求，并明确删除分组时 `404` 必须优先于绑定校验错误。
- 2026-04-01: 绑定键语义从运行时稳定 identity 调整为名称驱动的逻辑键（`fpb_*`），旧 `fpn_*` / legacy key 改为通过 metadata history 与当前 inventory canonicalize 到逻辑绑定键，避免订阅刷新后同名节点因 transport identity 漂移而失效。
- 2026-03-28: 补充 `vless/trojan` 稳定键回归的 follow-up：保存时拒绝“当前绑定集合零可选节点”的坏状态，并新增 Storybook 证据覆盖 warning + 保存按钮禁用场景。
- 2026-04-01: 刷新 `Automatic Routing` owner-facing 视觉证据，使其与当前共享分组设置弹窗的并发/429/绑定节点复合布局保持一致。
