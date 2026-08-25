# Runtime Read-Model Pressure Recovery - Implementation

> Canonical spec: `./SPEC.md`.

## Current Status

- Lifecycle: active Initiative Bootstrap.
- Implementation: no runtime implementation is included in this Bootstrap.
- Promotion policy: checkpointed; every included Ticket requires observed evidence after owner-confirmed manual deployment.

## Delivery Boundaries

| Delivery slice                      | Purpose                                                                                                    | Integration order                            | Completion evidence                                                                                    |
| ----------------------------------- | ---------------------------------------------------------------------------------------------------------- | -------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Summary archive hydration           | Recover exact Summary Projection from durable rollup plus exact boundary records without request-time I/O. | Wave 1                                       | child/integration CI, checkpoint release, owner-confirmed deployment, 900-second read-only observation |
| Pressure defer and startup backfill | Make pressure-gated background work event/deadline driven and distinguish it from real lock failure.       | Wave 1                                       | child/integration CI, checkpoint release, owner-confirmed deployment, 900-second read-only observation |
| Long-term legacy migration          | Replace legacy full-window migration scans with cursor/seek microtransactions.                             | Wave 2, after the pressure slice is observed | child/integration CI, checkpoint release, owner-confirmed deployment, 900-second read-only observation |

## Integration Rules

- The integration branch is `prd/runtime-read-model-pressure-recovery`; child PRs target it and do not release directly.
- Wave 1 may implement independently, but child integration is serialized at the shared frontier.
- The long-term slice cannot start until the pressure slice reaches its `observed` completion gate.
- Existing timeseries writer work remains outside this Initiative. Draft #714 stays out of the integration branch and is only marked superseded after the successor long-term slice has passed its technical acceptance.
- Checkpoints publish GitHub artifacts only. Dockrev and srv-101 deployment, restart, rollback and write operations remain owner-only.

## Verification Ownership

- Summary: exact-response comparison, >legacy-cardinality archive coverage, HTTP SQL/file counters, freshness and last-good behavior.
- Pressure: one defer wake per cooldown, no no-op task-run write, real-lock backoff separation and scheduler recovery.
- Long-term: query-plan assertion, cursor persistence, 512-row transaction cap, pressure/cancel recovery and P1 priority.
- Observation: `$srv-101-ops` is read-only and only starts after the owner confirms the exact released version is deployed.
