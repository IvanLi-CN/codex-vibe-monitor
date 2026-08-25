# Runtime Read-Model Pressure Recovery - History

- The Initiative adopts an exact-read-model contract: Summary availability may not be obtained by returning partial, empty or request-time reconstructed data.
- A bounded recent index records its first omitted live timestamp; rolling and account windows reaching that boundary fail closed while later fully retained windows continue from memory.
- The Initiative records pressure defer as a scheduler state separate from an actual SQLite lock failure, so retry and audit behavior remain observable and bounded.
- Pressure defer keeps durable progress untouched because admission happens before SQLite; its eligibility deadline belongs to the in-memory scheduler, while a persisted real lock failure closes the pressure gate before releasing its permit.
- The Initiative keeps long-term migration incremental and low priority, preserving durable cursor recovery and P1 writer priority.
- Checkpoint publication is intentionally separate from deployment; production observation is read-only and begins only after owner confirmation of the exact deployed release.
