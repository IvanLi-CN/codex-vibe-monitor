# 请求记录筛选下拉遮挡修复 - Implementation

## Current State

- Canonical spec: `docs/specs/8pjnh-records-filter-dropdown-overlap-fix/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-03-12
- Last: 2026-03-12

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `cd web && bun run test -- src/pages/Records.test.tsx`
- E2E tests: `cd web && bun run test:e2e -- records-filter-overlay.spec.ts`
- PR CI gate: `.github/workflows/ci.yml` runs `Front-end Tests` and `Records Overlay E2E`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增 hotfix spec 索引并同步状态/备注。
- `docs/specs/8pjnh-records-filter-dropdown-overlap-fix/SPEC.md`: 持续记录实现、验证与 PR 视觉证据。
