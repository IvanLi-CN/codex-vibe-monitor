# Implementation

## Coverage

- Backend schema/API/runtime:
  - group metadata gained `single_account_rotation_enabled`.
  - account selection now prefers reset-time proximity before older tie-breakers.
  - `429` recording clears sticky route only when this strategy is enabled.
  - quota snapshot/rate-limit inference does not rotate existing sticky conversations.
  - hard auth failures leave sticky records intact but route new requests to other accounts.
- Frontend:
  - group settings dialog exposes the switch.
  - create/import/batch paths preserve the field in draft state and save payloads.
- Tests:
  - backend route/failover, resolver, and selection tests added.
  - dialog/api normalization tests added.
  - Storybook story/play coverage and visual evidence added.

## Gaps

- None.
