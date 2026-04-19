# 共享 Team 组织账号去重修正（#p4y7m）

## Context

- 现网号池会把同一 `groupName` 下、同一 Team 组织 (`chatgptAccountId`) 的成员账号误标成“重复账号”，即使这些账号实际属于同一个 Team，而不是重复导入的同一凭据。
- `0414-3` 这类分组里，同一 Team 组织下的多个成员账号已经具备稳定特征：`planType=team`、共享 `chatgptAccountId`、共享 `groupName`，但 `chatgptUserId` 不同。
- 现有字段不足以可靠判断谁是“母号”；如果没有人工标记，系统必须诚实地保留“未知”，而不是猜一个默认母号。

## Goals

- 同组、同 Team 组织、不同成员用户的 OAuth 账号不再返回 `duplicateInfo`。
- 已有手动母号标记时，roster / detail 继续按手动结果展示。
- 没有手动母号标记时，不做自动母号识别。

## Non-Goals

- 不改动 mixed-plan duplicate 语义。
- 不新增新的 duplicate reason 或新的持久化字段。
- 不对跨组共享 Team 组织账号做自动消歧。
- 不基于创建时间、排序顺序或其他弱信号猜测母号。

## Decision

- 对 `sharedChatgptAccountId` 簇新增“共享 Team 组织成员”识别：
  - `planType=team`
  - `groupName` 非空且一致
  - `chatgptUserId` 非空且成员之间不同
- 命中上述条件的账号对不再视为重复账号。
- 母号展示规则：
  - 仅使用已持久化的 `isMother=true`
  - 若没有手动母号，则保持非母号展示，并明确当前数据无法可靠自动识别

## Acceptance Criteria

- Given 两个 OAuth 账号位于同一分组，且共享 `chatgptAccountId`、`planType=team`、`chatgptUserId` 不同，When 读取 roster / detail，Then 两边都不显示“重复账号”。
- Given 同一 Team 组织簇没有手动母号，When 读取 roster / detail，Then 两边都不显示“母号”。
- Given 同一 Team 组织簇已有手动母号，When 读取 roster / detail，Then 继续显示手动母号，不产生额外自动母号。

## Validation

- `cargo test same_group_team_shared_org_accounts_are_not_flagged_as_duplicates -- --test-threads=1`
- `cargo test same_group_team_shared_org_accounts_keep_manual_mother_only -- --test-threads=1`
- `cd web && bun run test -- src/components/UpstreamAccountsTable.test.tsx`
- `cd web && bun run build-storybook`

## Visual Evidence

- source_type: storybook_canvas
  story_id_or_title: Account Pool/Pages/Upstream Accounts/List/Team Shared Org Coexistence
  state: roster
  evidence_note: 验证共享 Team 组织场景下，列表中的成员账号不再显示“重复账号”，也不会凭空出现“母号”。
  ![共享 Team 组织列表不再显示重复账号](./assets/team-shared-org-roster.png)

- source_type: storybook_canvas
  story_id_or_title: Account Pool/Pages/Upstream Accounts/List/Team Shared Org Coexistence
  state: detail drawer
  evidence_note: 验证详情抽屉保留 Team 语义，但不再出现 duplicate warning、matched reasons 或自动母号。
  ![共享 Team 组织详情抽屉不再自动标记母号](./assets/team-shared-org-detail.png)
