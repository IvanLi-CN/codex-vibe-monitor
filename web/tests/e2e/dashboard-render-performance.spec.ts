import { expect, type Page, test } from "@playwright/test";

const DASHBOARD_PERFORMANCE_URL =
  "/#/dashboard?demoScene=operational&demoTheme=light&demoViewport=default";
const MEASURED_RUN_COUNT = 5;
const LONG_TASK_LIMIT_MS = 200;

type LongTaskEntry = {
  startTime: number;
  duration: number;
};

type DashboardPerformanceWindow = Window & {
  __dashboardPerformance?: {
    longTasks: LongTaskEntry[];
    longTaskObserverActive: boolean;
    longTaskObserverSupported: boolean;
  };
  __dashboardPerformanceDiagnostics__?: {
    todayChartRenderCount: number;
  };
};

type LongTaskState = {
  entries: LongTaskEntry[];
  observerActive: boolean;
  observerSupported: boolean;
};

type PhaseMetrics = {
  longTaskCount: number;
  maxLongTaskMs: number;
  over50msCount: number;
  totalBlockingTimeMs: number;
  p95LongTaskMs: number;
};

type RunMetrics = {
  run: number;
  dataReady: PhaseMetrics;
  dataUpdate: PhaseMetrics;
};

function summarizeLongTasks(entries: LongTaskEntry[], startTime: number, endTime: number) {
  const durations = entries
    .filter((entry) => entry.startTime >= startTime && entry.startTime < endTime)
    .map((entry) => entry.duration)
    .sort((left, right) => left - right);
  const p95Index = Math.max(0, Math.ceil(durations.length * 0.95) - 1);

  return {
    longTaskCount: durations.length,
    maxLongTaskMs: durations.length > 0 ? Math.max(...durations) : 0,
    over50msCount: durations.filter((duration) => duration > 50).length,
    totalBlockingTimeMs: durations.reduce(
      (total, duration) => total + Math.max(0, duration - 50),
      0,
    ),
    p95LongTaskMs: durations[p95Index] ?? 0,
  } satisfies PhaseMetrics;
}

async function readLongTaskState(page: Page) {
  return page.evaluate(() => {
    const performanceWindow = window as DashboardPerformanceWindow;
    const state = performanceWindow.__dashboardPerformance;
    return {
      entries: state?.longTasks ?? [],
      observerActive: state?.longTaskObserverActive ?? false,
      observerSupported: state?.longTaskObserverSupported ?? false,
    };
  }) as Promise<LongTaskState>;
}

async function readDashboardDiagnostics(page: Page) {
  return page.evaluate(() => {
    const performanceWindow = window as DashboardPerformanceWindow;
    return performanceWindow.__dashboardPerformanceDiagnostics__?.todayChartRenderCount ?? 0;
  });
}

test.describe("Dashboard render performance", () => {
  test.setTimeout(180_000);

  test.beforeAll(() => {
    if (process.env.E2E_PRODUCTION_BUILD !== "1") {
      throw new Error(
        "dashboard-render-performance.spec.ts requires E2E_PRODUCTION_BUILD=1 and a production demo preview",
      );
    }
  });

  test("keeps data-ready and same-scale update renders below the long-task budget", async ({
    page,
  }) => {
    await page.context().addInitScript(() => {
      localStorage.removeItem("dashboard.activityOverview.activeRange.v1");
      localStorage.setItem("dashboard.performanceDiagnostics.enabled.v1", "1");
      const performanceWindow = window as DashboardPerformanceWindow;
      const longTasks: LongTaskEntry[] = [];
      const observerSupported = PerformanceObserver.supportedEntryTypes.includes("longtask");
      performanceWindow.__dashboardPerformance = {
        longTasks,
        longTaskObserverActive: false,
        longTaskObserverSupported: observerSupported,
      };
      performance.clearMarks("dashboard-data-ready-start");
      performance.mark("dashboard-data-ready-start");

      if (observerSupported) {
        const observer = new PerformanceObserver((list) => {
          for (const entry of list.getEntries()) {
            longTasks.push({ startTime: entry.startTime, duration: entry.duration });
          }
        });
        observer.observe({ type: "longtask", buffered: true });
        performanceWindow.__dashboardPerformance.longTaskObserverActive = true;
      }
    });

    const runMetrics: RunMetrics[] = [];
    for (let run = 0; run <= MEASURED_RUN_COUNT; run += 1) {
      const runPage = run === 0 ? page : await page.context().newPage();
      await runPage.setViewportSize({ width: 1440, height: 1000 });
      try {
        await runPage.goto(DASHBOARD_PERFORMANCE_URL, { waitUntil: "domcontentloaded" });
        await expect(runPage.getByTestId("dashboard-working-conversations")).toBeVisible();
        const todayTab = runPage
          .locator('[role="tablist"]')
          .first()
          .locator('[role="tab"]')
          .first();
        await todayTab.click();
        await expect(todayTab).toHaveAttribute("aria-selected", "true");
        await expect(runPage.getByTestId("dashboard-today-activity-chart")).toBeVisible();
        await expect(
          runPage.getByTestId("dashboard-working-conversation-card").first(),
        ).toBeVisible();
        await expect.poll(() => readDashboardDiagnostics(runPage)).toBeGreaterThan(0);

        await runPage.waitForFunction(
          () => performance.getEntriesByName("dashboard-data-ready-start").length > 0,
        );
        const { readyAt, dataReadyStart } = await runPage.evaluate(() => ({
          readyAt: performance.now(),
          dataReadyStart:
            performance.getEntriesByName("dashboard-data-ready-start")[0]?.startTime ?? 0,
        }));
        expect(dataReadyStart).toBeGreaterThan(0);
        const rangeTabs = runPage.locator('[role="tablist"]').first().locator('[role="tab"]');
        const todayRangeTab = rangeTabs.nth(0);
        const yesterdayRangeTab = rangeTabs.nth(1);
        await runPage.evaluate(() => {
          localStorage.removeItem("dashboard.performanceDiagnostics.enabled.v1");
          localStorage.setItem("dashboard.performanceDiagnostics.enabled.v1", "1");
        });
        await expect.poll(() => readDashboardDiagnostics(runPage)).toBe(0);
        const updateStart = await runPage.evaluate(() => performance.now());
        await yesterdayRangeTab.click();
        await expect(yesterdayRangeTab).toHaveAttribute("aria-selected", "true");
        await expect(runPage.getByTestId("dashboard-activity-range-yesterday")).toBeVisible();
        await todayRangeTab.click();
        await expect(todayRangeTab).toHaveAttribute("aria-selected", "true");
        await expect(runPage.getByTestId("dashboard-today-activity-chart")).toBeVisible();
        await expect.poll(() => readDashboardDiagnostics(runPage)).toBeGreaterThan(0);
        const updateEnd = await runPage.evaluate(() => performance.now());
        await runPage.evaluate(() => new Promise((resolve) => setTimeout(resolve, 0)));
        const longTaskState = await readLongTaskState(runPage);
        expect(longTaskState.observerSupported).toBe(true);
        expect(longTaskState.observerActive).toBe(true);

        if (run > 0) {
          runMetrics.push({
            run,
            dataReady: summarizeLongTasks(longTaskState.entries, dataReadyStart, readyAt),
            dataUpdate: summarizeLongTasks(longTaskState.entries, updateStart, updateEnd),
          });
        }
      } finally {
        if (runPage !== page) {
          await runPage.close();
        }
      }
    }

    const allPhases = runMetrics.flatMap((metrics) => [metrics.dataReady, metrics.dataUpdate]);
    expect(runMetrics).toHaveLength(MEASURED_RUN_COUNT);
    const evidence = {
      viewport: "1440x1000",
      productionBuild: true,
      warmupRuns: 1,
      measuredRuns: MEASURED_RUN_COUNT,
      longTaskObserver: "required-and-active",
      runMetrics,
      maxLongTaskMs: Math.max(...allPhases.map((phase) => phase.maxLongTaskMs)),
      maxTotalBlockingTimeMs: Math.max(...allPhases.map((phase) => phase.totalBlockingTimeMs)),
      maxP95LongTaskMs: Math.max(...allPhases.map((phase) => phase.p95LongTaskMs)),
    };
    console.log(`DASHBOARD_PERF_EVIDENCE ${JSON.stringify(evidence)}`);

    expect(evidence.maxLongTaskMs).toBeLessThan(LONG_TASK_LIMIT_MS);
  });
});
