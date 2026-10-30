# PostgreSQL 18 Analytics Operations

> This is owner-authorized runbook work. Reading this document does not authorize or execute any command, configuration change, bootstrap, pause/resume, 101 operation, service deployment, production-data copy, or load/stress/pressure test. Each operational action requires a separate owner-approved change window. This documentation task performs none of them.

## Current Boundary

SQLite remains the default analytics backend and the existing state source. `ANALYTICS_BACKEND` accepts `sqlite` or `postgres`; the default is `sqlite`. Selecting `postgres` requires `POSTGRES_DATABASE_URL`. `POSTGRES_MAX_CONNECTIONS` defaults to `5` and must be positive. The connection URL is redacted from configuration debug and serialization output and from PostgreSQL failure diagnostics.

The PostgreSQL foundation only connects, verifies four pre-provisioned relations, and runs a disposable read-only transaction. It does not execute schema DDL, migrate business writes, backfill data, or silently fall back to a heavy SQLite analytics query after PostgreSQL is selected. Its schema contract creates `cvm_analytics.analytics_facts`, `analytics_daily_rollups`, `analytics_hourly_rollups`, and `analytics_state`; the initial state row is `preparing`.

Configuration alone never provisions the schema, copies data, or changes production query routing. Applying `ANALYTICS_BACKEND=postgres` through an owner-approved runtime change and restart initializes the optional PostgreSQL runtime, but the current foundation has no concrete analytics reader, shadow comparison, operator status route, or production read switch. Do not treat `/health` or a successful database connection as proof that analytics reads are serving from PostgreSQL. Keep production enablement gated until a later implementation supplies and validates those surfaces.

Invocation and attempt writes, credentials and routing, task state, TerminalJournal, and production switching are outside this batch. No guarantee or operational command for those paths is introduced here.

## Capacity Budget and Gate

Build separate estimates for each storage surface. Use measured representative non-production fixtures and an owner-approved growth and retention horizon; do not infer production capacity from schema size or a load test in this batch.

| Budget         | Include                                                                                                                                                                                                                                                                                                                  |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Data           | Heap for normalized facts, daily and hourly rollups, state rows, expected baseline/backfill volume, and retained growth.                                                                                                                                                                                                 |
| Indexes        | Each declared primary key and secondary index, plus separately measured growth and index-build workspace. Do not fold index bytes into the heap estimate.                                                                                                                                                                |
| WAL            | Expected WAL generation over the maximum approved archive delay, outage, and recovery window, plus the configured checkpoint target. `max_wal_size` is a soft checkpoint target, not a disk quota.                                                                                                                       |
| Change capture | No change-capture consumer is implemented by this foundation. For a separately approved future consumer, budget peak WAL generation multiplied by its maximum allowed lag/outage and catch-up window, plus any durable queue. Set a finite replication-slot retention limit and alert on stale slots before enabling it. |
| Backups        | Retained base backups and archived WAL, plus dump staging, restore workspace, and temporary duplicate capacity during rehearsal or recovery. Keep backup capacity separate from database and `pg_wal` capacity.                                                                                                          |
| Headroom       | Owner-defined free-space reserve, filesystem metadata, maintenance overhead, and incident recovery margin on every volume.                                                                                                                                                                                               |

**Disk-capacity gate:** before creating the database or applying schema SQL, record usable and free capacity separately for the data tablespace, `pg_wal`, WAL archive, backup destination, and restore workspace. For each volume, require its category budget plus its headroom reserve to fit within usable free capacity. A PASS requires every volume to pass and an owner-approved retention, alert, and recovery plan. If any estimate or volume is unknown or fails, stop before bootstrap. Do not use free space on another mount as a substitute unless the owner-approved storage design makes it available to that path.

## Private-Network PostgreSQL 18 Example

The following is an illustrative server configuration, not a production sizing recommendation. Replace the private address, client subnet, certificates, connection ceiling, and storage paths with values approved for the target environment. Bind only a private interface, restrict port `5432` at the network layer, and allow only the application and authorized DBA sources.

**Owner authorization required to apply or reload these settings.**

```conf
# postgresql.conf
listen_addresses = '10.20.0.15'
port = 5432
ssl = on
ssl_min_protocol_version = 'TLSv1.2'
ssl_cert_file = '/run/secrets/postgres.crt'
ssl_key_file = '/run/secrets/postgres.key'
password_encryption = 'scram-sha-256'

# Example ceiling only; account for the app pool, monitoring, DBA, and reserve.
max_connections = 20

# The foundation does not require logical decoding. Revisit only with a
# separately approved change-capture design and its WAL capacity budget.
wal_level = 'replica'
```

**Owner authorization required to change host-based authentication.** PostgreSQL applies the first matching `pg_hba.conf` record; keep any local DBA access in the site's reviewed policy.

```conf
# pg_hba.conf
hostssl  cvm_analytics  cvm_analytics_app  10.20.0.0/24  scram-sha-256
host     all            all                0.0.0.0/0      reject
host     all            all                ::/0           reject
```

Keep the application credential separate from the schema owner. Use a non-login owner for DDL, and a login role with no administrative attributes for the application. The current startup probe needs database `CONNECT`; it checks relation names and executes `SELECT 1`, but does not query table contents. Grant schema `USAGE` and table `SELECT` only when a separately reviewed read adapter requires them. Never grant the application role DDL or table writes.

**Owner authorization required to create roles/database or grant privileges. Run through the approved DBA channel; do not place a password in SQL, shell history, or logs. Execute statements individually or from a plain SQL file, not inside an explicit transaction; `CREATE DATABASE` cannot run in a transaction block.**

```sql
CREATE ROLE cvm_analytics_owner
  NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE cvm_analytics_reader
  NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE cvm_analytics_app
  LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;

CREATE DATABASE cvm_analytics OWNER cvm_analytics_owner;
REVOKE CONNECT ON DATABASE cvm_analytics FROM PUBLIC;
GRANT CONNECT ON DATABASE cvm_analytics TO cvm_analytics_app;
```

Set the application password through the approved secret manager or an interactive `psql` password prompt. Do not add it to the SQL above. A future read adapter may receive only the read role, after its scope and need are approved:

**Owner authorization required before granting future read access. These grants are not required by the current startup probe.**

```sql
GRANT USAGE ON SCHEMA cvm_analytics TO cvm_analytics_reader;
GRANT SELECT ON ALL TABLES IN SCHEMA cvm_analytics TO cvm_analytics_reader;
ALTER DEFAULT PRIVILEGES FOR ROLE cvm_analytics_owner IN SCHEMA cvm_analytics
  GRANT SELECT ON TABLES TO cvm_analytics_reader;
GRANT cvm_analytics_reader TO cvm_analytics_app;
```

The application configuration example is redacted. `<redacted>` is not a usable password. Store the real URL in the approved secret store, make its CA certificate available to the service, and use a private DNS name present in the server certificate.

**Owner authorization required to apply these environment values. Keep `sqlite` until a later, approved rollout gate.**

```dotenv
ANALYTICS_BACKEND=sqlite
POSTGRES_DATABASE_URL=postgresql://cvm_analytics_app:<redacted>@pg-analytics.internal:5432/cvm_analytics?sslmode=verify-full&sslrootcert=%2Frun%2Fsecrets%2Fpg-ca.crt
POSTGRES_MAX_CONNECTIONS=5
```

For PostgreSQL client tools, prepare owner-managed libpq service entries for the application, DBA, rehearsal source, and restore target. Keep passwords in a separate secret-managed password file with owner-only permissions; service files must not contain passwords.

**Owner authorization required to install or change client connection settings. This entry is illustrative and contains no credential.**

```conf
# pg_service.conf
[cvm-analytics-app]
host=pg-analytics.internal
port=5432
dbname=cvm_analytics
user=cvm_analytics_app
sslmode=verify-full
sslrootcert=/run/secrets/pg-ca.crt
```

The DBA, rehearsal, and restore entries use their own approved host, database, and role. Set `PGSERVICEFILE` to the protected service file and `PGPASSFILE` to the secret-managed password file in the authorized operator session. Neither file nor its credentials are provided by this runbook.

## Bootstrap, Status, Pause, Resume, and Verify Order

Run this sequence only under an owner-approved change window. Do not skip ahead from configuration to production reads.

1. **Capacity gate:** complete and approve the separate volume worksheet above. A failed or unknown budget blocks bootstrap.
2. **Bootstrap:** an authorized DBA creates the private PostgreSQL 18 database and roles, confirms TLS and network policy, then applies the exact `PG_ANALYTICS_SCHEMA_SQL` from the reviewed source revision. Application startup is not a schema installer. The current source defines this SQL in `src/pg_analytics/schema.rs`; there is no application CLI/API that exports or applies it. Prepare a reviewed standalone file from that exact revision, wrapped with `SET ROLE cvm_analytics_owner` and `RESET ROLE`, and retain its source SHA with the operations record.

   **Owner authorization required to apply the reviewed schema file. The DBA service must be authorized to `SET ROLE cvm_analytics_owner`; the file must contain the exact source SQL between the role statements.**

   ```sh
   psql 'service=cvm-analytics-admin' --set=ON_ERROR_STOP=1 \
     --file="$PG_ANALYTICS_SCHEMA_FILE"
   ```

3. **Status:** use PostgreSQL client tools for database status. The foundation does not expose a PostgreSQL status endpoint. `pg_isready` proves that a server accepts connections, not that the application selected PostgreSQL or that a read path is serving. `/health` remains service readiness, not an analytics status check.

   **Owner authorization required to run these read-only checks. Use a protected service/credential file; do not put secrets in command arguments.**

   ```sh
   pg_isready --dbname='service=cvm-analytics-app'
   psql 'service=cvm-analytics-app' --set=ON_ERROR_STOP=1 --command="SELECT current_setting('server_version_num')::int >= 180000 AND current_setting('server_version_num')::int < 190000 AS is_postgresql_18; SELECT to_regclass('cvm_analytics.analytics_facts'), to_regclass('cvm_analytics.analytics_daily_rollups'), to_regclass('cvm_analytics.analytics_hourly_rollups'), to_regclass('cvm_analytics.analytics_state');"
   psql 'service=cvm-analytics-app' --set=ON_ERROR_STOP=1 --command='BEGIN READ ONLY; SELECT 1; ROLLBACK;'
   ```

   For command-line clients, use an owner-controlled service file for host, database, username, TLS mode, and CA path, and a protected password file or secret manager for credentials. Never enable shell tracing while credentials are in scope.
4. **Backup and restore rehearsal:** before any future enablement, rehearse against a non-production database populated only with synthetic data. Store the archive and restore target on approved test volumes, verify the four relations and representative contents after restore, and record elapsed time and free-space observations. A SQL dump rehearsal does not by itself prove point-in-time recovery; the production backup design must also cover retained base backups and WAL archives. Prepare the restore database empty and configure its service entry to connect as its approved restore owner.

   **Owner authorization required. These commands are only for the synthetic non-production rehearsal; do not point them at a production database or production backup.**

   ```sh
   pg_dump --format=custom --no-owner --no-acl \
     --file="$BACKUP_DIR/cvm-analytics-rehearsal.dump" \
     --dbname='service=cvm-analytics-rehearsal'
   pg_restore --list "$BACKUP_DIR/cvm-analytics-rehearsal.dump"
   createdb --maintenance-db='service=cvm-analytics-admin' "$RESTORE_DB"
   pg_restore --exit-on-error --single-transaction --no-owner --no-acl \
     --dbname='service=cvm-analytics-restore' \
     "$BACKUP_DIR/cvm-analytics-rehearsal.dump"
   ```

5. **Pre-enable verify:** repeat the read-only checks below after the restore rehearsal and confirm that all four relations exist and the application role can connect. The foundation's `AnalyticsRuntimeStatus` (`disabled`, `ready`, `unavailable`) is internal only; no operator endpoint exposes it. A successful database probe is necessary but not sufficient for production enablement.

   **Owner authorization required to run these read-only checks.**

   ```sh
   pg_isready --dbname='service=cvm-analytics-app'
   psql 'service=cvm-analytics-app' --set=ON_ERROR_STOP=1 --command="SELECT current_setting('server_version_num')::int >= 180000 AND current_setting('server_version_num')::int < 190000 AS is_postgresql_18; SELECT to_regclass('cvm_analytics.analytics_facts'), to_regclass('cvm_analytics.analytics_daily_rollups'), to_regclass('cvm_analytics.analytics_hourly_rollups'), to_regclass('cvm_analytics.analytics_state');"
   psql 'service=cvm-analytics-app' --set=ON_ERROR_STOP=1 --command='BEGIN READ ONLY; SELECT 1; ROLLBACK;'
   ```

6. **Shadow comparison:** this foundation has no concrete `AnalyticsReadStore` implementation or shadow-read API. A later implementation must first define a supported compare path and acceptance thresholds. Compare the same source snapshot, range (`7d`, `30d`, `180d`, `365d`), dimension (`overall`, `model`, `upstream`), timezone (`Asia/Shanghai`), metric/sample counts, projection state, and daily-series shape. Preserve unavailable/incomplete states; do not convert them to zeros. Until the comparison is implemented and clear, keep production read routing on SQLite.
7. **Explicit enablement:** only after the owner accepts bootstrap, capacity, backup/restore, verification, and shadow evidence may a later change explicitly apply `ANALYTICS_BACKEND=postgres` and restart the service. Under I1/I2/I3 this selects and verifies the optional runtime only; it does not move business writes or wire long-term reads. This batch defines no production read-switch command or API.
8. **Observation:** a later read-enabled release must define its operator-visible PostgreSQL runtime status, projection freshness/unavailability checks, disk/WAL/slot/backups alerts, and observation window before enablement. Missing status or alert evidence keeps the enablement gate closed.
9. **Pause and resume:** to pause a PostgreSQL runtime that was explicitly enabled, an authorized operator restores `ANALYTICS_BACKEND=sqlite` through the approved deployment configuration and restarts the service; verify SQLite behavior before resuming traffic. Resume only after database status, schema/read-only verification, capacity, and backup gates pass again, then explicitly reapply the approved PostgreSQL selection and restart. Configuration-file edits without the approved apply/restart do nothing. Do not stop PostgreSQL or revoke roles as a substitute for pausing the application.
10. **Rollback:** pause PostgreSQL selection first, verify the application is using its unchanged SQLite state, and retain PostgreSQL data, schema, and backups for diagnosis. The current batch moves no business writes, so it defines no reverse-copy command. Any future cleanup or data deletion needs its own owner approval and retention review.

## Long-Term Pure-PostgreSQL Analytics Overview

“Pure PostgreSQL” here means one PostgreSQL-backed long-term analytics projection and query source after a separately approved migration; it does not mean moving every service domain into PostgreSQL. The intended stages are:

1. Keep SQLite as the current source while the optional PostgreSQL schema and bounded connection/runtime verification remain isolated.
2. In a later task, implement an idempotent normalized-fact ingestion/backfill adapter and daily/hourly rollup maintenance. Define source identity, completeness, restart behavior, retention, and unavailable semantics before running it.
3. Add a supported shadow comparison against SQLite on identical snapshots and the existing range, dimension, timezone, metric, sample-count, projection-state, and series contracts.
4. Require an explicit owner enablement after the capacity gate, backup/restore rehearsal, status surface, alerts, and shadow comparison all pass. Observe the approved window before declaring PostgreSQL the sole long-term analytics read source.
5. Only after stable parity, separately decide the retirement and retention of the SQLite long-term projection. Do not add an implicit fallback after PostgreSQL is explicitly selected.

This batch does not move invocation or attempt writes, credentials or routing, task state, TerminalJournal, or production read routing. Each remains governed by its existing SQLite contract unless a future approved design changes that boundary.

## References

- [PostgreSQL 18 connection and TLS settings](https://www.postgresql.org/docs/18/runtime-config-connection.html)
- [PostgreSQL 18 `pg_hba.conf` rules](https://www.postgresql.org/docs/18/auth-pg-hba-conf.html)
- [PostgreSQL 18 role creation](https://www.postgresql.org/docs/18/sql-createrole.html)
- [PostgreSQL 18 WAL settings](https://www.postgresql.org/docs/18/runtime-config-wal.html)
- [PostgreSQL 18 replication settings and slots](https://www.postgresql.org/docs/18/runtime-config-replication.html)
- [PostgreSQL 18 backup and restore](https://www.postgresql.org/docs/18/backup.html)
- [PostgreSQL 18 `pg_dump`](https://www.postgresql.org/docs/18/app-pgdump.html)
- [I1/I2/I3 specification](../specs/pg18-analytics/SPEC.md)
- [Implementation status](../specs/pg18-analytics/IMPLEMENTATION.md)
- [ADR 0051](../adr/0051-pg18-analytics-offload.md)
