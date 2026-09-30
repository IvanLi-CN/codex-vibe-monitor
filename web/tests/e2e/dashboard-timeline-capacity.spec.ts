import { writeFile } from "node:fs/promises";
import { expect, type Page, test } from "@playwright/test";

const CAPACITY_DASHBOARD_URL =
  "/#/dashboard?demoScene=operational&demoTimelineCapacity=1&demoTheme=dark&demoViewport=default";
const requestedScenarioDurationMs = Number(process.env.E2E_TIMELINE_CAPACITY_DURATION_MS);
const defaultScenarioDurationMs = process.env.CI === "true" ? 60_000 : 5 * 60 * 1_000;
const timelineCapacityEnabled =
  process.env.E2E_TIMELINE_CAPACITY === "1" || process.env.E2E_PRODUCTION_BUILD === "1";
const CAPACITY_SCENARIO_DURATION_MS =
  Number.isFinite(requestedScenarioDurationMs) && requestedScenarioDurationMs > 0
    ? requestedScenarioDurationMs
    : defaultScenarioDurationMs;
const MINIMUM_TRAVERSALS = Math.max(2, Math.floor(CAPACITY_SCENARIO_DURATION_MS / 15_000) - 1);
const MINIMUM_REFRESH_INTERVAL_MS = 15_000;
const REVISION_INTERVAL_MS = 1_000;
const MINIMUM_LIVE_EVENTS = Math.max(
  10,
  Math.floor((CAPACITY_SCENARIO_DURATION_MS / REVISION_INTERVAL_MS) * 0.8),
);
const MINIMUM_LIVE_EVENT_SPAN_MS = CAPACITY_SCENARIO_DURATION_MS * 0.8;
const MAXIMUM_LIVE_EVENT_GAP_MS = Math.max(
  5_000,
  Math.min(15_000, CAPACITY_SCENARIO_DURATION_MS * 0.1),
);
const MINIMUM_TEST_TIMEOUT_MS = 4 * 60 * 1_000;

type TimelineDiagnostics = {
  traversalStarts: number[];
  pageRequests: number;
  releasedSnapshots: number;
  releaseFailures: number;
  timelineHttpFailures: number;
  activeTraversals: number;
  maxActiveTraversals: number;
  nextTraversalDelayMs: number;
  longTasks: Array<{ duration: number; startTime: number }>;
  longTaskObserverSupported: boolean;
  timelineActivationStartMs: number | null;
  timelineReadyAtMs: number | null;
  sseOpenCount: number;
  sseMessageCount: number;
  sseLiveMessageCount: number;
  sseLiveMessageTimesMs: number[];
  sseErrorCount: number;
  lastSseMessageAtMs: number;
  sseSourceUrls: string[];
  demoRevisionTriggers: number;
  demoRevisionTriggerTimesMs: number[];
};

type TimelineDiagnosticsWindow = Window & {
  __CVM_TIMELINE_CAPACITY_DIAGNOSTICS__?: TimelineDiagnostics;
  __CVM_DEMO_TRIGGER_TIMESERIES_UPDATE__?: () => void;
  __CVM_DEMO_TIMESERIES_REVISION__?: number;
};

test.use({ video: "off" });

function installDiagnostics(page: Page, initialRange: "today" | "yesterday" | "7d" = "today") {
  return page.addInitScript((range) => {
    window.localStorage.setItem("dashboard.activityOverview.activeRange.v1", range);
    const diagnostics: TimelineDiagnostics = {
      traversalStarts: [],
      pageRequests: 0,
      releasedSnapshots: 0,
      releaseFailures: 0,
      timelineHttpFailures: 0,
      activeTraversals: 0,
      maxActiveTraversals: 0,
      nextTraversalDelayMs: 0,
      longTasks: [],
      longTaskObserverSupported: PerformanceObserver.supportedEntryTypes.includes("longtask"),
      timelineActivationStartMs: null,
      timelineReadyAtMs: null,
      sseOpenCount: 0,
      sseMessageCount: 0,
      sseLiveMessageCount: 0,
      sseLiveMessageTimesMs: [],
      sseErrorCount: 0,
      lastSseMessageAtMs: 0,
      sseSourceUrls: [],
      demoRevisionTriggers: 0,
      demoRevisionTriggerTimesMs: [],
    };
    const instrumentedWindow = window as TimelineDiagnosticsWindow;
    instrumentedWindow.__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__ = diagnostics;
    let demoRevisionTrigger: (() => void) | undefined;
    Object.defineProperty(instrumentedWindow, "__CVM_DEMO_TRIGGER_TIMESERIES_UPDATE__", {
      configurable: true,
      get: () => demoRevisionTrigger,
      set: (trigger: (() => void) | undefined) => {
        demoRevisionTrigger =
          typeof trigger === "function"
            ? () => {
                diagnostics.demoRevisionTriggers += 1;
                diagnostics.demoRevisionTriggerTimesMs.push(performance.now());
                trigger();
              }
            : trigger;
      },
    });
    let demoEventSourceFactory: ((path: string) => EventSource) | undefined;
    Object.defineProperty(instrumentedWindow, "__CVM_DEMO_CREATE_EVENT_SOURCE__", {
      configurable: true,
      get: () => demoEventSourceFactory,
      set: (factory: ((path: string) => EventSource) | undefined) => {
        if (typeof factory !== "function") {
          demoEventSourceFactory = factory;
          return;
        }
        demoEventSourceFactory = (path) => {
          diagnostics.sseSourceUrls.push(path);
          const eventSource = factory(path);
          eventSource.addEventListener("open", () => {
            diagnostics.sseOpenCount += 1;
          });
          eventSource.addEventListener("message", (event) => {
            diagnostics.sseMessageCount += 1;
            diagnostics.lastSseMessageAtMs = performance.now();
            try {
              if (JSON.parse((event as MessageEvent<string>).data).type === "live") {
                diagnostics.sseLiveMessageCount += 1;
                diagnostics.sseLiveMessageTimesMs.push(performance.now());
              }
            } catch {
              // Keep collecting counters when an unrelated demo event has another payload shape.
            }
          });
          eventSource.addEventListener("error", () => {
            diagnostics.sseErrorCount += 1;
          });
          return eventSource;
        };
      },
    });
    if (diagnostics.longTaskObserverSupported) {
      const observer = new PerformanceObserver((list) => {
        diagnostics.longTasks.push(
          ...list.getEntries().map((entry) => ({
            duration: entry.duration,
            startTime: entry.startTime,
          })),
        );
      });
      observer.observe({ type: "longtask", buffered: true });
    }
  }, initialRange);
}

function expectSustainedEvents(eventTimes: number[], baselineCount: number): void {
  const scenarioEvents = eventTimes.slice(baselineCount);
  expect(scenarioEvents.length).toBeGreaterThanOrEqual(MINIMUM_LIVE_EVENTS);
  expect(scenarioEvents.at(-1)! - scenarioEvents[0]!).toBeGreaterThanOrEqual(
    MINIMUM_LIVE_EVENT_SPAN_MS,
  );
  const maximumGapMs = Math.max(
    0,
    ...scenarioEvents.slice(1).map((time, index) => time - scenarioEvents[index]!),
  );
  expect(maximumGapMs).toBeLessThanOrEqual(MAXIMUM_LIVE_EVENT_GAP_MS);
}

async function expectVisibleCallsAreVirtualized(page: Page, totalCalls: number): Promise<void> {
  const laneScroll = page.getByTestId("dashboard-invocation-timeline-lane-scroll");
  const geometry = await laneScroll.evaluate((element) => {
    const viewport = element as HTMLElement;
    const bars = Array.from(viewport.querySelectorAll<HTMLElement>("[data-call-value]"))
      .map((bar) => {
        const rect = bar.getBoundingClientRect();
        return { top: rect.top, height: rect.height };
      })
      .sort((left, right) => left.top - right.top);
    return {
      clientHeight: viewport.clientHeight,
      scrollHeight: viewport.scrollHeight,
      bars,
    };
  });

  expect(geometry.bars.length).toBeGreaterThan(0);
  expect(geometry.bars.length).toBeLessThanOrEqual(Math.ceil(geometry.clientHeight / 9) + 4);
  expect(geometry.bars.length).toBeLessThan(totalCalls);
  expect(geometry.scrollHeight).toBeGreaterThan(geometry.clientHeight);
  for (const bar of geometry.bars) {
    expect(bar.height).toBeGreaterThanOrEqual(8);
    expect(bar.height).toBeLessThanOrEqual(16);
  }
  for (let index = 1; index < geometry.bars.length; index += 1) {
    const previous = geometry.bars[index - 1]!;
    const current = geometry.bars[index]!;
    expect(Math.round((current.top - previous.top - previous.height) * 1000) / 1000).toBe(1);
  }
}

async function instrumentTimelineFetch(page: Page) {
  await page.evaluate(() => {
    const diagnostics = (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__;
    if (!diagnostics) throw new Error("Timeline diagnostics are unavailable");
    const originalFetch = window.fetch.bind(window);
    window.fetch = async (input, init) => {
      const requestUrl = new URL(
        input instanceof Request ? input.url : input.toString(),
        window.location.href,
      );
      const method = (
        init?.method ?? (input instanceof Request ? input.method : "GET")
      ).toUpperCase();
      const isTimeline = requestUrl.pathname.startsWith("/api/stats/invocation-timeline");
      if (isTimeline && method === "GET") {
        diagnostics.pageRequests += 1;
        if (!requestUrl.searchParams.has("cursor")) {
          diagnostics.traversalStarts.push(performance.now());
          diagnostics.activeTraversals += 1;
          diagnostics.maxActiveTraversals = Math.max(
            diagnostics.maxActiveTraversals,
            diagnostics.activeTraversals,
          );
          const delayMs = diagnostics.nextTraversalDelayMs;
          diagnostics.nextTraversalDelayMs = 0;
          if (delayMs > 0) {
            await new Promise<void>((resolve) => window.setTimeout(resolve, delayMs));
          }
        }
      }
      const response = await originalFetch(input, init);
      if (isTimeline && response.status >= 400) diagnostics.timelineHttpFailures += 1;
      if (isTimeline && method === "DELETE") {
        if (response.status === 204) diagnostics.releasedSnapshots += 1;
        else diagnostics.releaseFailures += 1;
        diagnostics.activeTraversals = Math.max(0, diagnostics.activeTraversals - 1);
      }
      return response;
    };
  });
}

async function startCapacityDashboard(
  page: Page,
  viewport: { width: number; height: number },
  measureInitialTimelineMount = false,
) {
  await installDiagnostics(page, measureInitialTimelineMount ? "7d" : "today");
  await page.setViewportSize(viewport);
  await page.goto(CAPACITY_DASHBOARD_URL, { waitUntil: "domcontentloaded" });
  if (viewport.width < 769) {
    const rangeSelect = page.getByTestId("dashboard-activity-range-select");
    const metricSelect = page.getByTestId("dashboard-activity-metric-select");
    await expect(metricSelect).toHaveText(/count|次数/i);
    if (measureInitialTimelineMount) {
      await expect(rangeSelect).toHaveText(/7.?day|7.?天|7.?日|7d/i);
      await page.evaluate(() => {
        const diagnostics = (window as TimelineDiagnosticsWindow)
          .__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__;
        if (!diagnostics) throw new Error("Timeline diagnostics are unavailable");
        diagnostics.timelineActivationStartMs = performance.now();
      });
      await rangeSelect.click();
      await page.getByRole("option").first().click();
    }
    await expect(rangeSelect).toHaveText(/today|今日/i);
  } else {
    const todayTab = page.locator('[role="tablist"]').first().locator('[role="tab"]').first();
    if (measureInitialTimelineMount) {
      await expect(
        page.locator('[role="tablist"]').first().locator('[role="tab"]').nth(3),
      ).toHaveAttribute("aria-selected", "true");
      await expect(todayTab).toHaveAttribute("aria-selected", "false");
      await page.evaluate(() => {
        const diagnostics = (window as TimelineDiagnosticsWindow)
          .__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__;
        if (!diagnostics) throw new Error("Timeline diagnostics are unavailable");
        diagnostics.timelineActivationStartMs = performance.now();
      });
    }
    await todayTab.click();
    await expect(todayTab).toHaveAttribute("aria-selected", "true");
  }
  await expect(page.getByTestId("dashboard-invocation-timeline")).toBeVisible();
  const lanes = page.getByTestId("dashboard-invocation-timeline-lanes");
  await expect(lanes).toHaveAttribute("data-total-calls", "550");
  await expect
    .poll(() =>
      page
        .getByTestId("dashboard-invocation-timeline-lane-scroll")
        .locator("[data-call-value]")
        .count(),
    )
    .toBeGreaterThan(0);
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() =>
          requestAnimationFrame(() => {
            const diagnostics = (window as TimelineDiagnosticsWindow)
              .__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__;
            if (!diagnostics) throw new Error("Timeline diagnostics are unavailable");
            diagnostics.timelineReadyAtMs = performance.now();
            resolve();
          }),
        ),
      ),
  );
  await expect
    .poll(
      () =>
        page.evaluate(
          () =>
            (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__
              ?.sseOpenCount ?? 0,
        ),
      { timeout: 30_000 },
    )
    .toBeGreaterThan(0);
  await expect
    .poll(
      () =>
        page.evaluate(
          () =>
            (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__
              ?.sseMessageCount ?? 0,
        ),
      { timeout: 30_000 },
    )
    .toBeGreaterThan(0);
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const diagnostics = (window as TimelineDiagnosticsWindow)
            .__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__;
          return (
            diagnostics != null &&
            diagnostics.sseErrorCount === 0 &&
            diagnostics.lastSseMessageAtMs > 0 &&
            performance.now() - diagnostics.lastSseMessageAtMs >= 500
          );
        }),
      { timeout: 30_000 },
    )
    .toBe(true);
}

test("two clients release paged snapshots during high-frequency revisions", async ({
  browser,
  page,
}, testInfo) => {
  test.skip(
    !timelineCapacityEnabled,
    "Two-client capacity proof runs in production-build CI and explicit testbox validation.",
  );
  test.setTimeout(
    Math.max(MINIMUM_TEST_TIMEOUT_MS, CAPACITY_SCENARIO_DURATION_MS * 2 + 3 * 60 * 1_000),
  );
  const secondContext = await browser.newContext({ viewport: { width: 393, height: 852 } });
  const secondPage = await secondContext.newPage();
  const pages = [page, secondPage];
  let revisionInterval: ReturnType<typeof setInterval> | null = null;
  let revisionTriggerInFlight = false;
  const viewports = [
    { width: 1440, height: 1000 },
    { width: 393, height: 852 },
  ];
  try {
    await Promise.all(
      pages.map((clientPage, index) => startCapacityDashboard(clientPage, viewports[index]!)),
    );
    await Promise.all(pages.map(instrumentTimelineFetch));
    for (const [index, clientPage] of pages.entries()) {
      const timeline = clientPage.getByTestId("dashboard-invocation-timeline");
      await timeline.scrollIntoViewIfNeeded();
      await testInfo.attach(
        index === 0 ? "timeline-capacity-desktop" : "timeline-capacity-mobile",
        {
          body: await clientPage.screenshot(),
          contentType: "image/png",
        },
      );
    }
    const initialDesktopDiagnostics = await page.evaluate(
      () => (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__,
    );
    const initialClientDiagnostics = await Promise.all(
      pages.map((clientPage) =>
        clientPage.evaluate(
          () => (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__,
        ),
      ),
    );
    const releaseCountBeforeDelay = initialDesktopDiagnostics?.releasedSnapshots ?? 0;
    const traversalCountBeforeDelay = initialDesktopDiagnostics?.traversalStarts.length ?? 0;
    const liveMessageCountBeforeRevision = initialDesktopDiagnostics?.sseLiveMessageCount ?? 0;
    const revisionTriggerCountBeforeRevision = initialDesktopDiagnostics?.demoRevisionTriggers ?? 0;
    await page.evaluate(() => {
      const diagnostics = (window as TimelineDiagnosticsWindow)
        .__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__;
      if (!diagnostics) throw new Error("Timeline diagnostics are unavailable");
      diagnostics.nextTraversalDelayMs = 20_000;
    });
    const triggerRevision = async () =>
      Promise.all(
        pages.map((clientPage) =>
          clientPage.evaluate(() => {
            const trigger = (window as TimelineDiagnosticsWindow)
              .__CVM_DEMO_TRIGGER_TIMESERIES_UPDATE__;
            if (!trigger) throw new Error("Demo live revision trigger is unavailable");
            trigger();
          }),
        ),
      );
    revisionInterval = setInterval(() => {
      if (revisionTriggerInFlight) return;
      revisionTriggerInFlight = true;
      void triggerRevision()
        .catch((error: unknown) => {
          console.warn("[timeline-capacity] revision trigger failed", error);
        })
        .finally(() => {
          revisionTriggerInFlight = false;
        });
    }, REVISION_INTERVAL_MS);
    const startedAt = Date.now();
    await expect
      .poll(
        () =>
          page.evaluate(
            () =>
              (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__
                ?.demoRevisionTriggers ?? 0,
          ),
        { timeout: 15_000 },
      )
      .toBeGreaterThan(revisionTriggerCountBeforeRevision);
    await expect
      .poll(
        () =>
          page.evaluate(
            () =>
              (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__
                ?.sseLiveMessageCount ?? 0,
          ),
        { timeout: 15_000 },
      )
      .toBeGreaterThan(liveMessageCountBeforeRevision);
    await expect
      .poll(
        () =>
          page.evaluate(() => {
            const diagnostics = (window as TimelineDiagnosticsWindow)
              .__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__;
            return diagnostics?.traversalStarts.length ?? 0;
          }),
        { timeout: 45_000 },
      )
      .toBeGreaterThan(traversalCountBeforeDelay);
    await expect
      .poll(
        () =>
          page.evaluate(() => {
            const diagnostics = (window as TimelineDiagnosticsWindow)
              .__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__;
            return diagnostics?.activeTraversals ?? 0;
          }),
        { timeout: 45_000 },
      )
      .toBe(1);
    const lanesDuringDelayedRefresh = page.getByTestId("dashboard-invocation-timeline-lanes");
    await expect(lanesDuringDelayedRefresh).toHaveAttribute("data-total-calls", "550");
    await expectVisibleCallsAreVirtualized(page, 550);
    const releasesDuringDelayedRefresh = await page.evaluate(
      () =>
        (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__
          ?.releasedSnapshots ?? 0,
    );
    expect(releasesDuringDelayedRefresh).toBe(releaseCountBeforeDelay);
    await expect
      .poll(
        () =>
          page.evaluate(() => {
            const diagnostics = (window as TimelineDiagnosticsWindow)
              .__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__;
            return diagnostics?.releasedSnapshots ?? 0;
          }),
        { timeout: 45_000 },
      )
      .toBeGreaterThan(releaseCountBeforeDelay);
    const elapsedBeforeSteadyState = Date.now() - startedAt;
    await new Promise<void>((resolve) => {
      setTimeout(resolve, Math.max(0, CAPACITY_SCENARIO_DURATION_MS - elapsedBeforeSteadyState));
    });
    await new Promise<void>((resolve) => {
      setTimeout(resolve, 16_000);
    });
    const results = await Promise.all(
      pages.map((clientPage) =>
        clientPage.evaluate(
          () => (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__,
        ),
      ),
    );
    for (const [index, diagnostics] of results.entries()) {
      await testInfo.attach(`timeline-capacity-client-${index + 1}.json`, {
        body: JSON.stringify(diagnostics, null, 2),
        contentType: "application/json",
      });
    }
    for (const [index, diagnostics] of results.entries()) {
      const baseline = initialClientDiagnostics[index];
      expect(diagnostics?.traversalStarts.length).toBeGreaterThanOrEqual(MINIMUM_TRAVERSALS);
      expect(diagnostics?.releasedSnapshots).toBe(diagnostics?.traversalStarts.length);
      expect(diagnostics?.releaseFailures).toBe(0);
      expect(diagnostics?.timelineHttpFailures).toBe(0);
      expect(diagnostics?.sseErrorCount).toBe(0);
      expectSustainedEvents(
        diagnostics?.demoRevisionTriggerTimesMs ?? [],
        baseline?.demoRevisionTriggerTimesMs.length ?? 0,
      );
      expectSustainedEvents(
        diagnostics?.sseLiveMessageTimesMs ?? [],
        baseline?.sseLiveMessageTimesMs.length ?? 0,
      );
      expect(diagnostics?.pageRequests).toBeGreaterThanOrEqual(
        (diagnostics?.traversalStarts.length ?? 0) * 2,
      );
      expect(diagnostics?.maxActiveTraversals).toBe(1);
      expect(
        diagnostics?.traversalStarts.every(
          (startedAt, index) =>
            index === 0 ||
            startedAt - diagnostics.traversalStarts[index - 1]! >= MINIMUM_REFRESH_INTERVAL_MS,
        ),
      ).toBe(true);
    }
  } finally {
    if (revisionInterval != null) clearInterval(revisionInterval);
    try {
      const diagnostics = await Promise.all(
        pages.map((clientPage) =>
          clientPage
            .evaluate(
              () => (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__,
            )
            .catch(() => null),
        ),
      );
      await writeFile(
        testInfo.outputPath("timeline-capacity-final-diagnostics.json"),
        JSON.stringify(diagnostics, null, 2),
        "utf8",
      );
      await testInfo.attach("timeline-capacity-final-diagnostics.json", {
        body: JSON.stringify(diagnostics, null, 2),
        contentType: "application/json",
      });
    } catch {
      // The browser context may already be closing after a test timeout.
    }
    await secondContext.close().catch(() => undefined);
  }
});

test("single-client dense timeline stays below the long-task budget on desktop and mobile", async ({
  browser,
}, testInfo) => {
  test.skip(
    !timelineCapacityEnabled,
    "Timeline render performance proof runs in production-build CI and explicit testbox validation.",
  );
  test.setTimeout(120_000);
  for (const [name, viewport] of [
    ["desktop", { width: 1440, height: 1000 }],
    ["mobile", { width: 393, height: 852 }],
  ] as const) {
    const context = await browser.newContext({ viewport });
    const page = await context.newPage();
    try {
      await startCapacityDashboard(page, viewport, true);
      await page.getByTestId("dashboard-invocation-timeline").scrollIntoViewIfNeeded();
      await instrumentTimelineFetch(page);

      const initialDiagnostics = await page.evaluate(
        () => (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__,
      );
      expect(initialDiagnostics?.longTaskObserverSupported).toBe(true);
      expect(initialDiagnostics?.timelineActivationStartMs).not.toBeNull();
      expect(initialDiagnostics?.timelineReadyAtMs).not.toBeNull();
      const timelineMountStart = initialDiagnostics?.timelineActivationStartMs ?? 0;
      const timelineReadyAt = initialDiagnostics?.timelineReadyAtMs ?? 0;
      const timelineMountLongTasks = (initialDiagnostics?.longTasks ?? []).filter(
        (task) =>
          task.startTime < timelineReadyAt && task.startTime + task.duration > timelineMountStart,
      );
      const timelineMountMaxLongTaskMs = Math.max(
        0,
        ...timelineMountLongTasks.map((task) => task.duration),
      );

      await page.evaluate(() => {
        const diagnostics = (window as TimelineDiagnosticsWindow)
          .__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__;
        if (!diagnostics) throw new Error("Timeline diagnostics are unavailable");
        diagnostics.nextTraversalDelayMs = 20_000;
      });
      const beforeRefresh = await page.evaluate(() => {
        const diagnostics = (window as TimelineDiagnosticsWindow)
          .__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__;
        if (!diagnostics) throw new Error("Timeline diagnostics are unavailable");
        return {
          releaseCount: diagnostics.releasedSnapshots,
          liveMessages: diagnostics.sseLiveMessageCount,
          triggerCount: diagnostics.demoRevisionTriggers,
          refreshStart: performance.now(),
        };
      });
      await page.evaluate(() => {
        const trigger = (window as TimelineDiagnosticsWindow)
          .__CVM_DEMO_TRIGGER_TIMESERIES_UPDATE__;
        if (!trigger) throw new Error("Demo live revision trigger is unavailable");
        trigger();
      });
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__
                ?.demoRevisionTriggers ?? 0,
          ),
        )
        .toBeGreaterThan(beforeRefresh.triggerCount);
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__
                ?.sseLiveMessageCount ?? 0,
          ),
        )
        .toBeGreaterThan(beforeRefresh.liveMessages);
      await expect
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
      await page.evaluate(
        () =>
          new Promise<void>((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          ),
      );
      const refreshEnd = await page.evaluate(() => performance.now());
      const diagnostics = await page.evaluate(
        () => (window as TimelineDiagnosticsWindow).__CVM_TIMELINE_CAPACITY_DIAGNOSTICS__,
      );
      const refreshLongTasks = (diagnostics?.longTasks ?? []).filter(
        (task) =>
          task.startTime < refreshEnd &&
          task.startTime + task.duration > beforeRefresh.refreshStart,
      );
      const maxRefreshLongTaskMs = Math.max(0, ...refreshLongTasks.map((task) => task.duration));
      const screenshot = await page.screenshot();
      const evidence = {
        viewport: name,
        timelineMountMaxLongTaskMs,
        refreshMaxLongTaskMs: maxRefreshLongTaskMs,
        timelineMountLongTasks,
        longTasks: diagnostics?.longTasks ?? [],
        traversalCount: diagnostics?.traversalStarts.length ?? 0,
        pageRequests: diagnostics?.pageRequests ?? 0,
        releasedSnapshots: diagnostics?.releasedSnapshots ?? 0,
        releaseFailures: diagnostics?.releaseFailures ?? 0,
        timelineHttpFailures: diagnostics?.timelineHttpFailures ?? 0,
        sseLiveMessages: diagnostics?.sseLiveMessageCount ?? 0,
      };
      await writeFile(testInfo.outputPath(`timeline-dense-${name}.png`), screenshot);
      await writeFile(
        testInfo.outputPath(`timeline-dense-${name}-diagnostics.json`),
        JSON.stringify(evidence, null, 2),
        "utf8",
      );
      await testInfo.attach(`timeline-dense-${name}`, {
        body: screenshot,
        contentType: "image/png",
      });
      await testInfo.attach(`timeline-dense-${name}-diagnostics.json`, {
        body: JSON.stringify(evidence, null, 2),
        contentType: "application/json",
      });
      expect(diagnostics?.traversalStarts.length).toBeGreaterThanOrEqual(2);
      expect(diagnostics?.releasedSnapshots).toBe(diagnostics?.traversalStarts.length);
      expect(diagnostics?.maxActiveTraversals).toBe(1);
      expect(
        diagnostics?.traversalStarts.every(
          (startedAt, index) =>
            index === 0 ||
            startedAt - diagnostics.traversalStarts[index - 1]! >= MINIMUM_REFRESH_INTERVAL_MS,
        ),
      ).toBe(true);
      expect(diagnostics?.pageRequests).toBeGreaterThanOrEqual(
        (diagnostics?.traversalStarts.length ?? 0) * 2,
      );
      expect(diagnostics?.releaseFailures).toBe(0);
      expect(diagnostics?.timelineHttpFailures).toBe(0);
      expect(timelineMountMaxLongTaskMs).toBeLessThan(200);
      expect(maxRefreshLongTaskMs).toBeLessThan(200);
    } finally {
      await context.close();
    }
  }
});
