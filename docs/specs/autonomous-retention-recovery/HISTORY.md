# Autonomous Retention Recovery and Raw Capture Circuit Breaker History

> This file records topic lifecycle, compatibility, and necessary background. The current requirements remain in `./SPEC.md`.

## Lifecycle / Compatibility

- This active topic extends, rather than replaces, `9aucy-db-retention-archive`: that topic continues to own the baseline retention tiers, archive/rollup contract, and normal maintenance admission.
- Existing deployments retain per-payload `PROXY_RAW_MAX_BYTES` behavior. The new breaker is an aggregate-store policy and adds an explicit structured omission reason when it suppresses raw capture.
- System-status additions are additive. Existing clients may ignore them without changing proxy or structured invocation behavior.

## Replacements / Background

- The archived retention-backlog work established raw-codec and bounded-catchup behavior, but it did not establish a durable recovery journal, global physical storage budget, filesystem reserve, or autonomous reconciliation of pre-publication artifacts.
- This topic adopts the archive-publication proof and coordinated write-admission boundaries while making recovery progress independently observable and resumable.

## Related Changes

- PR1 delivers autonomous archive recovery and System Status diagnostics on `th/autonomous-retention-recovery`.
- PR2 adds the bounded physical raw inventory and capture circuit breaker as an additive extension to
  the retention metrics ledger; it preserves the existing proxy, archive, and structured-record
  contracts while exposing fail-closed storage health through System Status.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
- `../9aucy-db-retention-archive/SPEC.md`
