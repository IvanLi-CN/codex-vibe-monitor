# 上游账号分组持久化目录（#j5x9m）

## 背景

- `#thyxm` 把分组共享设置定义成“依附账号的前端草稿”，只有首个账号真正落入该分组后才允许持久化。
- 这会导致新分组在未先建账号前无法真正配置，也会让最后一个账号移走后分组从下拉列表消失，用户无法继续复用或维护该分组。
- 当前交互要求改成“先配置后创建”：新分组必须先完成设置并立即进入 catalog，之后所有写路径才能把该分组真正写回账号表单或批量弹窗。

## 目标

- 把分组提升为可持久化的 catalog 资源：保存分组设置后立即落库，`0` 账号分组仍保留在分组下拉里。
- 所有新分组创建入口统一改成“选择 create -> 自动弹配置对话框 -> 保存成功后才回填 groupName”。
- 分组列表统一返回 `accountCount`，下拉中展示“分组名 + 当前账号数”。
- 分组设置对话框支持“左下角删除空分组、右侧取消/保存”，并统一使用图标 + 文字按钮。

## 非目标

- 不做分组重命名。
- 不做删除非空分组时的自动迁移成员。
- 不把空分组额外渲染进 grouped roster 卡片区。

## 功能规格

### 数据与接口

- `GET /api/pool/upstream-accounts` 的 `groups[]` 必须返回“账号实际分组 ∪ 已保存分组元数据”的并集。
- `UpstreamAccountGroupSummary` 新增 `accountCount`，它始终表示后端全量成员数，不受当前分页、搜索或筛选影响。
- `PUT /api/pool/upstream-account-groups/:groupName` 改为 upsert catalog：空分组也允许保存并立即可见。
- 新增 `DELETE /api/pool/upstream-account-groups/:groupName`：仅允许删除 `accountCount=0` 的空分组；非空时返回冲突与成员数提示。
- 停止自动清理已保存空分组；只有历史上从未保存过的临时脏分组才允许自然消失。

### 前端交互

- `UpstreamAccountGroupCombobox` 必须支持结构化 option，至少包含 `groupName`、`accountCount` 与 `isPersisted`。
- create option 不得直接写入表单值；只能通过 `onCreateRequested` 打开分组设置对话框。
- 创建页、详情编辑、批量改分组三类写路径都必须记录原值；若新分组保存成功则回填新值，若取消则回退原值。
- 分组设置对话框在 persisted group 上显示删除按钮；当 `accountCount > 0` 时按钮禁用并显示“先移走 N 个账号”的提示。

## 验收标准

- Given 任一写路径输入一个不存在的分组，When 选择 create 选项，Then 立即弹出分组设置对话框，且当前字段不会先写入该新值。
- Given 新分组在对话框中保存成功，When 不创建任何账号直接刷新页面，Then 该分组仍出现在所有分组下拉里，且 `accountCount=0`。
- Given 已保存分组当前仍有成员，When 打开分组设置对话框，Then 左下角删除按钮可见但禁用，并显示剩余成员数提示。
- Given 已保存分组当前没有成员，When 点击删除，Then 删除成功且该分组从 dropdown catalog 中消失。

## 关联契约

- 主契约：`/Users/ivan/.codex/worktrees/1f23/codex-vibe-monitor/docs/specs/g4ek6-account-pool-upstream-accounts/contracts/http-apis.md`
- 被替换假设：`/Users/ivan/.codex/worktrees/1f23/codex-vibe-monitor/docs/specs/thyxm-upstream-account-group-notes/SPEC.md`

## Visual Evidence

- 待本次实现的 Storybook 证据与 owner-facing 截图一并补充到主 spec 的 `## Visual Evidence`。
