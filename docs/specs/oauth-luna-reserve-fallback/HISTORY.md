# OAuth Luna Reserve fallback history

## Lifecycle / Compatibility

- This topic adds a typed representation for an upstream Reserve capability while retaining compatibility with older SQLite databases and usage snapshots.
- The automatic fallback switch is default-on for OAuth accounts, but it is independent from usage synchronization and visibility.
- Reserve failures remain routing failures unless the upstream provides an explicit exhaustion signal or a later snapshot confirms the state.

## Decision Background

- The terminology and routing boundary are defined by [ADR 0051](../../adr/0051-oauth-luna-reserve-fallback.md).
- Normal `gpt-*-luna` model names are not Reserve aliases; `gpt-reserve` is the Reserve wire model.

## Delivery

- The current implementation candidate is being delivered through the `th/oauth-luna-reserve-fallback` topic branch.
- PR, merge, and release references are added after the corresponding live facts exist.
