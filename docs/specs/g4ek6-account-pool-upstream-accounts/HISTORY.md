# 号池模块第一阶段：上游账号管理 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/g4ek6-account-pool-upstream-accounts/SPEC.md`

## Migrated History Notes

## Change log

- 2026-04-23：把上游账号分组升级为可持久化 catalog，保存分组设置后立即落库并保留 `0` 账号分组；所有新分组创建入口改为先弹分组设置对话框、保存成功后才回填 `groupName`，分组下拉同步展示全量 `accountCount`，且分组设置对话框统一为“左下角删除空分组 / 右侧取消与保存”的图标+文字 CTA。
- 2026-03-16：补充账号详情抽屉的异步一致性约束，明确账号级 busy state 与 action error 都要按账号隔离、同一账号任一写操作进行中时其它写入口必须锁住、账号切换要在同一交互拍内使旧 detail 请求失效、保存/同步成功要先失效旧 detail reload、refresh 必须用列表数据纯计算最终选中账号后再刷新 detail 且列表失败时不得清空当前 detail、hook 级 list/detail 错误必须按来源隔离、同类动作跨账号并发时不得互相覆盖 busy/error 态、晚到 detail 成功/失败响应与 sync 响应都要按当前选中账号过滤，以及同步按钮 idle 态改用 outline 图标。
- 2026-03-20：补充账号级 actor 串行与后台维护去重约束，明确维护只允许阻塞同一账号、无关账号启停在维护竞争下需以 `1 秒内完成服务端提交` 为目标，并新增对应的 Rust 并发回归测试要求。
- 2026-04-01：将号池活跃 sticky 共享窗口从 30 分钟统一收敛为 5 分钟，`workStatus=working` 与 `activeConversationCount` 继续共用同一时间口径，且不引入新的 API、schema 或配置项。
- 2026-03-23：把上游账号列表的混合 `displayStatus` 读模型拆成 `workStatus` / `enableStatus` / `healthStatus` / `syncState` 四个维度，列表筛选同步拆成 `工作状态`、`启用状态`、`账号状态` 三组服务端交集筛选，并锁定“不新增持久化状态列”的实现边界。
- 2026-04-09：补充详情抽屉编辑会话的草稿冻结约束，明确同账号静默 refresh / detail reload / SSE open-resync 在草稿 pristine 时仍需跟随最新详情、在用户产生未提交修改后不得覆盖当前输入；保存成功只允许在仍属于当前活跃编辑会话且当前草稿仍等于发起保存时快照时重播种，关闭抽屉或切到别的账号后回到原账号都视为新会话，旧会话或较早保存的晚到回包不得覆盖更新后的草稿。

## Migrated Change History

## 变更记录（Change log）

- 2026-04-23: 把上游账号分组升级为可持久化 catalog，保存分组设置后立即落库并保留 `0` 账号分组；所有新分组创建入口改为先弹分组设置对话框、保存成功后才回填 `groupName`，分组下拉同步展示全量 `accountCount`，且分组设置对话框统一为“左下角删除空分组 / 右侧取消与保存”的图标+文字 CTA。
- 2026-03-11: 创建 spec，冻结账号管理第一阶段的范围、接口、状态机与验收口径。
- 2026-03-11: 完成后端账号管理 / OAuth 会话 / 前端号池页面实现，并通过 Rust + Web 自动化验证与本地浏览器 smoke。
- 2026-03-13: 扩展上游账号创建页为单账号 OAuth / 批量 OAuth / API Key 同页模式，并将批量 OAuth 表格纳入现有手动 OAuth 流程。
- 2026-03-13: 刷新 Storybook 视觉证据，补充路由设置弹窗、Sticky Key 对话与记录页上游筛选展示。
- 2026-03-14: 调整 OAuth 新建语义为“重复身份仅告警不合并”，并补充 `displayName` 全局唯一约束与 UI warning/inline error 验收口径。
- 2026-03-18: 账号列表头部改为 `分组 + 多 Tag` 双筛选，Tag 必须全匹配；移除头部 `打开详情` 冗余按钮并保持列表行点击/路由态打开详情抽屉的承接语义。
- 2026-03-25: 收敛母号 badge 与“设为母号”切换卡的 amber 对比度，补充独立 Storybook 画廊与 dark 详情抽屉证据，避免母号标记继续混入低可读 warning 文本配色。
- 2026-03-26: 批量 OAuth 完成态继续开放元数据编辑，默认分组与共享标签会继续联动已落库账号，同时锁定邮箱与 OAuth 身份控件只读。
