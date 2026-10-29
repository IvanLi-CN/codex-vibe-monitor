# Module-owned PostgreSQL migration history

## Lifecycle / Compatibility

The initial compatibility boundary is the deployed `4.13.11` source fixture. A migrated authority requires a compatible binary and forward recovery; retained SQLite tables are not an automatic rollback target.

## Replacements / Background

The module ownership, transaction boundaries and durable authority decision are recorded in [ADR 0050](../../adr/0050-module-owned-postgresql-migration.md). The acceptance contract and operations runbook retain detailed evidence and deployment procedures.

## References

- [Requirements](./SPEC.md)
- [Implementation coverage](./IMPLEMENTATION.md)
