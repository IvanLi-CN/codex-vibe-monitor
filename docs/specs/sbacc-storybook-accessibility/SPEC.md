# Storybook Accessibility Gate

## Context

PR #123 originally added a Storybook accessibility gate, but its branch drifted behind the current split CI topology and conflicted with the retired single-workflow layout. The branch is revived on top of the current `main` baseline with the gate wired into the active PR and main CI workflows.

Current Storybook contains many legacy interaction stories and known light-theme color contrast debt. The CI gate therefore uses an explicit opt-in Storybook test surface instead of auto-running every existing story. This keeps the gate deterministic while preserving Storybook build coverage for the full catalog.

## Requirements

- Storybook accessibility checks run through Vitest browser mode with Chromium.
- CI executes the Storybook accessibility suite as part of the existing `Front-end Tests` check so branch protection does not require a new status context rollout.
- Existing non-test stories are excluded from Vitest component execution by default.
- The opt-in accessibility fixture must fail CI on axe semantic violations.
- Color contrast remains disabled in the axe run until the tracked palette contrast debt is handled separately.

## Verification

- `cd web && bun run test-storybook`
- `cd web && bun run test`
- `cd web && bun run lint`
- `cd web && bun run build`
- `cd web && bun run storybook:build -- --quiet`
- `python3 .github/scripts/check_quality_gates_contract.py --repo-root "$PWD" --declaration .github/quality-gates.json --metadata-script .github/scripts/metadata_gate.py`
- `bash .github/scripts/test-quality-gates-contract.sh`
- `bash .github/scripts/test-live-quality-gates.sh`
