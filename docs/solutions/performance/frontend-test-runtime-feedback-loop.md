---
title: Frontend test runtime feedback loop
module: web-tests
problem_type: performance
component: Vitest and Playwright validation
tags:
  - web
  - tests
  - vitest
  - playwright
  - benchmark
status: active
related_specs:
  - docs/specs/ykhfu-web-demo/SPEC.md
---

# Frontend test runtime feedback loop

Frontend test wall time is not one metric. Vitest execution, browser launch and build costs, fixed validation replay, and real CI queue/install time must be reported separately. A change is only comparable when baseline and candidate use the same Bun lockfile, Chromium, worker policy, source stage, profile, and GitHub Actions runner contract.

## Locked execution boundary

All performance A/B comparisons for this project must execute in GitHub Actions only. Local machines and Agent VM sessions are forbidden execution environments for A/B performance validation, including warmups, cold samples, paired rounds, and acceptance decisions. Local and Agent VM runs may perform syntax, contract, type, lint, and functional checks, but their timings must never be reported as performance evidence.

The benchmark must run as a temporary candidate-bound GitHub Actions workflow on a pinned runner. For a PR validation it is triggered by `pull_request.synchronize`, checks out the PR base and head into independent directories, installs the exact Bun and lockfile dependencies in each directory, uses the same Chromium revision and worker policy, and invokes this script with `GITHUB_ACTIONS=true`. The workflow uploads raw JSONL, metadata, summary JSON, Markdown, and logs as a run-scoped artifact. After the result is recorded, delete the temporary benchmark workflow, workflow run, and uploaded artifacts; do not leave a benchmark trigger or stale performance artifact in the repository.

The result record must include the workflow URL, run ID, candidate and baseline SHAs, runner image, Bun/Chromium versions, command line, cache policy, all samples, and acceptance outcome. A missing, failed, retried, mixed-environment, or partially downloaded sample invalidates that comparison; cleanup never converts an invalid run into a passing result.

Thirty minutes is a hard upper bound for every performance-validation test invocation and GitHub Actions benchmark job. The temporary workflow and the benchmark subprocess timeout must both be set to 1800 seconds; a timeout or cancellation is an invalid comparison, and the limit must not be extended. If a complete matrix cannot fit, split it into separate GitHub Actions jobs and aggregate only complete, matching artifacts; never continue the same test past the limit or fall back to a local machine or Agent VM.

The repository keeps the Web Demo semantic contract in a shared 26-route x 4-scene matrix. Real React application integration runs in jsdom so route/provider/handler behavior is checked without launching a browser for every matrix cell. Chromium retains the full operational route surface and representative attention, empty, and failure journeys where layout, accessibility, responsive behavior, sharing, writes, and failure propagation require a real page.

Slow Vitest suites should register extracted cases as explicit `registerSuite(scope)` modules. The Vite test transform may compile the extracted source once, but the test process must not call `typescript.transpileModule`, `new Function`, or a dynamic `with` scope for every run. Interaction tests keep real component mounts; parser and validation assertions belong in Node-environment tests; debounces and retries use controlled clocks with an explicit completion signal.

Playwright CI uses `web/playwright.ci.config.ts` to group fixture and demo functional projects. PR runs the functional batch followed by the performance batch. Main allows at most two global workers for the functional batch, with each project using one worker in practice, then runs performance and timeline capacity serially with no competing test load. Result directories and HTML reports stay isolated, and the producer exits with the combined status so the required gate cannot pass after a failed project.

Use the command `python3 scripts/benchmark-web-tests.py --baseline-src <baseline> --candidate-src <candidate> --baseline-sha <sha> --candidate-sha <sha> --stage <demo|unit|functional-e2e|frontend> --profile <pr|main> --rounds 7 --output <dir>` only from that GitHub Actions workflow. The runner validates independent Git SHAs and matching lockfiles, records raw JSONL, command and environment metadata, wall and CPU time, independent `/usr/bin/time` RSS, exit code, retries, and logs. It warms each source once, alternates AB/BA order, keeps one isolated-process/cache sample descriptive, and excludes failures or retries from medians. For B0 it restores the legacy Playwright schedule when the grouped config is absent, so an optimization comparison cannot accidentally apply the candidate harness to the baseline. The frontend profile fails acceptance unless all seven pairs and warmups are valid, the PR improvement reaches 10% and 5 seconds, exceeds two baseline MADs, and has at least six faster pairs; main uses the bounded regression rule. The acceptance report must show each optimization delta and the final full-frontend replay separately; a fixed frontend replay is not a claim about the whole CI critical path.

When the complete matrix cannot fit in one 30-minute job, the workflow must split it into independent jobs using `--rounds 1 --pair-start <n> --partitioned`; pair numbers remain global so odd/even jobs preserve AB/BA order. Each partition accepts only complete, successful, retry-free samples; a final GitHub Actions aggregation step alone applies the seven-pair median/MAD and improvement or regression gate.
