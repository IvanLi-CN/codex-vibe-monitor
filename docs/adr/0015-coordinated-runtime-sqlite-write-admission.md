# Coordinated Runtime SQLite Write Admission

Status: Accepted

The service keeps SQLite as its main runtime store, so its single-writer property is an explicit
runtime boundary. All runtime main-database writes are classified as P1 terminal, interactive
proxy, derived, or maintenance work and obtain a turn from the shared write coordinator before a
short write transaction begins; schema/bootstrap work before runtime startup is the sole explicit
exception. P1 remains dominant, derived and maintenance work defer rather than compete, and the
system-status memory snapshot exposes only low-cardinality admission and classified-pressure
evidence. This avoids using larger pools, longer timeouts, or request-time Summary fallbacks to
mask contention while preserving the exact-or-unavailable Summary contract.

## Consequences

New runtime write entry points must declare their write class at the outermost short write boundary.
Diagnostic output records error class and timing only; it never records SQL, bound values, proxy
payloads, or account identifiers. A pressure incident must be diagnosed from the health snapshot
and structured event evidence before changing capacity parameters.
