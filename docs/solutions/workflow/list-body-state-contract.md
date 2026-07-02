---
title: List bodies own initial loading, error, and empty states
module: frontend
problem_type: ux-consistency
component: list bodies
tags:
  - frontend
  - list-state
  - error-state
  - skeleton
status: active
related_specs:
  - docs/specs/t6d9r-account-detail-stats-read-model/SPEC.md
---

# List Body State Contract

## Context

列表、表格和集合视图如果在首屏无数据时把错误放在列表外、把 loading 放成零散 spinner，用户会看到空白 body 或误判为无结果。账号详情健康与事件 tab 的 400 暴露了这个问题：错误 banner 出现在 tab 外层，但列表 body 没有承接失败状态。

## Resolution

- 首次无已有数据时，列表 body 必须承担三种状态：loading skeleton、initial error、empty success。
- 初始错误必须在 body 内展示，并在可恢复请求上提供 retry。
- 成功但结果为空时展示 empty placeholder，而不是空表格或只留外层说明。
- 已有数据刷新失败时保留旧数据，错误作为 inline/stale warning 呈现，不清空列表 body。
- React 组件优先复用 `ListBodyState`，不要在每个页面散落不同 spinner、alert 和空态 markup。
- 前端布尔 query 必须编码为 `true/false`，不能用 `1/0` 发送给 Rust `bool` / `Option<bool>` query extractor。

## Guardrails

- 列表 body 状态要有稳定 `data-testid` 或明确可访问语义，便于覆盖 loading/error/empty/stale-data refresh error。
- Loading skeleton 应占据列表 body 的稳定尺寸，避免首屏跳动。
- 错误文案和 retry 要在 body 内可见；外层 alert 只适合已有数据刷新失败。
- Empty state 只表示成功空结果；不能用 empty 文案吞掉 request failure。
- 同类 API query 编码需要一起审计，特别是 `include*`、`with*`、`enabled` 这类 bool 参数。

## References

- `web/src/components/ListBodyState.tsx`
- `web/src/components/InvocationTable.tsx`
- `web/src/components/InvocationRecordsTable.tsx`
- `web/src/components/UpstreamAccountsTable.tsx`
- `web/src/components/UpstreamAccountsGroupedRoster.tsx`
- `web/src/pages/system/SystemTasksPage.tsx`
- `web/src/pages/account-pool/Groups.tsx`
- `web/src/pages/account-pool/MaintenanceRecords.tsx`
