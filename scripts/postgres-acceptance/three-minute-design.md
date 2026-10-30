# Three-minute migration functional check design

The current migration PR excludes the existing compile-and-test PostgreSQL workflow from acceptance. Its job is excluded for this parent branch, its merge-group ref and a manual dispatch on this branch, and the evidence report records not-run. Existing shared suite scripts retain their behavior for other callers; they are not an approved substitute for this check.

The full frontend test job, backend test archive producer and legacy retention migration job are likewise excluded for this PR. Their skipped checks are not successful test evidence. Other PRs and shared main-branch callers retain their previous behavior.

The replacement must naturally finish the complete test work in less than three minutes. A timeout, forced termination, a retry, or splitting the same long work between jobs does not meet this requirement. This design has not yet demonstrated that duration and must not be executed or claimed as passing evidence.

## Fixed workload

- Use the exact candidate's existing binary and test artifacts, with PostgreSQL 18 already available. Downloading, fixture creation, service startup, checks and cleanup remain part of the measured test work. No compilation is allowed inside this check.
- Use an isolated empty installation with the complete supported schema and at most 32 synthetic records spanning both modules, including one update and one deletion. Do not use production data or a load generator.
- Drive the production migration state machine through its completion events. The harness must subscribe before starting the worker and await explicit per-module completion; it must not poll every few seconds, sleep, or retry the complete scenario.
- Verify the authoritative target rows and deleted-key absence, then stop and restart the candidate once and verify the persisted backend selection. Assert actual database contents separately from reported status.
- Clean only this run's isolated processes, database/schema and files, once.

## Eligibility and evidence

Before enabling the replacement, review its exact finite fixture and transition path and identify all waits, retries and setup costs. Record the entire operation's elapsed time without truncating execution or excluding slow stages. Retain exact candidate identity, raw result and the scope actually covered. Until natural completion below three minutes is supported, leave it not-run and outside this PR's acceptance. This check does not establish pressure, performance, large historical migration or all crash-boundary coverage.
