# Durable Hourly Invocation Prefixes

## Status

Accepted

Unbound Invocations have no conversation master. Their current hourly prefix and sequence counter
exist only in memory. Terminal records and the existing terminal journal are written after ID
allocation, so they cannot establish a durable ceiling for every issued ID. A separate journal
would duplicate that authority and introduce another file lifecycle.

Store hourly prefix reservations in a dedicated `hourly_invoke_prefixes` table in the business
SQLite database. Conversation reservations remain in `prompt_cache_conversations`; no conversation
mapping table is added. Both owner types use the range manager and committed-range policy in
[Conversation Invocation Range Reservations](0029-conversation-invocation-range-reservations.md).

## Durable Ownership

The table stores:

| Field                      | Contract                                                                                |
| -------------------------- | --------------------------------------------------------------------------------------- |
| `utc_hour`                 | Primary key: Unix timestamp divided by 3600, identifying one UTC hour.                  |
| `prefix`                   | Unique six-character NanoID using the existing 31-character alphabet.                   |
| `last_invoke_sequence`     | Reserved Invocation Ceiling, initially -1, bounded by 923520.                           |
| `created_at`, `updated_at` | UTC timestamps for initialization and reservation maintenance; not invocation activity. |

Create or recover the current hour's row on demand. Serialize prefix generation, collision checks,
and insertion with conversation prefix creation in one instance-local namespace. At most five
NanoID candidates are attempted. Candidates exclude existing conversation masters, unreleased
hourly rows, retained invocation prefixes, and known active or pending invocation identities.
Namespace protection tracks active or retained ownership and is removed on legitimate release;
it is not a permanent process-issued-prefix blacklist.

A restart in the same UTC hour recovers the same prefix and reserves above its committed ceiling.
Normal issuance uses memory. Each range contains 64 sequences, with a smaller final range allowed;
the fewer-than-32 active threshold, fewer-than-48 joining threshold, one standby, one refill, and
100 ms empty-range wait apply equally to hourly and conversation owners. One refill coordinator
can reserve both owner types in the same business-database batch and publishes ranges only after
commit. The allocation-cache sizing policy counts conversation entries; a current hourly owner is
managed separately and does not consume a conversation admission slot.

At an hour boundary, new unbound invocations use the new hour's owner. Already allocated
invocations keep their IDs. Old owners receive no new invocation allocations. Active invocation
references and pending persistence references protect them until terminal persistence or durable
journal reconciliation releases those references. Refills and cleanup use owner generations so
late results cannot publish into a released or replacement owner.

An ended hour's prefix is released only after its active references, pending persistence, range
operations, and invocation rows retained by the existing cleanup lifecycle are gone. Its table row
and memory namespace reservation are then removed. Current-hour rows remain reusable for that
hour, even when temporarily idle. No old invocation ID is rewritten, and archived history does not
become a new permanent prefix blacklist.

## Upgrade and Recovery

Install the table and its constraints as an additive, idempotent schema operation with an immutable
completion marker in the existing migration registry. Keep structural installation, on-demand
reservation-state recovery, and existing historical statistics materialization separately
observable. Installation does not scan invocation history or delay readiness for cache sizing.

For existing conversations, cold recovery preserves the maximum of the durable ceiling and known
retained or pending issued suffixes before reserving a new range. Statistics writers relinquish
ceiling ownership in the same program upgrade. Legacy unbound prefixes cannot reliably be mapped
to a single UTC hour; do not infer hourly rows or rewrite their IDs. New prefix creation excludes
known legacy retained and pending identities. Journal recovery must register pending identities
before they can race namespace creation or retention release. The new guarantee covers issuance
under the new reservation authority; an upgrade cannot reconstruct old IDs that the previous
program issued without persisting anywhere.

Committed ranges survive a stopped release and are skipped on restart unless a bounded conditional
return was confirmed. An interrupted schema installation is retried idempotently. Recovery uses
a subsequent forward-repair release; program rollback does not delete hourly rows, reinterpret
reservation ceilings, or perform an automatic down-migration. Older allocation writers are not
part of the newly migrated state's supported writer contract.

## Consequences

Prefix creation and range refill perform low-frequency business-database work. Hot issuance no
longer needs per-invocation sequence SQL or a new preallocation file. Hourly ownership now survives
restarts, at the cost of an additive durable table and explicit active/pending lifecycle tracking.

Structured diagnostics identify owner type, UTC hour, generated prefix, generation, range bounds,
commit/publication result, rollover, and release outcome. They exclude raw Prompt Cache Keys and
introduce no synchronous per-invocation file writes. Upgrade, crash, collision, rollover, mixed-owner
batching, and retention acceptance are defined in the identity Spec.
