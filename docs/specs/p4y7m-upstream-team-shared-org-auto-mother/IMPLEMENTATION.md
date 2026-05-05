# 共享 Team 组织账号去重修正 - Implementation

## Current State

- Canonical spec: `docs/specs/p4y7m-upstream-team-shared-org-auto-mother/SPEC.md`
- Implementation summary: See companion notes and linked PR/check history for implementation context.

## Migrated Implementation Notes

## Validation

- `cargo test same_group_team_shared_org_accounts_are_not_flagged_as_duplicates -- --test-threads=1`
- `cargo test same_group_team_shared_org_accounts_keep_manual_mother_only -- --test-threads=1`
- `cd web && bun run test -- src/components/UpstreamAccountsTable.test.tsx`
- `cd web && bun run build-storybook`
