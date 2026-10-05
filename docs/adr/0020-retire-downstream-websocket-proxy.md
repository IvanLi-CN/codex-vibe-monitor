# ADR 0020: Retire downstream WebSocket proxy support

## Status

Accepted

## Date

2026-09-28

## Context

The proxy currently accepts WebSocket upgrades under `/v1/*`, routes them to
upstream `ws`/`wss` endpoints, relays frames, records per-turn usage, and
maintains WebSocket-specific settings, account capability tags, tests, and
dependencies. The capability is not in use and has no near-term product
requirement. Keeping the live path creates a separate transport boundary whose
routing, failover, accounting, UI, and compatibility behavior must continue to
be maintained alongside HTTP.

The existing implementation also left durable state behind: two global
settings columns, a one-time initialization marker, the
`unsupported_transport:websocket` system tag, and historical invocation data
with `transport = "websocket"`. Removing the live capability must not erase
historical observability or make a database migration depend on deleting
unrelated state.

## Decision

### Retire the live transport boundary

- Remove downstream WebSocket proxy support for all `/v1/*` paths, including
  the upstream WebSocket dialer and frame relay.
- A request carrying a WebSocket upgrade is rejected before authentication,
  account routing, upstream selection, invocation persistence, or retry
  handling. The ordinary HTTP path is unchanged.
- The rejection uses the existing JSON error envelope with HTTP `501`:

  ```json
  {"error":"WebSocket proxy support has been removed","code":"websocket_proxy_removed"}
  ```

- The rejection does not create `cvmId`, `x-cvm-invoke-id`, an Invocation, or
  an Upstream Attempt. It emits only sanitized structured rejection telemetry;
  payloads, API keys, and account identifiers are excluded.
- There is no fallback from a WebSocket upgrade to an ordinary HTTP request.

### Remove live configuration and implementation surfaces

- Remove the downstream and upstream WebSocket settings from the Settings API,
  Settings UI, TypeScript contracts, environment configuration, and runtime
  initialization. Older update requests may contain these fields and must be
  ignored; new responses omit them.
- Remove the WebSocket relay module, Axum WebSocket feature, direct
  `tokio-tungstenite`/`tungstenite` dependencies, and WebSocket-only routing,
  failover, capability, usage-refresh, and test paths.
- Retain shared HTTP socket byte metering, generic network statistics, and
  other transport-neutral infrastructure.
- Remove creation, capability learning, startup ensuring, and UI surfacing of
  the `unsupported_transport:websocket` / `不支持 WS` system tag.

### Preserve durable history and migrate legacy state

- Keep the two legacy WebSocket settings columns and
  `websocket_settings_migrated` as inert compatibility state. They are not a
  source of runtime capability truth and are not dropped in this release.
- Keep historical invocation/archive rows with `transport = "websocket"`
  readable, filterable, and serializable. Historical UI labels use
  `WebSocket（历史）`; no new WebSocket rows, refreshes, or backfills are
  produced.
- Add one named `schema_refresh_migrations` migration,
  `retire_openai_websocket_proxy_v1`, executed in one transaction:
  1. Set both legacy WebSocket settings to `false` and mark the legacy
     initialization flag complete.
  2. Delete account associations for the exact WebSocket unsupported system
     tag.
  3. Delete that system tag row.
  4. Record the migration completion marker.
- The migration is idempotent and re-entry-safe. Failure rolls back the whole
  transaction. It reports affected-row counts without exposing account data.
  No unrelated tag, historical invocation, archive, or network-stat row is
  modified.
- The old startup settings initializer and system-tag ensure path are removed,
  so the retired state cannot be recreated by the new binary.

### Rollback and compatibility

- The migration is forward-only. Program rollback does not automatically
  reverse the persisted state or restore the retired contract.
- An earlier binary may still start against the retained schema, but WebSocket
  behavior is not guaranteed to return automatically. Explicit re-enablement
  through the earlier binary or restoration from a pre-migration database
  backup is required if an emergency recovery ever needs the old capability.
- The release records `public_api = major` because a public transport and its
  settings/configuration surface are removed. It records
  `persistent_state = minor` because the existing columns and historical rows
  remain readable while the migration disables live use. The release unit's
  maximum impact is `major`.

## Alternatives considered

- **Remove only the upstream WebSocket dialer.** Rejected because the
  downstream upgrade contract, settings, route handling, and client-facing
  ambiguity would remain.
- **Keep the route but return the existing disabled error.** Rejected because
  it leaves the full transport implementation and configuration burden in the
  product while presenting a capability that is intentionally retired.
- **Drop the legacy columns and delete historical WebSocket records.**
  Rejected because it adds unnecessary destructive migration risk and removes
  useful historical evidence.
- **Keep historical WebSocket labels unchanged.** Rejected because the UI
  would imply that WebSocket is still a live transport.

## Consequences

- `/v1/*` has one live proxy transport, HTTP; clients that need WebSocket must
  use a separately supported endpoint or direct upstream connection.
- The proxy no longer needs WebSocket-specific routing, failover, accounting,
  dependency, and settings maintenance.
- Legacy schema columns remain intentional inert debt and can be considered by
  a later, separately reviewed cleanup.
- Historical records remain available for audits and comparisons, but they are
  clearly separated from current transport capability.

## Implementation acceptance

- Every `/v1/*` WebSocket upgrade receives the exact `501` JSON contract and
  does not create an invocation, attempt, or upstream connection.
- Ordinary HTTP requests, HTTP streaming, shared socket metering, generic
  network statistics, and historical WebSocket record reads remain functional.
- Settings responses no longer expose the retired fields, old update payloads
  are tolerated, and the two legacy database columns remain present.
- The named migration passes on a pre-migration database, is safe to rerun,
  deletes only the exact WebSocket system tag and its associations, and leaves
  historical records untouched.
- No new runtime or UI path can create WebSocket proxy records or restore the
  retired system tag.

## Related Specs

- [Retired WebSocket proxy topic](../specs/w5s2x-openai-websocket-proxy/SPEC.md)
