# Storybook Accessibility Gate - Implementation

## Current State

- Canonical spec: `docs/specs/sbacc-storybook-accessibility/SPEC.md`
- Implementation summary: Implemented, pending PR CI convergence

## Migrated Implementation Notes

## Status

- Spec ID: `sbacc`
- Status: Implemented, pending PR CI convergence
- Created: 2026-03-13
- Updated: 2026-04-26

## Implementation

- `web/vitest.config.ts` defines separate `unit` and `storybook` Vitest projects.
- `web/.storybook/main.ts` registers `@storybook/addon-vitest`.
- `web/.storybook/preview.ts` enables addon-a11y `test: 'error'`, disables `color-contrast`, and sets default `!test` tags for legacy stories.
- `web/src/components/AccessibilityGate.stories.tsx` is the opt-in `test` story used by CI.
- `.github/workflows/ci-pr.yml` and `.github/workflows/ci-main.yml` run `bun run test-storybook` inside `Front-end Tests` after the unit suite.
