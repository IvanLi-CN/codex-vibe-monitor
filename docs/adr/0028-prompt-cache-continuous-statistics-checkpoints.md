# Prompt-Cache Statistics Commit Continuous Conversation Checkpoints

Status: Accepted

This succeeds the statistics batch-cursor and per-pending-page retry decisions in
[ADR 0027](0027-prompt-cache-materialization-step-boundaries.md). Identity batching,
the maintenance control authority, step registration, and generation fences remain active.

## Context

An outer cursor committed only after an entire key batch completes is too coarse for
resumable statistics. Publishing a preceding key deletes its staging row; if a later
key yields, the next run selects that preceding key again and restarts its scan. Several
large keys multiply this repeated work. Waiting fifteen seconds after every successful
source page also wastes the existing short run budget.

## Decision

For statistics rebuild, each conversation's final page commits its exact aggregate,
generation-bound queue/staging removal, and the continuous outer key cursor atomically.
Partial pages commit only their staging cursor and accumulator. Queue drain uses the
generation-bound queue deletion as the completion checkpoint. Later source changes
enqueue the affected key without resetting the completed rebuild prefix.

Continue unchanged-source pages within the existing three-second run budget, checking
operator control and interactive pressure before every page and yielding a ten-millisecond
scheduler window between commits. A key consumes one visited-key allowance per run, even
when it spans many pages. Each query uses the smaller of its existing two-second budget
and the remaining run budget. Generation changes and budget exhaustion retain the
fifteen-second follow-up; pressure keeps its existing eligibility/deadline behavior.

The source-page seek order is backed by the additive expression index
`idx_codex_invocations_prompt_cache_key_occurred_at_id` on prompt-cache key,
`occurred_at`, and invocation `id`. Startup creates it idempotently, and an
interrupted index refresh is safe to re-enter through the existing schema refresh
marker. Older readers continue to use the existing invocation rows and may ignore
the additional index.

Run counts describe visited keys and complete publications. Identity counters retain
their existing meaning; the nullable ETA is unavailable during incomplete statistics
phases because identity-key estimates do not measure remaining statistics work.

## Consequences

Completed prefixes survive cancellation and restart without restarting statistics scans.
Publication and cursor movement cannot disagree after transaction failure. No table,
column, response field, control authority, or source record encoding changes; the
page-order index is additive and idempotent. Existing
partial staging rows can resume directly. Validation must include multiple large keys
in one batch, transaction failure at publication, and a real dual-database service replay.
