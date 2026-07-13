import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useLayoutEffect, useRef, useState } from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import { DashboardActivityOverview } from "./DashboardActivityOverview";
import { DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY } from "./dashboardActivityRange";

type SummaryKey = "today" | "yesterday" | "previous7d" | "1d" | "7d";
type TimeseriesKey = "today:1m" | "yesterday:1m" | "1d:1m" | "7d:1h" | "6mo:1d";
type PersistedRange = "today" | "yesterday" | "1d" | "7d" | "usage" | null;
type SummaryFixture = ReturnType<typeof createSummary>;
type TimeseriesFixture =
  | ReturnType<typeof buildTodayMinutePoints>
  | ReturnType<typeof build24HourPoints>
  | ReturnType<typeof buildHourlyPoints>
  | ReturnType<typeof buildDailyPoints>;
type WindowWithDashboardFetchLog = Window & {
  __dashboardOverviewFetchLog__?: string[];
};
type DashboardOverviewParameters = {
  persistedRange?: PersistedRange;
  failTodayTimeseries?: boolean;
  summaryOverrides?: Partial<Record<SummaryKey, SummaryFixture>>;
  timeseriesOverrides?: Partial<Record<TimeseriesKey, TimeseriesFixture>>;
  delaySummaryWindows?: SummaryKey[];
  responseDelayMs?: number;
};

type AccountActivityOverviewProps = {
  title: string;
  upstreamAccountId: number;
  testId: string;
};

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}

function createSummary(
  totalCount: number,
  successCount: number,
  failureCount: number,
  totalCost: number,
  totalTokens: number,
) {
  return {
    totalCount,
    successCount,
    failureCount,
    totalCost,
    totalTokens,
    inProgressConversationCount: Math.max(1, Math.round(successCount / 330)),
    inProgressRetryConversationCount: Math.max(0, Math.round(failureCount / 44)),
    inProgressAvgWaitMs: 1400 + Math.round(failureCount * 3.5),
    nonSuccessCost: Number((totalCost * 0.031).toFixed(2)),
    nonSuccessTokens: Math.max(0, Math.round(totalTokens * 0.024)),
  };
}

function deriveNonSuccessCost(totalCost: number, failureCount: number, totalCount: number) {
  if (totalCost <= 0 || failureCount <= 0 || totalCount <= 0) {
    return 0;
  }

  return Number(((totalCost * failureCount) / totalCount).toFixed(2));
}

const TODAY_SUMMARY_FIXTURE = createSummary(3428, 3296, 132, 42.86, 18764200);
const YESTERDAY_SUMMARY_FIXTURE = createSummary(4876, 4718, 158, 61.72, 26918400);

function buildTodayMinutePoints(summary = TODAY_SUMMARY_FIXTURE) {
  const rangeStart = new Date("2026-04-09T00:00:00+08:00");
  const rangeEnd = new Date("2026-04-09T12:24:00+08:00");
  const points: Array<Record<string, number | string | null>> = [];
  const minuteCount = Math.floor((rangeEnd.getTime() - rangeStart.getTime()) / 60_000) + 1;
  const minuteIndexes = Array.from({ length: minuteCount }, (_, index) => index);
  const successCounts = distributeInteger(
    summary.successCount,
    minuteIndexes.map((index) => buildActivityWeight(index, "success")),
  );
  const failureCounts = distributeInteger(
    summary.failureCount,
    minuteIndexes.map((index) => buildActivityWeight(index, "failure")),
  );
  const totalTokens = distributeInteger(
    summary.totalTokens,
    minuteIndexes.map((index) =>
      buildUsageWeight(successCounts[index] + failureCounts[index], index, "tokens"),
    ),
  );
  const totalCostCents = distributeInteger(
    Math.round(summary.totalCost * 100),
    minuteIndexes.map((index) =>
      buildUsageWeight(successCounts[index] + failureCounts[index], index, "cost"),
    ),
  );

  for (let minute = 0; minute < minuteCount; minute += 1) {
    const bucketStart = new Date(rangeStart.getTime() + minute * 60_000);
    const bucketEnd = new Date(bucketStart.getTime() + 60_000);
    const successCount = successCounts[minute] ?? 0;
    const failureCount = failureCounts[minute] ?? 0;
    const totalCount = successCount + failureCount;
    const firstResponseByteTotalAvgMs =
      totalCount > 0 ? buildLatencyMs(minute, totalCount, 0) : null;
    points.push({
      bucketStart: bucketStart.toISOString(),
      bucketEnd: bucketEnd.toISOString(),
      totalCount,
      successCount,
      failureCount,
      totalTokens: totalTokens[minute] ?? 0,
      cacheInputTokens: Math.round((totalTokens[minute] ?? 0) * 0.24),
      totalCost: Number(((totalCostCents[minute] ?? 0) / 100).toFixed(2)),
      nonSuccessCost: deriveNonSuccessCost(
        Number(((totalCostCents[minute] ?? 0) / 100).toFixed(2)),
        failureCount,
        totalCount,
      ),
      firstResponseByteTotalSampleCount: totalCount,
      avgTotalMs:
        firstResponseByteTotalAvgMs == null
          ? null
          : buildTotalLatencyMs(firstResponseByteTotalAvgMs, minute, 0),
      totalLatencySampleCount: totalCount,
      firstResponseByteTotalAvgMs,
    });
  }

  return {
    rangeStart: rangeStart.toISOString(),
    rangeEnd: rangeEnd.toISOString(),
    bucketSeconds: 60,
    points,
  };
}

function buildYesterdayMinutePoints(summary = YESTERDAY_SUMMARY_FIXTURE) {
  const rangeStart = new Date("2026-04-08T00:00:00+08:00");
  const activityEnd = new Date("2026-04-08T18:36:00+08:00");
  const rangeEnd = new Date("2026-04-09T00:00:00+08:00");
  const points: Array<Record<string, number | string | null>> = [];
  const minuteCount = Math.floor((activityEnd.getTime() - rangeStart.getTime()) / 60_000) + 1;
  const minuteIndexes = Array.from({ length: minuteCount }, (_, index) => index);
  const successCounts = distributeInteger(
    summary.successCount,
    minuteIndexes.map((index) => buildActivityWeight(index + 17, "success")),
  );
  const failureCounts = distributeInteger(
    summary.failureCount,
    minuteIndexes.map((index) => buildActivityWeight(index + 17, "failure")),
  );
  const totalTokens = distributeInteger(
    summary.totalTokens,
    minuteIndexes.map((index) =>
      buildUsageWeight(successCounts[index] + failureCounts[index], index + 9, "tokens"),
    ),
  );
  const totalCostCents = distributeInteger(
    Math.round(summary.totalCost * 100),
    minuteIndexes.map((index) =>
      buildUsageWeight(successCounts[index] + failureCounts[index], index + 9, "cost"),
    ),
  );

  for (let minute = 0; minute < minuteCount; minute += 1) {
    const bucketStart = new Date(rangeStart.getTime() + minute * 60_000);
    const bucketEnd = new Date(bucketStart.getTime() + 60_000);
    const successCount = successCounts[minute] ?? 0;
    const failureCount = failureCounts[minute] ?? 0;
    const totalCount = successCount + failureCount;
    const firstResponseByteTotalAvgMs =
      totalCount > 0 ? buildLatencyMs(minute, totalCount, 36) : null;
    points.push({
      bucketStart: bucketStart.toISOString(),
      bucketEnd: bucketEnd.toISOString(),
      totalCount,
      successCount,
      failureCount,
      totalTokens: totalTokens[minute] ?? 0,
      cacheInputTokens: Math.round((totalTokens[minute] ?? 0) * 0.19),
      totalCost: Number(((totalCostCents[minute] ?? 0) / 100).toFixed(2)),
      nonSuccessCost: deriveNonSuccessCost(
        Number(((totalCostCents[minute] ?? 0) / 100).toFixed(2)),
        failureCount,
        totalCount,
      ),
      firstResponseByteTotalSampleCount: totalCount,
      avgTotalMs:
        firstResponseByteTotalAvgMs == null
          ? null
          : buildTotalLatencyMs(firstResponseByteTotalAvgMs, minute, 36),
      totalLatencySampleCount: totalCount,
      firstResponseByteTotalAvgMs,
    });
  }

  return {
    rangeStart: rangeStart.toISOString(),
    rangeEnd: rangeEnd.toISOString(),
    bucketSeconds: 60,
    points,
  };
}

function build24HourPoints() {
  const end = new Date("2026-04-09T12:20:00+08:00");
  const start = new Date(end.getTime() - 24 * 60 * 60_000);
  const points: Array<Record<string, number | string | null>> = [];
  for (let index = 0; index < 24 * 60; index += 1) {
    const bucketStart = new Date(start.getTime() + index * 60_000);
    const bucketEnd = new Date(bucketStart.getTime() + 60_000);
    const totalCount = index % 17 === 0 ? 0 : index % 6;
    const failureCount = totalCount > 0 && index % 19 === 0 ? 1 : 0;
    const successCount = Math.max(totalCount - failureCount, 0);
    const firstResponseByteTotalAvgMs = totalCount > 0 ? 620 + ((index * 13) % 280) : null;
    points.push({
      bucketStart: bucketStart.toISOString(),
      bucketEnd: bucketEnd.toISOString(),
      totalCount,
      successCount,
      failureCount,
      totalTokens: totalCount * 390,
      cacheInputTokens: totalCount * 64,
      totalCost: Number((totalCount * 0.017).toFixed(4)),
      nonSuccessCost: deriveNonSuccessCost(
        Number((totalCount * 0.017).toFixed(4)),
        failureCount,
        totalCount,
      ),
      firstResponseByteTotalSampleCount: totalCount,
      avgTotalMs:
        firstResponseByteTotalAvgMs == null
          ? null
          : buildTotalLatencyMs(firstResponseByteTotalAvgMs, index, 9),
      totalLatencySampleCount: totalCount,
      firstResponseByteTotalAvgMs,
    });
  }
  return {
    rangeStart: start.toISOString(),
    rangeEnd: end.toISOString(),
    bucketSeconds: 60,
    points,
  };
}

function buildHourlyPoints() {
  const end = new Date("2026-04-09T00:00:00+08:00");
  const start = new Date(end.getTime() - 7 * 24 * 60 * 60_000);
  const points: Array<Record<string, number | string | null>> = [];
  for (let index = 0; index < 7 * 24; index += 1) {
    const bucketStart = new Date(start.getTime() + index * 60 * 60_000);
    const bucketEnd = new Date(bucketStart.getTime() + 60 * 60_000);
    const hour = bucketStart.getHours();
    const day = bucketStart.getDay();
    const density = ((hour + 3) * (day + 2)) % 9;
    const firstResponseByteTotalAvgMs = density > 0 ? 700 + ((index * 23) % 300) : null;
    points.push({
      bucketStart: bucketStart.toISOString(),
      bucketEnd: bucketEnd.toISOString(),
      totalCount: density,
      successCount: Math.max(density - (density > 6 ? 1 : 0), 0),
      failureCount: density > 6 ? 1 : 0,
      totalTokens: density * 620,
      cacheInputTokens: density * 110,
      totalCost: Number((density * 0.23).toFixed(2)),
      nonSuccessCost: deriveNonSuccessCost(
        Number((density * 0.23).toFixed(2)),
        density > 6 ? 1 : 0,
        density,
      ),
      firstResponseByteTotalSampleCount: density,
      avgTotalMs:
        firstResponseByteTotalAvgMs == null
          ? null
          : buildTotalLatencyMs(firstResponseByteTotalAvgMs, index, 21),
      totalLatencySampleCount: density,
      firstResponseByteTotalAvgMs,
    });
  }
  return {
    rangeStart: start.toISOString(),
    rangeEnd: end.toISOString(),
    bucketSeconds: 3600,
    points,
  };
}

function buildDailyPoints() {
  const endExclusive = new Date("2026-04-09T00:00:00+08:00");
  const start = new Date(endExclusive);
  start.setDate(start.getDate() - 180);
  const points: Array<Record<string, number | string | null>> = [];
  for (let index = 0; index < 180; index += 1) {
    const bucketStart = new Date(start);
    bucketStart.setDate(start.getDate() + index);
    const bucketEnd = new Date(bucketStart);
    bucketEnd.setDate(bucketEnd.getDate() + 1);
    const weekday = bucketStart.getDay();
    const amplitude = (index * 5 + weekday * 3) % 11;
    points.push({
      bucketStart: bucketStart.toISOString(),
      bucketEnd: bucketEnd.toISOString(),
      totalCount: amplitude,
      successCount: amplitude,
      failureCount: 0,
      totalTokens: amplitude * 840,
      cacheInputTokens: amplitude * 140,
      totalCost: Number((amplitude * 0.31).toFixed(2)),
      nonSuccessCost: 0,
    });
  }
  return {
    rangeStart: start.toISOString(),
    rangeEnd: endExclusive.toISOString(),
    bucketSeconds: 86400,
    points,
  };
}

const SUMMARY_FIXTURES: Record<SummaryKey, ReturnType<typeof createSummary>> = {
  today: TODAY_SUMMARY_FIXTURE,
  yesterday: YESTERDAY_SUMMARY_FIXTURE,
  previous7d: createSummary(32420, 31310, 1110, 421.76, 180246000),
  "1d": createSummary(76421, 70115, 6306, 3128.74, 8764311220),
  "7d": createSummary(182904, 171240, 11664, 8422.18, 21640351742),
};

const TIMESERIES_FIXTURES: Record<
  TimeseriesKey,
  | ReturnType<typeof buildTodayMinutePoints>
  | ReturnType<typeof build24HourPoints>
  | ReturnType<typeof buildHourlyPoints>
  | ReturnType<typeof buildDailyPoints>
> = {
  "today:1m": buildTodayMinutePoints(),
  "yesterday:1m": buildYesterdayMinutePoints(),
  "1d:1m": build24HourPoints(),
  "7d:1h": buildHourlyPoints(),
  "6mo:1d": buildDailyPoints(),
};

function buildActivityWeight(index: number, mode: "success" | "failure") {
  const hour = Math.floor(index / 60);
  const minute = index % 60;
  const rush = hour < 6 ? 2 : hour < 9 ? 5 : hour < 12 ? 9 : 4;
  const pulse = (index % 11) + 1;
  const boundaryBoost = minute % 15 === 0 ? 4 : minute % 5 === 0 ? 2 : 0;
  const failureBias = mode === "failure" ? (hour >= 9 && hour <= 11 ? 6 : 3) : 0;
  return rush + pulse + boundaryBoost + failureBias;
}

function buildUsageWeight(totalCount: number, index: number, mode: "tokens" | "cost") {
  const base = Math.max(totalCount, 1);
  if (mode === "tokens") {
    return base * (32 + (index % 13)) + ((index % 7) + 1) * 11;
  }
  return base * (24 + (index % 7)) + ((index % 5) + 1) * 5;
}

function buildLatencyMs(index: number, totalCount: number, offset: number) {
  const hour = Math.floor(index / 60);
  const rushPenalty = hour >= 9 && hour <= 11 ? 120 : hour >= 14 && hour <= 17 ? 85 : 30;
  const loadPenalty = Math.min(180, totalCount * 11);
  const wave = ((index + offset) % 23) * 4;
  return 380 + rushPenalty + loadPenalty + wave;
}

function buildTotalLatencyMs(firstResponseByteTotalAvgMs: number, index: number, offset: number) {
  const settleOverhead = 150 + ((index + offset) % 7) * 22;
  return firstResponseByteTotalAvgMs + settleOverhead;
}

function distributeInteger(total: number, weights: number[]) {
  if (weights.length === 0) return [];
  const sanitizedWeights = weights.map((weight) =>
    Number.isFinite(weight) && weight > 0 ? weight : 1,
  );
  const weightSum = sanitizedWeights.reduce((sum, weight) => sum + weight, 0);
  if (weightSum <= 0) {
    const evenShare = Math.floor(total / weights.length);
    const remainder = total - evenShare * weights.length;
    return weights.map((_, index) => evenShare + (index < remainder ? 1 : 0));
  }

  const rawAllocations = sanitizedWeights.map((weight) => (total * weight) / weightSum);
  const allocations = rawAllocations.map((value) => Math.floor(value));
  let remainder = total - allocations.reduce((sum, value) => sum + value, 0);

  if (remainder > 0) {
    const remainders = rawAllocations
      .map((value, index) => ({
        index,
        fraction: value - Math.floor(value),
        weight: sanitizedWeights[index],
      }))
      .sort((left, right) => {
        if (right.fraction !== left.fraction) return right.fraction - left.fraction;
        if (right.weight !== left.weight) return right.weight - left.weight;
        return left.index - right.index;
      });

    for (let cursor = 0; cursor < remainders.length && remainder > 0; cursor += 1, remainder -= 1) {
      allocations[remainders[cursor].index] += 1;
    }
  }

  return allocations;
}

function DashboardOverviewMockApi({
  children,
  failTodayTimeseries = false,
  summaryOverrides = {},
  timeseriesOverrides = {},
  delaySummaryWindows = [],
  responseDelayMs = 0,
}: {
  children: ReactNode;
  failTodayTimeseries?: boolean;
  summaryOverrides?: Partial<Record<SummaryKey, SummaryFixture>>;
  timeseriesOverrides?: Partial<Record<TimeseriesKey, TimeseriesFixture>>;
  delaySummaryWindows?: SummaryKey[];
  responseDelayMs?: number;
}) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null);
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null);

  useLayoutEffect(() => {
    const windowWithFetchLog = window as WindowWithDashboardFetchLog;
    originalFetchRef.current = window.fetch.bind(window);
    originalEventSourceRef.current = window.EventSource;
    windowWithFetchLog.__dashboardOverviewFetchLog__ = [];

    window.fetch = async (input, init) => {
      const inputUrl =
        typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      const url = new URL(inputUrl, window.location.origin);
      windowWithFetchLog.__dashboardOverviewFetchLog__?.push(`${url.pathname}${url.search}`);

      if (url.pathname === "/api/stats/summary") {
        const windowKey = url.searchParams.get("window") as SummaryKey | null;
        if (windowKey && windowKey in SUMMARY_FIXTURES) {
          if (delaySummaryWindows.includes(windowKey) && responseDelayMs > 0) {
            await new Promise((resolve) => window.setTimeout(resolve, responseDelayMs));
          }
          return jsonResponse(summaryOverrides[windowKey] ?? SUMMARY_FIXTURES[windowKey]);
        }
      }

      if (url.pathname === "/api/stats/timeseries") {
        const range = url.searchParams.get("range");
        const bucket = url.searchParams.get("bucket");
        const key = `${range}:${bucket}` as TimeseriesKey;
        if (failTodayTimeseries && key === "today:1m") {
          return new Response(JSON.stringify({ error: "timeseries failed" }), {
            status: 500,
            headers: { "Content-Type": "application/json" },
          });
        }
        if (key in TIMESERIES_FIXTURES) {
          return jsonResponse(timeseriesOverrides[key] ?? TIMESERIES_FIXTURES[key]);
        }
      }

      return (originalFetchRef.current ?? fetch)(input as RequestInfo | URL, init);
    };

    Object.defineProperty(window, "EventSource", {
      configurable: true,
      writable: true,
      value: undefined,
    });

    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current;
      }
      Object.defineProperty(window, "EventSource", {
        configurable: true,
        writable: true,
        value: originalEventSourceRef.current,
      });
      delete windowWithFetchLog.__dashboardOverviewFetchLog__;
    };
  }, [
    delaySummaryWindows,
    failTodayTimeseries,
    responseDelayMs,
    summaryOverrides,
    timeseriesOverrides,
  ]);

  return <>{children}</>;
}

function DashboardOverviewStoryEnvironment({
  children,
  parameters,
  maxWidth = "1660px",
}: {
  children: ReactNode;
  parameters: DashboardOverviewParameters;
  maxWidth?: string;
}) {
  return (
    <I18nProvider>
      <DashboardOverviewMockApi
        failTodayTimeseries={parameters.failTodayTimeseries === true}
        summaryOverrides={
          (parameters.summaryOverrides ?? {}) as Partial<Record<SummaryKey, SummaryFixture>>
        }
        timeseriesOverrides={
          (parameters.timeseriesOverrides ?? {}) as Partial<
            Record<TimeseriesKey, TimeseriesFixture>
          >
        }
        delaySummaryWindows={(parameters.delaySummaryWindows ?? []) as SummaryKey[]}
        responseDelayMs={(parameters.responseDelayMs ?? 0) as number}
      >
        <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
          <div className="mx-auto w-full" style={{ maxWidth }}>
            <RangeStorageHarness
              persistedRange={(parameters.persistedRange ?? null) as PersistedRange}
            >
              {children}
            </RangeStorageHarness>
          </div>
        </div>
      </DashboardOverviewMockApi>
    </I18nProvider>
  );
}

function EmbeddedAccountActivityOverview({
  title = "账号活动总览",
  upstreamAccountId = 42,
  testId = "upstream-account-records-activity-overview",
}: Partial<AccountActivityOverviewProps>) {
  return (
    <DashboardActivityOverview
      title={title}
      upstreamAccountId={upstreamAccountId}
      testId={testId}
      storageKey={`storybook.dashboard.account-activity.${upstreamAccountId}`}
    />
  );
}

function RangeStorageHarness({
  persistedRange,
  children,
}: {
  persistedRange: PersistedRange;
  children: ReactNode;
}) {
  const [ready, setReady] = useState(false);

  useLayoutEffect(() => {
    setReady(false);
    const previousValue = window.localStorage.getItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY);
    if (persistedRange) {
      window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, persistedRange);
    } else {
      window.localStorage.removeItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY);
    }
    setReady(true);

    return () => {
      if (previousValue === null) {
        window.localStorage.removeItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY);
      } else {
        window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, previousValue);
      }
      setReady(false);
    };
  }, [persistedRange]);

  return ready ? children : null;
}

const meta = {
  title: "Dashboard/DashboardActivityOverview",
  component: DashboardActivityOverview,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1660" },
    persistedRange: null,
  },
  decorators: [
    (Story, context) => (
      <DashboardOverviewStoryEnvironment
        parameters={context.parameters as DashboardOverviewParameters}
      >
        <Story />
      </DashboardOverviewStoryEnvironment>
    ),
  ],
} satisfies Meta<typeof DashboardActivityOverview>;

export default meta;

type Story = StoryObj<typeof meta>;

export const TodayView: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /今日|today/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(canvas.getByTestId("today-stats-value-tpm")).toBeVisible();
      expect(canvas.getByTestId("today-stats-value-spend-rate")).toBeVisible();
      expect(canvas.getByTestId("today-stats-value-total-tokens")).toHaveAttribute(
        "data-compact",
        "true",
      );
      expect(canvas.getByTestId("today-stats-secondary-success-ratio")).not.toHaveTextContent("—");
      expect(
        canvas.getByTestId("today-stats-secondary-tpm-per-conversation"),
      ).not.toHaveTextContent("—");
      expect(canvas.getByTestId("today-stats-secondary-in-progress-retry")).not.toHaveTextContent(
        "—",
      );
      expect(
        canvas.getByTestId("today-stats-secondary-response-time-avg-total"),
      ).not.toHaveTextContent("—");
      expect(canvas.getByTestId("today-stats-secondary-cost-failed")).not.toHaveTextContent("—");
      expect(canvas.getByTestId("today-stats-secondary-tokens-failed")).not.toHaveTextContent("—");
    });
  },
};

export const Mobile390DropdownControls: Story = {
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const mobileControls = canvas.getByTestId("dashboard-activity-mobile-selects");
    const rangeSelect = canvas.getByTestId("dashboard-activity-range-select");
    const metricSelect = canvas.getByTestId("dashboard-activity-metric-select");

    await expect(mobileControls).toHaveClass(/grid-cols-2/);
    await expect(rangeSelect).toHaveTextContent(/今日|today/i);
    await expect(metricSelect).toHaveTextContent(/次数|calls/i);

    await userEvent.click(rangeSelect);
    await userEvent.click(within(document.body).getByRole("option", { name: /昨日|yesterday/i }));
    await expect(rangeSelect).toHaveTextContent(/昨日|yesterday/i);

    await userEvent.click(metricSelect);
    await userEvent.click(within(document.body).getByRole("option", { name: /tokens/i }));
    await expect(metricSelect).toHaveTextContent(/tokens/i);
  },
};

export const TodayViewNarrowDesktop: Story = {
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
  },
  decorators: [
    (Story, context) => (
      <DashboardOverviewStoryEnvironment
        parameters={context.parameters as DashboardOverviewParameters}
        maxWidth="1280px"
      >
        <Story />
      </DashboardOverviewStoryEnvironment>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /今日|today/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(canvas.getByTestId("today-stats-value-total-tokens")).toHaveAttribute(
        "data-compact",
        "true",
      );
      expect(canvas.getByTestId("today-stats-value-total-tokens")).toHaveAttribute(
        "data-compact-precision",
        "0",
      );
      expect(canvas.getByTestId("today-stats-value-total-tokens").textContent ?? "").toContain(
        "18M",
      );
    });
  },
};

const NARROW_OVERFLOW_TODAY_SUMMARY = createSummary(3428, 3296, 132, 173.3, 281110000);

export const TodayViewNarrowDesktopOverflow: Story = {
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
    summaryOverrides: {
      today: NARROW_OVERFLOW_TODAY_SUMMARY,
    },
    timeseriesOverrides: {
      "today:1m": buildTodayMinutePoints(NARROW_OVERFLOW_TODAY_SUMMARY),
    },
  },
  decorators: [
    (Story, context) => (
      <DashboardOverviewStoryEnvironment
        parameters={context.parameters as DashboardOverviewParameters}
        maxWidth="1280px"
      >
        <Story />
      </DashboardOverviewStoryEnvironment>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /今日|today/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(canvas.getByTestId("today-stats-value-total-tokens")).toHaveAttribute(
        "data-compact",
        "true",
      );
      expect(canvas.getByTestId("today-stats-value-total-tokens")).toHaveAttribute(
        "data-compact-precision",
        "0",
      );
      expect(canvas.getByTestId("today-stats-value-total-tokens").textContent ?? "").toContain(
        "281M",
      );
      expect(canvas.getByTestId("dashboard-today-activity-chart")).toBeVisible();
    });
  },
};

export const TodayViewNarrowDesktopLoading: Story = {
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
    delaySummaryWindows: ["today"],
    responseDelayMs: 15000,
  },
  decorators: [
    (Story, context) => (
      <DashboardOverviewStoryEnvironment
        parameters={context.parameters as DashboardOverviewParameters}
        maxWidth="1280px"
      >
        <Story />
      </DashboardOverviewStoryEnvironment>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /今日|today/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(canvas.getByTestId("today-stats-value-tpm-loading")).toBeVisible();
      expect(canvas.getByTestId("today-stats-value-total-tokens-loading")).toBeVisible();
      expect(canvas.getByTestId("dashboard-today-activity-chart")).toBeVisible();
    });
  },
};

export const AccountTodayNarrowDesktopOverflowDark: Story = {
  globals: {
    themeMode: "dark",
  },
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
    summaryOverrides: {
      today: NARROW_OVERFLOW_TODAY_SUMMARY,
    },
    timeseriesOverrides: {
      "today:1m": buildTodayMinutePoints(NARROW_OVERFLOW_TODAY_SUMMARY),
    },
  },
  render: () => <EmbeddedAccountActivityOverview />,
  decorators: [
    (Story, context) => (
      <DashboardOverviewStoryEnvironment
        parameters={context.parameters as DashboardOverviewParameters}
        maxWidth="1280px"
      >
        <Story />
      </DashboardOverviewStoryEnvironment>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByTestId("upstream-account-records-activity-overview")).toBeVisible();
      expect(canvas.getByRole("heading", { name: "账号活动总览" })).toBeVisible();
      expect(canvas.getByRole("tab", { name: /今日|today/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(canvas.getByTestId("today-stats-value-total-tokens")).toHaveAttribute(
        "data-compact",
        "true",
      );
      expect(canvas.getByTestId("today-stats-value-total-tokens")).toHaveAttribute(
        "data-compact-precision",
        "0",
      );
      expect(canvas.getByTestId("today-stats-value-total-tokens").textContent ?? "").toContain(
        "281M",
      );
      expect(canvas.getByTestId("today-stats-secondary-tokens-failed")).not.toHaveTextContent("—");
      expect(canvas.getByTestId("today-stats-secondary-cost-failed")).not.toHaveTextContent("—");
      expect(canvas.queryByText(/并行对话|parallel/i)).toBeNull();
      expect(canvas.getByTestId("dashboard-today-activity-chart")).toBeVisible();
    });
  },
};

export const AccountTodayCostCumulative: Story = {
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
  },
  render: () => <EmbeddedAccountActivityOverview />,
  decorators: [
    (Story, context) => (
      <DashboardOverviewStoryEnvironment
        parameters={context.parameters as DashboardOverviewParameters}
        maxWidth="1280px"
      >
        <Story />
      </DashboardOverviewStoryEnvironment>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /今日|today/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
    await userEvent.click(canvas.getByRole("tab", { name: /金额|cost/i }));
    await waitFor(() => {
      expect(canvas.getByTestId("dashboard-today-activity-chart")).toHaveAttribute(
        "data-chart-mode",
        "cumulative-area",
      );
      expect(canvas.getByTestId("upstream-account-records-activity-overview")).toBeVisible();
    });
  },
};

export const AccountTodayNarrowDesktopLoadingDark: Story = {
  globals: {
    themeMode: "dark",
  },
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
    delaySummaryWindows: ["today"],
    responseDelayMs: 15000,
  },
  render: () => <EmbeddedAccountActivityOverview />,
  decorators: [
    (Story, context) => (
      <DashboardOverviewStoryEnvironment
        parameters={context.parameters as DashboardOverviewParameters}
        maxWidth="1280px"
      >
        <Story />
      </DashboardOverviewStoryEnvironment>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByTestId("upstream-account-records-activity-overview")).toBeVisible();
      expect(canvas.getByRole("heading", { name: "账号活动总览" })).toBeVisible();
      expect(canvas.getByRole("tab", { name: /今日|today/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(canvas.getByTestId("today-stats-value-tpm-loading")).toBeVisible();
      expect(canvas.getByTestId("today-stats-value-total-tokens-loading")).toBeVisible();
      expect(canvas.queryByText(/并行对话|parallel/i)).toBeNull();
      expect(canvas.getByTestId("dashboard-today-activity-chart")).toBeVisible();
    });
  },
};

export const TodayRateUnavailable: Story = {
  parameters: {
    failTodayTimeseries: true,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /今日|today/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(canvas.getByTestId("today-stats-value-tpm")).toHaveTextContent("—");
      expect(canvas.getByTestId("today-stats-value-spend-rate")).toHaveTextContent("—");
      expect(canvas.getByTestId("today-stats-value-success")).toBeVisible();
    });
  },
};

export const YesterdayView: Story = {
  parameters: {
    persistedRange: "yesterday",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /昨日|yesterday/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(canvas.getByTestId("today-stats-value-tpm")).toBeVisible();
      expect(canvas.getByTestId("dashboard-today-activity-chart")).toHaveAttribute(
        "data-chart-mode",
        "count-bars",
      );
    });
  },
};

export const TodayTrendView: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("tab", { name: /趋势|trend/i }));
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /趋势|trend/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(canvas.getByTestId("dashboard-today-activity-chart")).toHaveAttribute(
        "data-chart-mode",
        "trend-area",
      );
    });
  },
};

export const YesterdayTrendView: Story = {
  parameters: {
    persistedRange: "yesterday",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /昨日|yesterday/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
    await userEvent.click(canvas.getByRole("tab", { name: /趋势|trend/i }));
    await waitFor(() => {
      expect(canvas.getByTestId("dashboard-today-activity-chart")).toHaveAttribute(
        "data-chart-mode",
        "trend-area",
      );
    });
  },
};

export const SevenDayView: Story = {
  parameters: {
    persistedRange: "7d",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /7 日|7 days/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(canvas.queryByRole("tab", { name: /趋势|trend/i })).toBeNull();
    });
  },
};

export const TodayCostCumulative: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("tab", { name: /金额|cost/i }));
    await waitFor(() => {
      expect(canvas.getByTestId("dashboard-today-activity-chart")).toHaveAttribute(
        "data-chart-mode",
        "cumulative-area",
      );
    });
  },
};

export const HistoryView: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("tab", { name: /历史|history/i }));
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /历史|history/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
    await expect(canvas.getByTestId("usage-calendar-card")).toBeVisible();
    await expect(canvas.queryByText(/总 TOKENS|total tokens/i)).toBeNull();
  },
};

export const RestoresPersistedHistory: Story = {
  parameters: {
    persistedRange: "usage",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /历史|history/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
  },
};

export const MetricMemoryFlow: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("tab", { name: /金额|cost/i }));
    await userEvent.click(canvas.getByRole("tab", { name: /7 日|7 days/i }));
    await userEvent.click(canvas.getByRole("tab", { name: /tokens/i }));
    await userEvent.click(canvas.getByRole("tab", { name: /今日|today/i }));
    await waitFor(() => {
      expect(canvas.getByRole("tab", { name: /金额|cost/i })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
  },
};

export const LoadsRangesOnDemand: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const windowWithFetchLog = window as WindowWithDashboardFetchLog;

    await waitFor(() => {
      const fetchLog = windowWithFetchLog.__dashboardOverviewFetchLog__ ?? [];
      expect(fetchLog).toContain("/api/stats/summary?window=today");
      expect(fetchLog).toContain("/api/stats/timeseries?range=today&bucket=1m");
      expect(fetchLog.some((entry) => entry.includes("window=yesterday"))).toBe(false);
      expect(fetchLog.some((entry) => entry.includes("range=yesterday"))).toBe(false);
      expect(fetchLog.some((entry) => entry.includes("window=1d"))).toBe(false);
      expect(fetchLog.some((entry) => entry.includes("window=7d"))).toBe(false);
    });

    await userEvent.click(canvas.getByRole("tab", { name: /昨日|yesterday/i }));
    await waitFor(() => {
      const fetchLog = windowWithFetchLog.__dashboardOverviewFetchLog__ ?? [];
      expect(fetchLog).toContain("/api/stats/summary?window=yesterday");
      expect(fetchLog).toContain("/api/stats/timeseries?range=yesterday&bucket=1m");
      expect(fetchLog.some((entry) => entry.includes("window=1d"))).toBe(false);
    });

    await userEvent.click(canvas.getByRole("tab", { name: /7 日|7 days/i }));
    await waitFor(() => {
      const fetchLog = windowWithFetchLog.__dashboardOverviewFetchLog__ ?? [];
      expect(fetchLog).toContain("/api/stats/summary?window=7d");
      expect(fetchLog.some((entry) => entry.includes("window=1d"))).toBe(false);
    });

    await userEvent.click(canvas.getByRole("tab", { name: /24 小时|24 hours/i }));
    await waitFor(() => {
      const fetchLog = windowWithFetchLog.__dashboardOverviewFetchLog__ ?? [];
      expect(fetchLog).toContain("/api/stats/summary?window=1d");
    });
  },
};
