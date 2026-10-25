#!/usr/bin/env python3
"""Manual, whole-PR frontend A/B acceptance. Performance execution is Actions-only."""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import importlib.util
import json
import math
import os
import re
import resource
import shlex
import signal
import shutil
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path


GROUP_SECONDS = 180
AB_TOTAL_SECONDS = 600
WORKLOAD = ("unit", "pwa", "storybook-a11y", "docs-demo-pages", "e2e-pr", "e2e-main", "tooling-contracts")
ROOT = Path(__file__).resolve().parents[1]
LOCKFILES = (".bun-version", "bun.lock", "web/bun.lock", "docs-site/bun.lock")
SLOW_SUITES = (
    "src/pages/account-pool/UpstreamAccountCreate.test.tsx",
    "src/pages/account-pool/UpstreamAccounts.test.tsx",
    "src/features/dashboard/DashboardWorkingConversationsSection.test.tsx",
    "src/features/prompt-cache/PromptCacheConversationTable.test.tsx",
)
E2E_SPECS = (
    "records-filter-overlay.spec.ts", "demo-runtime.spec.ts",
    "retention-maintenance-ownership.spec.ts",
    "dashboard-working-conversations-layout.spec.ts",
    "dashboard-render-performance.spec.ts", "dashboard-timeline-capacity.spec.ts",
)


def digest(value: object) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=False).encode()).hexdigest()


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def command_output(command: list[str], cwd: Path | None = None) -> str:
    return subprocess.check_output(command, cwd=cwd, text=True, timeout=30).strip()


def actions_only() -> None:
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("Performance A/B execution is allowed only in GitHub Actions; local and Agent VM execution is forbidden")


def phase_exit_code(group: str, status: str) -> int:
    """Keep an expected baseline deadline available for the aggregate decision."""
    return 0 if status == "passed" or (group == "A" and status == "timeout") else 1


def load_workflow(source: Path, profile: str) -> dict:
    # Use the repository's existing parser, without importing scripts from either measured tree.
    spec = importlib.util.spec_from_file_location("web_ab_contract", ROOT / ".github/scripts/check_quality_gates_contract.py")
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module.load_yaml(source / f".github/workflows/ci-{profile}.yml")


def named_steps(source: Path, profile: str, job: str, names: tuple[str, ...], optional_names: tuple[str, ...] = ()) -> str:
    steps = load_workflow(source, profile)["jobs"][job]["steps"]
    selected = []
    for name in names:
        matches = [step for step in steps if step.get("name") == name]
        if not matches and name in optional_names:
            continue
        if len(matches) != 1 or not matches[0].get("run"):
            raise ValueError(f"{profile}/{job}: missing or ambiguous required step {name}")
        step = matches[0]
        cwd = source / step.get("working-directory", ".")
        # Workflow steps have independent shells: readiness uses `exit 0`.
        # A subshell preserves that exit without accidentally skipping the remaining workload.
        selected.append(f"(\ncd {shlex.quote(str(cwd))}\n{step['run']}\n)\n")
    return "\n".join(selected)


@dataclass(frozen=True)
class Phase:
    name: str
    command: str
    cwd: str = "."


def instrument_playwright(command: str, reports: Path) -> str:
    """Only add machine-readable reporters; preserve scheduling, budgets and retries."""
    index = 0
    def replace(match: re.Match) -> str:
        nonlocal index
        index += 1
        report = shlex.quote(str(reports / f"playwright-{index}.json"))
        return (f"PLAYWRIGHT_JSON_OUTPUT_NAME={report} bun run test:e2e -- "
                "--reporter=list,json,html " + match.group(1))
    return re.sub(r"bun run test:e2e -- ([^\n]+)", replace, command)


def phases(source: Path, reports: Path) -> list[Phase]:
    pwa = named_steps(source, "pr", "frontend-tests", ("Build and verify PWA icon contract",))
    storybook = named_steps(source, "pr", "storybook-accessibility-tests", ("Run Storybook accessibility suite",))
    site = named_steps(source, "pr", "docs-demo-build", (
        "Build Storybook static site", "Build docs site", "Build Web Demo static site",
        "Assemble public docs site smoke check",
    ))
    result = [
        Phase("unit", f"bun run test -- --reporter=default --reporter=json --outputFile={shlex.quote(str(reports / 'unit.json'))}", "web"),
        Phase("pwa", pwa),
        Phase("storybook-a11y", storybook.replace("bun run test-storybook", f"bun run test-storybook -- --reporter=default --reporter=json --outputFile={shlex.quote(str(reports / 'storybook.json'))}")),
        Phase("docs-demo-pages", site),
    ]
    for profile in ("pr", "main"):
        job = "records-overlay-e2e-producer" if profile == "pr" else "records-overlay-e2e"
        # Both profiles own fresh servers and keep their original production build cost.
        commands = named_steps(source, profile, job, (
            "Start Vite dev server", "Wait for Vite readiness",
            "Build production Web Demo for performance E2E", "Start production Web Demo server",
            "Wait for Web Demo readiness", "Start performance Web Demo server",
            "Wait for performance Web Demo readiness", "Start timeline capacity Web Demo server",
            "Wait for timeline capacity Web Demo readiness",
            "Run records overlay and Web Demo Playwright regression",
        ), optional_names=(
            "Start performance Web Demo server", "Wait for performance Web Demo readiness",
            "Start timeline capacity Web Demo server", "Wait for timeline capacity Web Demo readiness",
        ))
        directory = reports / profile
        directory.mkdir(parents=True, exist_ok=True)
        result.append(Phase(f"e2e-{profile}", instrument_playwright(commands, directory)))
    tools = "bun run typecheck:web\nbun run lint:web\nbun run lint:docs\npython3 .github/scripts/check_quality_gates_contract.py\nbash .github/scripts/test-quality-gates-contract.sh"
    # The baseline did not have benchmark self-tests. This is an added B-side cost, never hidden.
    if (source / "scripts/test_benchmark_web_tests.py").is_file():
        tools += "\npython3 scripts/test_benchmark_web_tests.py"
    result.append(Phase("tooling-contracts", tools))
    return result


def test_files(source: Path) -> list[str]:
    return sorted(str(path.relative_to(source / "web")) for path in (source / "web/src").rglob("*")
                  if re.search(r"\.(?:test|spec)\.tsx?$", path.name))


def normalize_approved_e2e_performance_change(path: Path, text: str) -> str:
    """Ignore only reviewed execution-only tuning while hashing test coverage."""
    if path.name == "dashboard-working-conversations-layout.spec.ts":
        text = text.replace(
            'await page.goto("/#/dashboard", { waitUntil: "networkidle" });',
            'await page.goto("/#/dashboard");',
        )
    if path.name != "dashboard-timeline-capacity.spec.ts":
        return text
    text = re.sub(
        r"\n// Keep the delayed request just beyond one refresh interval to exercise overlap\n"
        r"// protection without adding an unnecessary fixed wait to every viewport\.\n"
        r"const DELAYED_TRAVERSAL_MS = MINIMUM_REFRESH_INTERVAL_MS \+ 250;\n"
        r"// The dense render proof only needs an in-flight request; cross-refresh overlap is covered above\.\n"
        r"const DENSE_TRAVERSAL_DELAY_MS = 250;\n",
        "\n",
        text,
    )
    text = text.replace(
        "page.evaluate((delayMs) => {",
        "page.evaluate(() => {",
    )
    text = text.replace(
        "diagnostics.nextTraversalDelayMs = delayMs;",
        "diagnostics.nextTraversalDelayMs = 20_000;",
    )
    text = text.replace("    }, DELAYED_TRAVERSAL_MS);", "    });")
    text = text.replace("    }, DENSE_TRAVERSAL_DELAY_MS);", "    });")
    text = text.replace(
        """      try {
        const response = await originalFetch(input, init);
        if (isTimeline && response.status >= 400) diagnostics.timelineHttpFailures += 1;
        if (isTimeline && method === \"DELETE\") {
          if (response.status === 204) diagnostics.releasedSnapshots += 1;
          else diagnostics.releaseFailures += 1;
          diagnostics.activeTraversals = Math.max(0, diagnostics.activeTraversals - 1);
        }
        return response;
      } catch (error) {
        if (isTimeline && method === \"GET\" && !requestUrl.searchParams.has(\"cursor\")) {
          diagnostics.activeTraversals = Math.max(0, diagnostics.activeTraversals - 1);
        }
        throw error;
      }
""",
        """      const response = await originalFetch(input, init);
      if (isTimeline && response.status >= 400) diagnostics.timelineHttpFailures += 1;
      if (isTimeline && method === \"DELETE\") {
        if (response.status === 204) diagnostics.releasedSnapshots += 1;
        else diagnostics.releaseFailures += 1;
        diagnostics.activeTraversals = Math.max(0, diagnostics.activeTraversals - 1);
      }
      return response;
""",
    )
    text = text.replace(
        "        .toBeGreaterThan(beforeRefresh.releaseCount);\n",
        "        .toBeGreaterThan(beforeRefresh.releaseCount + 1);\n",
    )
    text = re.sub(
        r"\n      const timelineControls = page.*?\n      await expectVisibleCallsAreVirtualized\(page, 550\);\n",
        """      await expect
        .poll(
          () =>
            page.evaluate(
              () =>
                (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__
                  ?.activeTraversals ?? 0,
            ),
          { timeout: 45_000 },
        )
        .toBe(1);

      const refreshedLanes = page.getByTestId("dashboard-invocation-timeline-lanes");
      await expect(refreshedLanes).toBeVisible();
      await expect(refreshedLanes).toHaveAttribute("data-total-calls", "550");
      await expect(page.getByTestId("dashboard-invocation-timeline-state")).toHaveCount(0);
      await expectVisibleCallsAreVirtualized(page, 550);

      await expect
        .poll(
          () =>
            page.evaluate(
              () =>
                (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__
                  ?.releasedSnapshots ?? 0,
            ),
          { timeout: 35_000 },
        )
        .toBeGreaterThan(beforeRefresh.releaseCount + 1);
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__
                ?.activeTraversals ?? 0,
          ),
        )
        .toBe(0);
""",
        text,
        count=1,
        flags=re.DOTALL,
    )
    text = text.replace("const LIVE_REFRESH_TIMER_GUARD_MS = 50;\n", "")
    text = text.replace(
        """      await page.addInitScript((minimumRefreshIntervalMs) => {
        const nativeSetTimeout = window.setTimeout.bind(window);
        window.setTimeout = ((...args: Parameters<typeof window.setTimeout>) => {
          const [handler, timeout, ...rest] = args;
          const acceleratedTimeout =
            typeof timeout === \"number\" && timeout >= minimumRefreshIntervalMs ? 0 : timeout;
          return nativeSetTimeout(handler, acceleratedTimeout, ...rest);
        }) as typeof window.setTimeout;
      }, MINIMUM_REFRESH_INTERVAL_MS);
""",
        "",
    )
    text = text.replace(
        """      // The real refresh cadence is covered by the two-client proof above. Advance the scheduler
      // clock here so the dense render proof measures the refresh instead of waiting 15 seconds.
""",
        "",
    )
    text = text.replace(
        """      expect(
        diagnostics?.traversalStarts.every(
          (startedAt, index) =>
            index === 0 ||
            startedAt - diagnostics.traversalStarts[index - 1]! >= MINIMUM_REFRESH_INTERVAL_MS,
        ),
      ).toBe(true);
""",
        "",
    )
    text = text.replace(
        """      // The real refresh cadence is covered by the two-client proof above. Advance the scheduler
      // clock here so the dense render proof measures the refresh instead of waiting 15 seconds.
      await page.evaluate(
        (advanceByMs) => {
          const now = Date.now();
          Date.now = () => now + advanceByMs;
        },
        MINIMUM_REFRESH_INTERVAL_MS + LIVE_REFRESH_TIMER_GUARD_MS + 1,
      );
""",
        "",
    )
    text = text.replace(
        """      // The real refresh cadence is covered by the two-client proof above. Advance the scheduler
      // clock here so the dense render proof measures the refresh instead of waiting 15 seconds.
      await page.evaluate((advanceByMs) => {
        const now = Date.now();
        Date.now = () => now + advanceByMs;
      }, MINIMUM_REFRESH_INTERVAL_MS + LIVE_REFRESH_TIMER_GUARD_MS + 1);
""",
        "",
    )
    text = re.sub(
        r"\n    await new Promise<void>\(\(resolve\) => \{\n"
        r"      setTimeout\(resolve, 16_000\);\n"
        r"    \}\);",
        "",
        text,
    )
    return text


def fixture_inventory(source: Path) -> dict:
    """Freeze unchanged input chunks and specs so deleting tests cannot manufacture a win."""
    paths = list((source / "web/src/pages/account-pool").glob("*.txt"))
    paths += [source / "web/tests/e2e" / name for name in E2E_SPECS if name != "demo-runtime.spec.ts"]
    return {str(path.relative_to(source)): hashlib.sha256(
                normalize_approved_e2e_performance_change(path, path.read_text()).encode()
            ).hexdigest()
            for path in sorted(paths)}


def skip_inventory(source: Path) -> list[str]:
    # Existing skipped cases remain visible; newly disabled coverage blocks acceptance.
    return sorted(f"{path.relative_to(source)}:{line.strip()}"
                  for directory in (source / "web/src", source / "web/tests")
                  for path in directory.rglob("*") if path.suffix in (".ts", ".tsx")
                  for line in path.read_text().splitlines()
                  if re.search(r"\b(?:it|test|describe)\.(?:skip|todo|skipIf)\b", line))


def demo_coverage_audit(source: Path) -> dict:
    """Require route terminal states and expected failed-request contracts, not landmarks alone."""
    integration = source / "web/src/demo/demo-route.integration.test.tsx"
    issues = []
    if integration.is_file():
        text = integration.read_text()
        required_markers = (
            "assertRouteTerminalState",
            "expectedNetworkFailurePath",
            "failedDemoRequests",
            'expect(unhandledRequests, `${route.path} / ${scene}`).toEqual([])',
        )
        missing = [marker for marker in required_markers if marker not in text]
        if missing:
            issues.append(
                "Demo integration is missing semantic terminal/request assertions: "
                + ", ".join(missing)
            )
    else:
        issues.append("Demo route integration test is missing")
    return {"verified": not issues, "issues": issues}


def prepare(baseline: Path, candidate: Path, baseline_sha: str, candidate_sha: str, output: Path) -> dict:
    identities = {}
    plans = {}
    for group, source, expected in (("A", baseline, baseline_sha), ("B", candidate, candidate_sha)):
        if not re.fullmatch(r"[0-9a-f]{40}", expected):
            raise ValueError("Both sources must be full, immutable 40-character SHAs")
        actual = command_output(["git", "rev-parse", "HEAD"], source)
        if actual != expected or command_output(["git", "status", "--porcelain", "--untracked-files=no"], source):
            raise ValueError(f"{group}: source SHA mismatch or dirty tracked files")
        identities[group] = {"sha": actual, "locks": {name: hashlib.sha256((source / name).read_bytes()).hexdigest() for name in LOCKFILES}}
        plans[group] = [asdict(phase) for phase in phases(source, output / group / "reports")]
    if baseline == candidate or identities["A"]["locks"] != identities["B"]["locks"]:
        raise ValueError("Sources must be isolated and all Bun versions/lockfiles identical")
    if (baseline / ".bun-version").read_text().strip() != "1.3.14":
        raise ValueError("Bun 1.3.14 is required")
    if fixture_inventory(baseline) != fixture_inventory(candidate):
        raise ValueError("Extracted case inputs or unchanged E2E specs differ; coverage needs a new audit")
    missing = set(test_files(baseline)) - set(test_files(candidate))
    if missing:
        raise ValueError(f"Candidate removed test files: {sorted(missing)}")
    if skip_inventory(baseline) != skip_inventory(candidate):
        raise ValueError("Skipped/todo test declarations changed; coverage needs a new audit")
    changed = command_output(["git", "diff", "--name-only", baseline_sha, candidate_sha], candidate).splitlines()
    for name in changed:
        if name.startswith(".github/workflows/") and (baseline / name).is_file() and (candidate / name).is_file():
            limits = lambda path: re.findall(r"^\s*timeout-minutes:.*$", path.read_text(), re.MULTILINE)
            if limits(baseline / name) != limits(candidate / name):
                raise ValueError(f"Existing workflow timeouts changed: {name}")
    unimplemented = [name for name in SLOW_SUITES if "web/" + name not in changed]
    manifest = {
        "schema": 2, "budget_seconds": GROUP_SECONDS, "ab_total_budget_seconds": AB_TOTAL_SECONDS,
        "identities": identities,
        "source_roots": {"A": str(baseline.resolve()), "B": str(candidate.resolve())},
        "output_root": str(output.resolve()),
        "workload": [phase["name"] for phase in plans["A"]], "plans": plans,
        "slow_suites": list(SLOW_SUITES), "e2e_specs": list(E2E_SPECS),
        "test_files": {"A": test_files(baseline), "B": test_files(candidate)},
        "fixture_inventory": fixture_inventory(baseline), "changed_files": changed,
        "unimplemented_slow_suites": unimplemented,
        "demo_coverage_audit": demo_coverage_audit(candidate),
        "declared_demo_cells": {"semantic": 104, "baseline_browser": 104, "candidate_jsdom": 104, "candidate_browser": 38},
        "cache_policy": "independent fresh source/build caches; no prebuild, warmup or automatic repeat",
    }
    manifest["digest"] = digest(manifest)
    write_json(output / "manifest.json", manifest)
    return manifest


def terminate_group(process: subprocess.Popen) -> None:
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def isolate_phase_sources(source: Path, phases_to_run: list[Phase], root: Path) -> dict[str, Path]:
    """Give concurrent CI jobs the independent checkout each hosted job would have."""
    sources: dict[str, Path] = {}
    root.mkdir(parents=True, exist_ok=True)
    for phase in phases_to_run:
        target = root / phase.name
        subprocess.run(["git", "clone", "--local", str(source), str(target)],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for relative in ("node_modules", "web/node_modules", "docs-site/node_modules"):
            dependency = source / relative
            link = target / relative
            if dependency.is_dir() and not link.exists():
                link.symlink_to(dependency, target_is_directory=True)
        sources[phase.name] = target
    return sources


def rebase_phase(phase: Phase, source: Path, isolated: Path, group: str) -> Phase:
    command = phase.command.replace(str(source), str(isolated))
    if phase.name == "e2e-pr":
        ports = {"60080": "60180", "60083": "60183", "60084": "60184", "60085": "60185"}
    elif phase.name == "e2e-main":
        ports = {"60080": "60280", "60083": "60283", "60084": "60284", "60085": "60285"}
    else:
        ports = {}
    for old, new in ports.items():
        command = command.replace(old, new)
    return Phase(phase.name, command, phase.cwd)


def execute_phase(phase: Phase, source: Path, directory: Path, deadline: float, env: dict) -> dict:
    remaining = deadline - time.monotonic()
    started_unix = time.time()
    record = {"name": phase.name, "command": phase.command, "status": "timeout", "returncode": 124,
              "wall_seconds": 0.0, "cpu_seconds": 0.0, "children_peak_rss_kb_cumulative": 0,
              "started_unix": started_unix, "finished_unix": started_unix}
    if remaining <= 0:
        return record
    started = time.monotonic()
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    with (directory / f"{phase.name}.log").open("w") as log:
        # Each phase owns its entire tree, including background Vite and Chromium processes.
        process = subprocess.Popen(["bash", "-euo", "pipefail", "-c", phase.command],
                                   cwd=source / phase.cwd, env=env, stdout=log, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        try:
            code = process.wait(timeout=max(0.001, deadline - time.monotonic()))
            record.update(status="passed" if code == 0 else "failed", returncode=code)
        except subprocess.TimeoutExpired:
            terminate_group(process)
            process.wait()
        finally:
            terminate_group(process)
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    record.update(wall_seconds=time.monotonic() - started,
                  cpu_seconds=after.ru_utime + after.ru_stime - usage.ru_utime - usage.ru_stime,
                  children_peak_rss_kb_cumulative=after.ru_maxrss,
                  finished_unix=time.time())
    if record["status"] == "passed" and time.monotonic() > deadline:
        record.update(status="timeout", returncode=124)
    return record


def materialize_phase(phase: Phase, manifest: dict, group: str, source: Path, directory: Path) -> Phase:
    command = phase.command
    command = command.replace(manifest["source_roots"][group], str(source.resolve()))
    old_reports = Path(manifest["output_root"]) / group / "reports"
    command = command.replace(str(old_reports), str((directory / "reports").resolve()))
    return Phase(phase.name, command, phase.cwd)


def environment_for_source(source: Path) -> dict:
    context = environment()
    descriptor = source / "web/node_modules/playwright-core/browsers.json"
    context["chromium_descriptor"] = json.loads(descriptor.read_text())["browsers"]
    return context


def phase_report(phase_name: str, directory: Path) -> dict:
    if phase_name == "unit":
        return {"unit": vitest_results(directory / "reports/unit.json")}
    if phase_name == "storybook-a11y":
        return {"storybook": vitest_results(directory / "reports/storybook.json")}
    if phase_name in ("e2e-pr", "e2e-main"):
        profile = phase_name.removeprefix("e2e-")
        return {"browser": {profile: playwright_results(directory / "reports" / profile)}}
    return {}


def run_phase(group: str, source: Path, source_sha: str, phase_name: str, manifest: dict, output: Path) -> dict:
    actions_only()
    if group not in ("A", "B") or phase_name not in manifest["workload"]:
        raise ValueError(f"Unknown group or workload phase: {group}/{phase_name}")
    directory = output / "results" / group / phase_name
    directory.mkdir(parents=True, exist_ok=True)
    actual_sha = command_output(["git", "rev-parse", "HEAD"], source)
    if actual_sha != source_sha or actual_sha != manifest["identities"][group]["sha"]:
        raise ValueError("Phase source SHA does not match the prepared manifest")
    context = environment_for_source(source)
    if context["bun"] != "1.3.14":
        raise ValueError("Bun 1.3.14 is required")
    planned = [Phase(**raw) for raw in manifest["plans"][group]]
    phase = next((candidate for candidate in planned if candidate.name == phase_name), None)
    if phase is None:
        raise ValueError(f"Manifest is missing phase {group}/{phase_name}")
    phase = materialize_phase(phase, manifest, group, source, directory)
    env = {**os.environ, "CI": "1", "DOCS_BASE": f"/{os.environ['GITHUB_REPOSITORY'].split('/')[-1]}/",
           "PYTHONDONTWRITEBYTECODE": "1", "PLAYWRIGHT_BROWSERS_PATH": str(output / "browsers"),
           "STORYBOOK_DISABLE_TELEMETRY": "1", "XDG_CACHE_HOME": str(output / "cache" / group / phase_name)}
    record = execute_phase(phase, source, directory, time.monotonic() + GROUP_SECONDS, env)
    record.update(group=group, sha=source_sha, manifest_digest=manifest["digest"], environment=context)
    if record["status"] == "passed":
        try:
            record["report"] = phase_report(phase_name, directory)
        except (OSError, ValueError, KeyError) as error:
            record.update(status="failed", returncode=1, reason=f"Missing or malformed phase report: {error}")
    write_json(directory / "result.json", record)
    return record


def normalized_environment(records: list[dict]) -> dict:
    if not records:
        return {}
    keys = ("bun", "runner_image", "runner_image_version", "kernel", "chromium_descriptor")
    return {key: records[0].get(key) for key in keys}


def aggregate_group(group: str, manifest: dict, results_root: Path) -> dict:
    records = []
    for name in manifest["workload"]:
        path = results_root / group / name / "result.json"
        if path.is_file():
            records.append(json.loads(path.read_text()))
        else:
            records.append({"name": name, "group": group, "status": "environment_failed", "returncode": 1,
                            "wall_seconds": 0.0, "cpu_seconds": 0.0, "children_peak_rss_kb_cumulative": 0,
                            "reason": "Missing phase evidence"})
    statuses = {record.get("status") for record in records}
    if "timeout" in statuses:
        status = "timeout"
    elif statuses == {"passed"}:
        status = "passed"
    else:
        status = "failed"
    unit = next((record.get("report", {}).get("unit") for record in records if record["name"] == "unit"), None)
    unit = unit or {"success": False, "suites": {}, "cases": []}
    storybook = next((record.get("report", {}).get("storybook") for record in records if record["name"] == "storybook-a11y"), None)
    storybook = storybook or {"success": False, "cases": []}
    browser = {}
    for profile in ("pr", "main"):
        record = next((record for record in records if record["name"] == f"e2e-{profile}"), {})
        browser[profile] = record.get("report", {}).get("browser", {}).get(profile, {"success": False, "cases": []})
        expected = set(E2E_SPECS) - ({"dashboard-timeline-capacity.spec.ts"} if profile == "pr" else set())
        actual = {Path(case["file"]).name for case in browser[profile].get("cases", [])}
        browser[profile]["success"] = browser[profile].get("success", False) and actual == expected
    reports = {"success": bool(unit.get("success")) and bool(storybook.get("success"))
               and all(report.get("success", False) for report in browser.values())
               and all(name in unit.get("suites", {}) for name in SLOW_SUITES),
               "unit": unit, "storybook": storybook, "browser": browser}
    wall_seconds = max((float(record.get("wall_seconds", 0.0)) for record in records), default=0.0)
    started = [record["started_unix"] for record in records if "started_unix" in record]
    finished = [record["finished_unix"] for record in records if "finished_unix" in record]
    result = {"group": group, "sha": manifest["identities"][group]["sha"], "manifest_digest": manifest["digest"],
              "status": status, "score": int(status == "passed"), "environment": normalized_environment(records),
              "phases": records, "reports": reports, "phase_wall_seconds_sum": sum(record.get("wall_seconds", 0.0) for record in records),
              "execution_wall_seconds": wall_seconds, "wall_seconds": wall_seconds}
    if started and finished:
        result["phase_span_seconds"] = max(finished) - min(started)
    return result


def aggregate(manifest: dict, results_root: Path, output: Path) -> dict:
    actions_only()
    groups = {group: aggregate_group(group, manifest, results_root) for group in ("A", "B")}
    for group in ("A", "B"):
        raw = output / group / "raw.jsonl"
        raw.parent.mkdir(parents=True, exist_ok=True)
        with raw.open("w") as stream:
            for record in groups[group]["phases"]:
                stream.write(json.dumps(record, ensure_ascii=False) + "\n")
    verdict = compare(groups["A"], groups["B"], manifest)
    summary = {"run_id": os.environ["GITHUB_RUN_ID"], "manifest": manifest, "A": groups["A"], "B": groups["B"],
               "verdict": verdict,
               "implementation_ready": verdict["passed"] and manifest["demo_coverage_audit"]["verified"]
               and not manifest["unimplemented_slow_suites"]}
    write_json(output / "summary.json", summary)
    (output / "summary.md").write_text(markdown(groups["A"], groups["B"], manifest, verdict))
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            stream.write((output / "summary.md").read_text())
    return summary


def vitest_results(path: Path) -> dict:
    report = json.loads(path.read_text())
    suites = {}
    cases = []
    for suite in report.get("testResults", []):
        name = suite["name"].replace("\\", "/").split("/web/", 1)[-1]
        assertions = suite.get("assertionResults", [])
        suites[name] = {"seconds": (suite.get("endTime", 0) - suite.get("startTime", 0)) / 1000,
                        "cases": len(assertions), "status": suite.get("status")}
        cases.extend({"file": name, "name": item.get("fullName"), "status": item.get("status")}
                     for item in assertions)
    return {"success": bool(report.get("success")) and bool(cases)
                       and not report.get("numFailedTests") and not report.get("numFailedTestSuites")
                       and all(case["status"] in ("passed", "pending", "todo", "skipped") for case in cases),
            "suites": suites, "cases": cases}


def playwright_results(reports: Path) -> dict:
    cases = []
    def walk(suites: list[dict], parents: tuple = ()) -> None:
        for suite in suites:
            names = (*parents, suite.get("title", ""))
            for spec in suite.get("specs", []):
                for test in spec.get("tests", []):
                    results = test.get("results", [])
                    cases.append({"file": spec.get("file", suite.get("file")),
                                  "name": " > ".join((*names, spec["title"])),
                                  "project": test.get("projectName"),
                                  "passed": bool(results) and all(item.get("status") == "passed" and item.get("retry", 0) == 0 for item in results)
                                            and test.get("status") == "expected"})
            walk(suite.get("suites", []), names)
    errors = []
    files = sorted(reports.glob("playwright-*.json"))
    for path in files:
        report = json.loads(path.read_text())
        walk(report.get("suites", []))
        errors.extend(report.get("errors", []))
    return {"success": bool(files) and bool(cases) and not errors and all(case["passed"] for case in cases), "cases": cases}


def validate_reports(directory: Path) -> dict:
    unit = vitest_results(directory / "reports/unit.json")
    storybook = vitest_results(directory / "reports/storybook.json")
    browser = {profile: playwright_results(directory / "reports" / profile) for profile in ("pr", "main")}
    for profile, report in browser.items():
        expected = set(E2E_SPECS) - ({"dashboard-timeline-capacity.spec.ts"} if profile == "pr" else set())
        actual = {Path(case["file"]).name for case in report["cases"]}
        report["success"] = report["success"] and actual == expected
    return {"success": unit["success"] and storybook["success"] and all(report["success"] for report in browser.values())
                       and all(name in unit["suites"] for name in SLOW_SUITES),
            "unit": unit, "storybook": storybook, "browser": browser}


def environment() -> dict:
    cpu = next((line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")), "unknown")
    return {"bun": command_output(["bun", "--version"]), "cpu_model": cpu, "cpu_count": os.cpu_count(),
            "memory": Path("/proc/meminfo").read_text().splitlines()[0],
            "runner_image": os.environ.get("ImageOS"), "runner_image_version": os.environ.get("ImageVersion"),
            "run_id": os.environ.get("GITHUB_RUN_ID"), "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
            "python": sys.version, "kernel": os.uname().release}


def run_group(group: str, source: Path, manifest: dict, output: Path) -> dict:
    actions_only()
    directory = output / group
    directory.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "CI": "1", "DOCS_BASE": f"/{os.environ['GITHUB_REPOSITORY'].split('/')[-1]}/",
           "PYTHONDONTWRITEBYTECODE": "1", "PLAYWRIGHT_BROWSERS_PATH": str(output / "browsers"),
           "STORYBOOK_DISABLE_TELEMETRY": "1"}
    context = environment()
    descriptor = source / "web/node_modules/playwright-core/browsers.json"
    context["chromium_descriptor"] = json.loads(descriptor.read_text())["browsers"]
    if context["bun"] != "1.3.14" or command_output(["git", "rev-parse", "HEAD"], source) != manifest["identities"][group]["sha"]:
        raise ValueError("Group environment or source identity changed after preparation")
    planned = [Phase(**raw) for raw in manifest["plans"][group]]
    phase_sources = isolate_phase_sources(source, planned, directory / "isolated-sources")
    executable = [rebase_phase(phase, source, phase_sources[phase.name], group) for phase in planned]
    started = time.monotonic()
    deadline = started + GROUP_SECONDS
    result = {"group": group, "sha": manifest["identities"][group]["sha"], "manifest_digest": manifest["digest"],
              "status": "running", "score": 0, "budget_seconds": GROUP_SECONDS,
              "execution_mode": "parallel-independent-ci-jobs", "environment": context, "phases": []}
    write_json(directory / "result.json", result)
    print(f"::{group}:: launching {len(executable)} independent CI jobs", flush=True)
    records: dict[str, dict] = {}
    def run_one(phase: Phase) -> dict:
        phase_env = {**env, "XDG_CACHE_HOME": str(directory / "cache" / phase.name)}
        return execute_phase(phase, phase_sources[phase.name], directory, deadline, phase_env)
    with ThreadPoolExecutor(max_workers=len(executable)) as executor:
        futures = {executor.submit(run_one, phase): phase.name for phase in executable}
        for future in as_completed(futures):
            name = futures[future]
            try:
                records[name] = future.result()
            except (OSError, subprocess.SubprocessError, ValueError) as error:
                records[name] = {"name": name, "status": "environment_failed", "returncode": 1,
                                 "wall_seconds": 0.0, "cpu_seconds": 0.0,
                                 "children_peak_rss_kb_cumulative": 0, "reason": str(error)}
    result["phases"] = [records[phase.name] for phase in executable]
    with (directory / "raw.jsonl").open("w") as stream:
        for record in result["phases"]:
            stream.write(json.dumps(record) + "\n")
    statuses = {record["status"] for record in result["phases"]}
    if "timeout" in statuses:
        result["status"] = "timeout"
    elif statuses == {"passed"}:
        result["status"] = "passed"
    else:
        result["status"] = "failed"
    result["phase_wall_seconds_sum"] = sum(record["wall_seconds"] for record in result["phases"])
    result["execution_wall_seconds"] = time.monotonic() - started
    if result["status"] == "passed":
        try:
            result["reports"] = validate_reports(directory)
            if not result["reports"]["success"]:
                result.update(status="failed", reason="Incomplete, skipped, retried or failed test reports")
        except (OSError, ValueError, KeyError) as error:
            result.update(status="failed", reason=f"Missing or malformed report: {error}")
    elif (directory / "reports/unit.json").is_file():
        # Keep completed unit-suite timings even when a later phase hits the deadline.
        try:
            result["reports"] = {"success": False, "unit": vitest_results(directory / "reports/unit.json")}
        except (OSError, ValueError, KeyError):
            pass
    result["wall_seconds"] = time.monotonic() - started
    if result["status"] == "passed" and result["wall_seconds"] > GROUP_SECONDS:
        result["status"] = "timeout"
    # A completion time is not a score: a timeout always scores zero, never zero seconds.
    if result["status"] == "passed":
        result["score"] = 1
    write_json(directory / "result.json", result)
    # Export diagnostics after timing; no test work or continuation is allowed here.
    for name in ("test-results", "playwright-report", "playwright-report-functional", "playwright-report-performance", "playwright-report-timeline-capacity"):
        for phase_source in phase_sources.values():
            path = phase_source / "web" / name
            if path.is_dir():
                shutil.copytree(path, directory / "diagnostics" / name, dirs_exist_ok=True)
    shutil.rmtree(directory / "isolated-sources", ignore_errors=True)
    return result


def compare(a: dict, b: dict, manifest: dict) -> dict:
    result = {"passed": False, "reason": "candidate_failed", "delta_seconds": None, "delta_percent": None,
              "delta_is_lower_bound": False, "total_wall_seconds": None,
              "ab_total_budget_seconds": AB_TOTAL_SECONDS,
              "unimplemented_slow_suites": manifest.get("unimplemented_slow_suites", [])}
    if (a.get("manifest_digest") != manifest["digest"] or b.get("manifest_digest") != manifest["digest"]
            or a.get("group") != "A" or b.get("group") != "B"
            or a.get("sha") != manifest["identities"]["A"]["sha"] or b.get("sha") != manifest["identities"]["B"]["sha"]
            or a.get("environment") != b.get("environment")):
        return {**result, "reason": "incomparable_identity_or_environment"}
    def complete(group: dict) -> bool:
        seconds = group.get("wall_seconds", 0)
        return (group.get("status") == "passed" and group.get("reports", {}).get("success")
                and isinstance(seconds, (int, float)) and math.isfinite(seconds) and 0 < seconds <= GROUP_SECONDS
                and [phase.get("name") for phase in group.get("phases", [])] == list(WORKLOAD)
                and all(phase.get("status") == "passed" for phase in group["phases"]))
    if isinstance(a.get("wall_seconds"), (int, float)) and isinstance(b.get("wall_seconds"), (int, float)):
        total = a["wall_seconds"] + b["wall_seconds"]
        if math.isfinite(total):
            result["total_wall_seconds"] = total
            if total > AB_TOTAL_SECONDS:
                result["reason"] = "ab_total_wall_budget_exceeded"
                return result
    if not complete(b):
        return result
    if a.get("status") == "timeout":
        if a.get("wall_seconds", 0) < GROUP_SECONDS:
            return {**result, "reason": "unproven_baseline_deadline"}
        delta = GROUP_SECONDS - b["wall_seconds"]
        return {**result, "passed": True, "reason": "baseline_timeout_candidate_complete", "delta_seconds": delta,
                "delta_percent": delta / GROUP_SECONDS * 100, "delta_is_lower_bound": True}
    if not complete(a):
        return {**result, "reason": "baseline_failed_or_missing"}
    delta = a["wall_seconds"] - b["wall_seconds"]
    percent = delta / a["wall_seconds"] * 100
    # Compare common semantic test identities, allowing the explicit Demo execution-layer migration.
    old = {(c["file"], c["name"], c["status"]) for c in a["reports"]["unit"]["cases"]}
    new = {(c["file"], c["name"], c["status"]) for c in b["reports"]["unit"]["cases"]}
    if not old <= new:
        return {**result, "reason": "candidate_removed_unit_cases", "missing_cases": sorted(old - new)}
    for key in ("storybook",):
        old_cases = {(c["name"], c["status"]) for c in a["reports"][key]["cases"]}
        new_cases = {(c["name"], c["status"]) for c in b["reports"][key]["cases"]}
        if not old_cases <= new_cases:
            return {**result, "reason": "candidate_removed_storybook_cases"}
    for profile in ("pr", "main"):
        old_cases = {(Path(c["file"]).name, c["name"]) for c in a["reports"]["browser"][profile]["cases"]}
        new_cases = {(Path(c["file"]).name, c["name"]) for c in b["reports"]["browser"][profile]["cases"]}
        if not old_cases <= new_cases:
            return {**result, "reason": f"candidate_removed_{profile}_browser_tests"}
    return {**result, "passed": delta >= 5 and percent >= 10, "reason": "improvement_threshold",
            "delta_seconds": delta, "delta_percent": percent}


def markdown(a: dict, b: dict, manifest: dict, verdict: dict) -> str:
    lines = ["# Whole-PR frontend A/B", "", f"Verdict: **{'PASS' if verdict['passed'] else 'FAIL'}** — `{verdict['reason']}`.", "",
             f"A `{a.get('sha')}`; B `{b.get('sha')}`; workload `{manifest['digest']}`.", "",
             f"Each phase has a {GROUP_SECONDS}-second process wall-clock budget; group wall is the maximum phase wall, and A+B group wall must stay within {AB_TOTAL_SECONDS} seconds. Independent CI jobs run concurrently, with all builds and test reports included.", "",
             "| Phase | A seconds | B seconds | A − B seconds |", "|---|---:|---:|---:|"]
    for name in manifest["workload"]:
        left = next((p for p in a.get("phases", []) if p["name"] == name), None)
        right = next((p for p in b.get("phases", []) if p["name"] == name), None)
        delta = f"{left['wall_seconds'] - right['wall_seconds']:.3f}" if left and right and left["status"] == right["status"] == "passed" else "—"
        lines.append(f"| {name} | {left['wall_seconds'] if left else 'not run'} | {right['wall_seconds'] if right else 'not run'} | {delta} |")
    lines += ["", f"A: {a.get('status')}, score {a.get('score', 0)}, max phase {a.get('wall_seconds')} s, phase sum {a.get('phase_wall_seconds_sum')} s, span {a.get('phase_span_seconds')} s.",
              f"B: {b.get('status')}, score {b.get('score', 0)}, max phase {b.get('wall_seconds')} s, phase sum {b.get('phase_wall_seconds_sum')} s, span {b.get('phase_span_seconds')} s.",
              f"A+B measured wall clock: {verdict.get('total_wall_seconds')} s / {AB_TOTAL_SECONDS} s.", "",
              f"Saving {'lower bound' if verdict['delta_is_lower_bound'] else 'measured'}: {verdict['delta_seconds']} s / {verdict['delta_percent']}%.", "",
              "## Four target Vitest suites", "", "| Suite | A seconds | B seconds | Implementation |", "|---|---:|---:|---|"]
    for name in SLOW_SUITES:
        times = [group.get("reports", {}).get("unit", {}).get("suites", {}).get(name, {}).get("seconds", "unavailable") for group in (a, b)]
        lines.append(f"| {name} | {times[0]} | {times[1]} | {'unchanged / not implemented' if name in manifest['unimplemented_slow_suites'] else 'changed'} |")
    audit = manifest["demo_coverage_audit"]
    lines += ["", f"Coverage equivalence verified: {audit['verified']}."] + audit["issues"]
    return "\n".join(lines + ["", "Performance verdict does not certify semantic equivalence or PR readiness. One A/B observation; no seven-round statistical claim or CI queue-time claim.", ""])


def cleanup(run_id: str, archive: Path, repository: str) -> None:
    """Export exact evidence first, then delete only the resolved, completed A/B run."""
    if not run_id.isdecimal() or not re.fullmatch(r"[\w.-]+/[\w.-]+", repository):
        raise ValueError("Exact numeric run ID and owner/repository are required")
    run = json.loads(command_output(["gh", "api", f"repos/{repository}/actions/runs/{run_id}"]))
    if run["status"] != "completed" or run["event"] != "workflow_dispatch" or run["path"] not in (
        ".github/workflows/web-test-ab.yml", ".github/workflows/ci-observability-performance.yml",
    ):
        raise ValueError("Cleanup target is not a completed manual A/B run")
    archive.mkdir(parents=True, exist_ok=False)
    subprocess.run(["gh", "run", "download", run_id, "--repo", repository, "--name", f"web-test-ab-{run_id}", "--dir", str(archive)], check=True)
    summary = json.loads((archive / "summary.json").read_text())
    if str(summary["run_id"]) != run_id:
        raise ValueError("Downloaded evidence is not bound to the cleanup target")
    write_json(archive / "github-run.json", run)
    subprocess.run(["gh", "api", "--method", "DELETE", f"repos/{repository}/actions/runs/{run_id}"], check=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="mode", required=True)
    for mode in ("prepare", "run"):
        command = commands.add_parser(mode)
        command.add_argument("--baseline-src", type=Path, required=True)
        command.add_argument("--candidate-src", type=Path, required=True)
        command.add_argument("--baseline-sha", required=True)
        command.add_argument("--candidate-sha", required=True)
        command.add_argument("--output", type=Path, required=True)
    phase = commands.add_parser("phase")
    phase.add_argument("--group", choices=("A", "B"), required=True)
    phase.add_argument("--phase", dest="phase_name", choices=WORKLOAD, required=True)
    phase.add_argument("--source-src", type=Path, required=True)
    phase.add_argument("--source-sha", required=True)
    phase.add_argument("--manifest", type=Path, required=True)
    phase.add_argument("--output", type=Path, required=True)
    aggregate_command = commands.add_parser("aggregate")
    aggregate_command.add_argument("--manifest", type=Path, required=True)
    aggregate_command.add_argument("--results-root", type=Path, required=True)
    aggregate_command.add_argument("--output", type=Path, required=True)
    clean = commands.add_parser("cleanup")
    clean.add_argument("--run-id", required=True)
    clean.add_argument("--archive", type=Path, required=True)
    clean.add_argument("--repo", required=True)
    args = parser.parse_args()
    if args.mode == "cleanup":
        cleanup(args.run_id, args.archive, args.repo)
        return 0
    actions_only()
    if args.mode == "phase":
        manifest = json.loads(args.manifest.read_text())
        try:
            result = run_phase(args.group, args.source_src.resolve(), args.source_sha, args.phase_name, manifest, args.output.resolve())
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            directory = args.output.resolve() / "results" / args.group / args.phase_name
            directory.mkdir(parents=True, exist_ok=True)
            result = {"name": args.phase_name, "group": args.group, "sha": args.source_sha,
                      "manifest_digest": manifest.get("digest"), "status": "environment_failed", "returncode": 1,
                      "wall_seconds": 0.0, "cpu_seconds": 0.0, "children_peak_rss_kb_cumulative": 0,
                      "reason": str(error)}
            write_json(directory / "result.json", result)
        return phase_exit_code(args.group, result["status"])
    if args.mode == "aggregate":
        manifest = json.loads(args.manifest.read_text())
        summary = aggregate(manifest, args.results_root.resolve(), args.output.resolve())
        return 0 if summary["verdict"]["passed"] else 1
    sources = (args.baseline_src.resolve(), args.candidate_src.resolve())
    output = args.output.resolve()
    manifest = prepare(*sources, args.baseline_sha, args.candidate_sha, output)
    if args.mode == "prepare":
        return 0
    results = []
    for group, source in zip(("A", "B"), sources):
        try:
            results.append(run_group(group, source, manifest, output))
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            failed = {"group": group, "sha": manifest["identities"][group]["sha"], "status": "environment_failed", "score": 0, "reason": str(error)}
            write_json(output / group / "result.json", failed)
            results.append(failed)
    verdict = compare(*results, manifest)
    summary = {"run_id": os.environ["GITHUB_RUN_ID"], "manifest": manifest, "A": results[0], "B": results[1], "verdict": verdict,
               "implementation_ready": verdict["passed"] and manifest["demo_coverage_audit"]["verified"] and not manifest["unimplemented_slow_suites"]}
    write_json(output / "summary.json", summary)
    report = markdown(*results, manifest, verdict)
    (output / "summary.md").write_text(report)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            stream.write(report)
    return 0 if verdict["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
