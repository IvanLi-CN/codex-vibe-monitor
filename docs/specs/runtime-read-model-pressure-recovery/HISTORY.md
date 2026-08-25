# Runtime Read-Model Pressure Recovery - History

- The Initiative adopts an exact-read-model contract: Summary availability may not be obtained by returning partial, empty or request-time reconstructed data.
- A bounded recent index records its first omitted live timestamp; rolling and account windows reaching that boundary fail closed while later fully retained windows continue from memory.
- The Initiative records pressure defer as a scheduler state separate from an actual SQLite lock failure, so retry and audit behavior remain observable and bounded.
- Pressure defer keeps durable progress untouched because admission happens before SQLite; its eligibility deadline belongs to the in-memory scheduler, while a persisted real lock failure closes the pressure gate before releasing its permit.
- Coverage repair applies that same permit-scoped lock classification to its repair outcome and all following progress reads and writes, preserving one pressure record and the normal non-lock scheduler failure path.
- A retry-progress lock following a repair lock is one failed coverage attempt, so the persistence-error-first fallback records one pressure event while preserving the original-error fallback for non-pressure persistence failures.
- Coverage repair now returns every permit-scoped SQLite pressure error to the maintenance loop as a deferred outcome, preserving the in-memory pressure deadline and eligibility wake while suppressing task-run audit and generic retry writes; ordinary coverage errors remain visible as failed audited retries.
- The Initiative keeps long-term migration incremental and low priority, preserving durable cursor recovery and P1 writer priority.
- Checkpoint publication is intentionally separate from deployment; production observation is read-only and begins only after owner confirmation of the exact deployed release.
