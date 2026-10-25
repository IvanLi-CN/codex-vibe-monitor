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

Frontend test wall time is not one metric. Report fixed validation replay, individual suite costs, and real CI queue/install time separately. A/B changes the test implementation and scheduling; runner, runtime, dependency versions, input fixtures, and intended semantic coverage remain controlled.

## Locked execution boundary

All performance A/B comparisons for this project must execute in GitHub Actions only. Local machines and Agent VM sessions are forbidden execution environments for A/B performance validation, including warmups, cold samples, paired rounds, and acceptance decisions. Local and Agent VM runs may perform syntax, contract, type, lint, and functional checks, but their timings must never be reported as performance evidence.

The persistent manual entry is `.github/workflows/web-test-ab.yml`. It has only `workflow_dispatch` and `workflow_call` triggers; it never launches on PR updates, labels, or a schedule. A and B use immutable full SHAs, independent checkouts and dependency/build caches, Bun 1.3.14, matching lockfiles, and the same installed Chromium. Each workload phase is one GitHub-hosted job that runs A and then B sequentially, so the paired samples share the same runner CPU, memory, kernel, runtime and filesystem without concurrent test load. The measured source trees, cache directories, ports and report paths remain isolated. There is no warmup, automatic repeat, or selective rerun. This is one controlled observation, not a multi-round statistical claim.

Dependency download and artifact transfer happen before measurement and are reported separately. They do not execute frontend tests or prebuild either source. Each measured group includes every build and test below; each E2E profile owns fresh processes and ports. The measurement job keeps the single existing 30-minute GitHub Actions workflow ceiling; the harness adds no subprocess deadline or fallback timeout. Playwright retry policies and Vitest worker limits remain unchanged. An actual retry prevents a successful measured result.

Every measured phase runs until its command naturally exits or the existing GitHub Actions workflow ceiling ends the job. The harness records wall time at command completion and performs process cleanup afterward; cleanup time is not included in the measured wall time. If the workflow ends before a natural completion record exists, the phase is incomplete/censored and has no numeric duration. No script-level deadline, fallback timeout, A+B total, or inferred completion time is permitted.

An A-phase workflow timeout is a threshold observation only when GitHub Actions confirms that the A job ended at the workflow ceiling and the complete B workload has natural-completion evidence. The A phase then has no numeric duration and no inferred saving; B timeouts, failed or incomplete reports, retries, missing specs, and infrastructure errors remain producer failures, and the aggregate result remains the required gate.

## Whole-PR workload and decision

| Measured phase | Included work                                                                                                            |
| -------------- | ------------------------------------------------------------------------------------------------------------------------ |
| Unit           | All existing unit tests, four target slow suites, and all added Demo jsdom integration cost                              |
| PWA            | Production PWA test build and asset contract                                                                             |
| Storybook      | Complete browser accessibility suite                                                                                     |
| Docs and Demo  | Storybook static build, docs build, Web Demo build, Pages assembly and file assertions                                   |
| PR E2E         | Source PR workflow's server startup, repeated production build, all functional and performance specs                     |
| Main E2E       | Source main workflow's server startup, repeated production build, functional, performance and timeline-capacity specs    |
| Tooling        | Web typecheck/lint, docs lint, quality-gate checker and negative tests, benchmark self-tests when present in that source |

The runner reads the effective named steps from **each source's own** `ci-pr.yml` and `ci-main.yml`. It never injects the candidate's grouped Playwright config into A. Each measured phase runs from an isolated checkout with separate generated outputs, cache directory and E2E ports; A and B execute sequentially in the same hosted job. Machine-readable reporters record four-suite timings, full unit/Storybook identities, browser spec coverage, failures and retries. Missing reports, missing specs, changed input chunks, removed test files or added skip declarations cannot establish an improvement.

| A result                                 | B result                                  | Performance decision                                                              |
| ---------------------------------------- | ----------------------------------------- | --------------------------------------------------------------------------------- |
| Workflow timeout/incomplete              | Natural completion with complete reports  | Threshold result only: B may pass; no numeric A duration or speedup is reported   |
| Natural completion with complete reports | Natural completion with complete reports  | Compare the measured natural wall times; apply the declared improvement threshold |
| Any result                               | Timed out, failed, incomplete, or retried | Fail; B scores 0                                                                  |
| Setup/infrastructure failure             | Any result                                | Invalid comparison; never reinterpret setup failure as an A workload timeout      |

The performance decision does not itself certify coverage equivalence or PR readiness. The Demo jsdom matrix therefore asserts each route's terminal state for operational, attention, and empty scenes, each network-failure terminal state, the expected failed API or SSE request, and the absence of unhandled or 501 responses. The manifest and report still list any target slow suite unchanged by the PR; test counts such as 104 jsdom cells and 38 browser cells are scope checks, while the route assertions provide the semantic certificate. Do not claim completion while a required target suite remains unchanged or the semantic audit is false.

## Trigger, evidence and cleanup

After this workflow exists on the default branch, trigger it manually with exact SHAs:

```sh
gh workflow run web-test-ab.yml --repo IvanLi-CN/codex-vibe-monitor \
  --ref th/web-test-performance \
  -f baseline_sha='<full-baseline-sha>' -f candidate_sha='<full-candidate-sha>'
```

GitHub requires a `workflow_dispatch` entry to exist on the default branch. Before merge, use the already registered manual Observability Performance entry on the PR branch. Its optional frontend input routes exclusively to the reusable frontend workflow; no backend image build or observability experiment runs:

```sh
gh workflow run ci-observability-performance.yml --repo IvanLi-CN/codex-vibe-monitor \
  --ref th/web-test-performance \
  -f web_test_baseline_sha='<full-baseline-sha>' -f candidate_sha='<full-candidate-sha>'
```

The run uploads `web-test-ab-<run-id>` with `manifest.json`, `preparation.json`, `summary.json`, `summary.md`, and A/B raw JSONL, logs and machine-readable reports. The manifest binds exact SHAs, dependencies, commands, input hashes, cache policy and workload. Reports record stage wall/CPU time and the cumulative child-process RSS high-water mark; RSS is not falsely described as an independent per-stage maximum. Test execution elapsed time excludes queue/install/artifact transfer and does not predict a proportional change in real CI merge waiting time.

Export and verify evidence before removing the completed manual run and **all** its artifacts (including dependency artifacts):

```sh
python3 scripts/benchmark-web-tests.py cleanup \
  --repo IvanLi-CN/codex-vibe-monitor --run-id '<exact-run-id>' \
  --archive '<new-absolute-directory-outside-the-repository>'
```

Cleanup checks the exact workflow, completed manual event and evidence run ID before deletion. Keep the exported evidence for owner review; raw data does not belong in Git. The reusable manual workflow remains available for future comparisons. This workflow needs no temporary remote branches; hosted runner processes, source checkouts and caches disappear with the job.

Synthetic contract tests may run locally with `PYTHONDONTWRITEBYTECODE=1 python3 scripts/test_benchmark_web_tests.py`. They use fixed records and mocked clocks/processes, never execute a frontend A/B workload, and are not performance evidence.

The repository keeps the Web Demo semantic contract in a shared 26-route x 4-scene matrix. Real React application integration runs in jsdom so route/provider/handler behavior is checked without launching a browser for every matrix cell. Chromium retains the full operational route surface and representative attention, empty, and failure journeys where layout, accessibility, responsive behavior, sharing, writes, and failure propagation require a real page.

Slow Vitest suites should register extracted cases as explicit `registerSuite(scope)` modules. The Vite test transform may compile the extracted source once, but the test process must not call `typescript.transpileModule`, `new Function`, or a dynamic `with` scope for every run. Interaction tests keep real component mounts; parser and validation assertions belong in Node-environment tests; debounces and retries use controlled clocks with an explicit completion signal.

Playwright CI uses `web/playwright.ci.config.ts` to group fixture and demo functional projects. PR runs the functional batch first and the performance batch second against separate preview ports. Main runs the functional batch first, then the performance batch, then timeline capacity against separate preview ports; each project remains limited to its declared worker count. Main's capacity proof uses a 30-second observation window with the existing two-client, desktop/mobile, 15-second refresh, 1-second revision, overlap, release, virtualization, long-task, and error assertions; it does not change any workflow timeout or performance budget. This ordering keeps browser CPU and renderer load out of the performance and capacity measurements. Result directories and HTML reports stay isolated, and every batch status is propagated so the required gate cannot pass after a failed project.
