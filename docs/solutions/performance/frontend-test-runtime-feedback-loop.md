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

Frontend test wall time is not one metric. Vitest execution, browser launch and build costs, fixed validation replay, and real CI queue/install time must be reported separately. A change is only comparable when baseline and candidate use the same Bun lockfile, Chromium, worker policy, source stage, profile, and VM resources.

The repository keeps the Web Demo semantic contract in a shared 26-route x 4-scene matrix. Real React application integration runs in jsdom so route/provider/handler behavior is checked without launching a browser for every matrix cell. Chromium retains the full operational route surface and representative attention, empty, and failure journeys where layout, accessibility, responsive behavior, sharing, writes, and failure propagation require a real page.

Slow Vitest suites should register extracted cases as explicit `registerSuite(scope)` modules. The Vite test transform may compile the extracted source once, but the test process must not call `typescript.transpileModule`, `new Function`, or a dynamic `with` scope for every run. Interaction tests keep real component mounts; parser and validation assertions belong in Node-environment tests; debounces and retries use controlled clocks with an explicit completion signal.

Playwright CI uses `web/playwright.ci.config.ts` to group fixture and demo functional projects. PR runs the functional batch followed by the performance batch. Main allows at most two global workers for the functional batch, with each project using one worker in practice, then runs performance and timeline capacity serially with no competing test load. Result directories and HTML reports stay isolated, and the producer exits with the combined status so the required gate cannot pass after a failed project.

Use `python3 scripts/benchmark-web-tests.py --baseline-src <baseline> --candidate-src <candidate> --baseline-sha <sha> --candidate-sha <sha> --stage <demo|unit|functional-e2e|frontend> --profile <pr|main> --rounds 7 --output <dir>` for paired evidence. The runner validates independent Git SHAs and matching lockfiles, records raw JSONL, command and environment metadata, wall and CPU time, independent `/usr/bin/time` RSS, exit code, retries, and logs. It warms each source once, alternates AB/BA order, keeps one isolated-process/cache sample descriptive, and excludes failures or retries from medians. For B0 it restores the legacy Playwright schedule when the grouped config is absent, so an optimization comparison cannot accidentally apply the candidate harness to the baseline. The frontend profile fails acceptance unless all seven pairs and warmups are valid, the PR improvement reaches 10% and 5 seconds, exceeds two baseline MADs, and has at least six faster pairs; main uses the bounded regression rule. The acceptance report must show each optimization delta and the final full-frontend replay separately; a fixed frontend replay is not a claim about the whole CI critical path.
