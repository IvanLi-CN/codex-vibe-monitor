# OAuth Luna Reserve fallback implementation

## Current Candidate

The current candidate implements the Reserve snapshot, additive SQLite fields, quota-route selection, bounded OAuth fallback, request observability, pool-routing settings, account-list/detail display, and formal Web App Demo fixtures.

## Coverage

- Rust usage parsing and persistence live in `src/upstream_accounts/sync_usage_snapshot.rs`, `src/upstream_accounts/core_models_rows.rs`, and `src/upstream_accounts/core_schema_maintenance.rs`.
- Candidate selection and failover live in `src/upstream_accounts/routing/selection.rs`, `src/upstream_accounts/routing/candidate_loading.rs`, and `src/proxy/failover/`.
- Settings and API response types live in `src/upstream_accounts/routing/settings_runtime.rs`, `src/upstream_accounts/core_runtime_types.rs`, and the related API slices. The settings contract includes `automaticReserveFallbackEnabled` and the bounded `reserveFallbackModelPolicy` mode/patterns.
- Web list/detail usage cards and the fallback switch live under `web/src/features/account-pool/`, `web/src/features/settings/`, and `web/src/pages/`.
- The mock-only Web App Demo covers the grouped account list, account detail Reserve card, settings toggle, and Reserve model policy mode/pattern controls at the requested browser viewport.

## Compatibility

- Existing SQLite databases are upgraded additively.
- Existing usage snapshots remain readable when Reserve data is absent.
- Normal quota routing and its statistics keep the existing behavior.
- Reserve state is independent from the automatic fallback switch.

## Validation

Local candidate validation is recorded in the topic Spec. PR-stage required checks and formal review remain delivery gates; the migration and version-impact records under the related ADR remain pending until those gates complete.

## References

- `./SPEC.md`
- `./HISTORY.md`
- `../../adr/0051-oauth-luna-reserve-fallback.md`
