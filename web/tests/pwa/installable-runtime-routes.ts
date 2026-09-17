import { mkdir } from "node:fs/promises";
import path from "node:path";
import type { Page, Route } from "@playwright/test";

function jsonRouteBody(body: unknown) {
  return {
    status: 200,
    contentType: "application/json; charset=utf-8",
    body: JSON.stringify(body),
  };
}

type ManifestIcon = { src: string };
type TestManifest = {
  id: string;
  scope: string;
  start_url: string;
  icons: ManifestIcon[];
  shortcuts: Array<{ icons: ManifestIcon[] }>;
};

function _manifestIconUrls(manifest: TestManifest): string[] {
  return [
    ...manifest.icons.map((icon) => icon.src),
    ...manifest.shortcuts.flatMap((shortcut) => shortcut.icons.map((icon) => icon.src)),
  ];
}

async function _maybeCaptureScreenshot(page: Page, filename: string) {
  const captureDir = process.env.PWA_CAPTURE_DIR;
  if (!captureDir) return;
  await mkdir(captureDir, { recursive: true });
  await page.screenshot({
    path: path.join(captureDir, filename),
  });
}

function buildSummary({
  totalCount,
  successCount,
  failureCount,
  totalCost,
  totalTokens,
}: {
  totalCount: number;
  successCount: number;
  failureCount: number;
  totalCost: number;
  totalTokens: number;
}) {
  return {
    totalCount,
    successCount,
    failureCount,
    totalCost,
    totalTokens,
  };
}

function buildDashboardActivityResponse({
  range,
  summary,
}: {
  range: "today" | "yesterday" | "1d" | "7d";
  summary: ReturnType<typeof buildSummary>;
}) {
  return {
    range,
    rangeStart:
      range === "yesterday"
        ? "2026-04-08T00:00:00.000Z"
        : range === "1d"
          ? "2026-04-08T12:20:00.000Z"
          : range === "7d"
            ? "2026-04-02T12:20:00.000Z"
            : "2026-04-09T00:00:00.000Z",
    rangeEnd: "2026-04-09T12:20:00.000Z",
    snapshotId: 1775718000000,
    rateWindow: {
      start: "2026-04-09T12:19:00.000Z",
      end: "2026-04-09T12:20:00.000Z",
      windowMinutes: 1,
      mode: "last_complete_1m_sma",
    },
    summary: {
      stats: {
        ...summary,
        inProgressConversationCount: Math.max(1, Math.round(summary.successCount / 320)),
        inProgressRetryConversationCount: Math.max(0, Math.round(summary.failureCount / 64)),
        inProgressAvgWaitMs: 1680,
      },
      tokensPerMinute:
        range === "today" ? 1340 : range === "yesterday" ? 1210 : range === "1d" ? 2580 : 6840,
      spendRate:
        range === "today" ? 1.02 : range === "yesterday" ? 0.88 : range === "1d" ? 3.14 : 8.44,
    },
  };
}

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
    return base * (14 + (index % 17)) + ((index % 7) + 1) * 19;
  }
  return base * (6 + (index % 9)) + ((index % 5) + 1) * 7;
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

function buildMinuteTimeseries({
  rangeStart,
  rangeEnd,
  summary,
}: {
  rangeStart: string;
  rangeEnd: string;
  summary: ReturnType<typeof buildSummary>;
}) {
  const startMs = Date.parse(rangeStart);
  const endMs = Date.parse(rangeEnd);
  const count = Math.floor((endMs - startMs) / 60_000) + 1;
  const minuteIndexes = Array.from({ length: count }, (_, index) => index);
  const successCounts = distributeInteger(
    summary.successCount,
    minuteIndexes.map((index) => buildActivityWeight(index, "success")),
  );
  const failureCounts = distributeInteger(
    summary.failureCount,
    minuteIndexes.map((index) => buildActivityWeight(index, "failure")),
  );
  const tokenTotals = distributeInteger(
    summary.totalTokens,
    minuteIndexes.map((index) =>
      buildUsageWeight(successCounts[index] + failureCounts[index], index, "tokens"),
    ),
  );
  const costCents = distributeInteger(
    Math.round(summary.totalCost * 100),
    minuteIndexes.map((index) =>
      buildUsageWeight(successCounts[index] + failureCounts[index], index, "cost"),
    ),
  );

  return {
    rangeStart,
    rangeEnd,
    bucketSeconds: 60,
    effectiveBucket: "1m",
    availableBuckets: ["1m"],
    bucketLimitedToDaily: false,
    points: minuteIndexes.map((index) => {
      const bucketStartMs = startMs + index * 60_000;
      const bucketEndMs = bucketStartMs + 60_000;
      const successCount = successCounts[index] ?? 0;
      const failureCount = failureCounts[index] ?? 0;
      const totalCount = successCount + failureCount;
      return {
        bucketStart: new Date(bucketStartMs).toISOString(),
        bucketEnd: new Date(bucketEndMs).toISOString(),
        totalCount,
        successCount,
        failureCount,
        totalTokens: tokenTotals[index] ?? 0,
        cacheInputTokens: Math.round((tokenTotals[index] ?? 0) * 0.23),
        totalCost: Number(((costCents[index] ?? 0) / 100).toFixed(2)),
        firstResponseByteTotalSampleCount: totalCount,
        firstResponseByteTotalAvgMs: totalCount > 0 ? 820 + ((index * 37) % 340) : null,
      };
    }),
  };
}

function buildFixedTimeseries({
  count,
  bucketSeconds,
  rangeStart,
  valueOffset,
}: {
  count: number;
  bucketSeconds: number;
  rangeStart: string;
  valueOffset: number;
}) {
  const startMs = Date.parse(rangeStart);
  return {
    rangeStart,
    rangeEnd: new Date(startMs + count * bucketSeconds * 1000).toISOString(),
    bucketSeconds,
    effectiveBucket: bucketSeconds === 3600 ? "1h" : bucketSeconds === 86400 ? "1d" : "1m",
    availableBuckets: [bucketSeconds === 3600 ? "1h" : bucketSeconds === 86400 ? "1d" : "1m"],
    bucketLimitedToDaily: false,
    points: Array.from({ length: count }, (_, index) => {
      const bucketStartMs = startMs + index * bucketSeconds * 1000;
      const bucketEndMs = bucketStartMs + bucketSeconds * 1000;
      const pulse = (index + valueOffset) % 24;
      const totalCount =
        pulse >= 7 && pulse <= 11
          ? 24 + (index % 6)
          : pulse >= 18 && pulse <= 22
            ? 16 + (index % 5)
            : index % 4;
      const failureCount = totalCount === 0 ? 0 : index % 11 === 0 ? 2 : index % 7 === 0 ? 1 : 0;
      const successCount = Math.max(totalCount - failureCount, 0);
      return {
        bucketStart: new Date(bucketStartMs).toISOString(),
        bucketEnd: new Date(bucketEndMs).toISOString(),
        totalCount,
        successCount,
        failureCount,
        totalTokens: totalCount * 3200,
        cacheInputTokens: totalCount * 720,
        totalCost: Number((totalCount * 0.018).toFixed(4)),
        firstResponseByteTotalSampleCount: totalCount,
        firstResponseByteTotalAvgMs: totalCount > 0 ? 760 + ((index * 19) % 280) : null,
      };
    }),
  };
}

function buildParallelWorkWindow(counts: number[], rangeStart: string, bucketSeconds: number) {
  const startMs = Date.parse(rangeStart);
  return {
    rangeStart,
    rangeEnd: new Date(startMs + counts.length * bucketSeconds * 1000).toISOString(),
    bucketSeconds,
    completeBucketCount: counts.length,
    activeBucketCount: counts.filter((count) => count > 0).length,
    minCount: counts.length > 0 ? Math.min(...counts) : null,
    maxCount: counts.length > 0 ? Math.max(...counts) : null,
    avgCount:
      counts.length > 0
        ? Number((counts.reduce((sum, count) => sum + count, 0) / counts.length).toFixed(2))
        : null,
    effectiveTimeZone: "Asia/Shanghai",
    timeZoneFallback: false,
    points: counts.map((parallelCount, index) => ({
      bucketStart: new Date(startMs + index * bucketSeconds * 1000).toISOString(),
      bucketEnd: new Date(startMs + (index + 1) * bucketSeconds * 1000).toISOString(),
      parallelCount,
    })),
    conversations: [],
  };
}

function buildParallelWorkResponse(
  currentCounts: number[],
  historyCounts: number[],
  rangeStart: string,
) {
  return {
    current: buildParallelWorkWindow(currentCounts, rangeStart, 60),
    minute7d: buildParallelWorkWindow(historyCounts, "2026-04-03T00:00:00.000Z", 60),
    hour30d: buildParallelWorkWindow([5, 6, 7, 8], "2026-03-11T00:00:00.000Z", 3600),
    dayAll: buildParallelWorkWindow([7], "2026-04-08T00:00:00.000Z", 86400),
  };
}

function buildDashboardNetworkTimeseries(range: "today" | "yesterday" | "1d") {
  const bucketSeconds = 300;
  const rangeStart =
    range === "today"
      ? "2026-04-09T00:00:00.000Z"
      : range === "yesterday"
        ? "2026-04-08T00:00:00.000Z"
        : "2026-04-08T12:20:00.000Z";
  const rangeEnd = range === "yesterday" ? "2026-04-09T00:00:00.000Z" : "2026-04-09T12:20:00.000Z";
  const startMs = Date.parse(rangeStart);
  const endMs = Date.parse(rangeEnd);
  const bucketCount = Math.max(1, Math.ceil((endMs - startMs) / (bucketSeconds * 1000)));
  return {
    range,
    rangeStart,
    rangeEnd,
    snapshotId: endMs,
    bucketSeconds,
    points: Array.from({ length: bucketCount }, (_, index) => {
      const bucketStart = new Date(startMs + index * bucketSeconds * 1000);
      const bucketEnd = new Date(bucketStart.getTime() + bucketSeconds * 1000);
      const uploadBytesPerSecond = 4800 + ((index * 430) % 6400);
      const downloadBytesPerSecond = 28_000 + ((index * 1730) % 36_000);
      return {
        bucketStart: bucketStart.toISOString(),
        bucketEnd: bucketEnd.toISOString(),
        uploadBytesPerSecond,
        downloadBytesPerSecond,
        uploadBytes: Math.round(uploadBytesPerSecond * bucketSeconds),
        downloadBytes: Math.round(downloadBytesPerSecond * bucketSeconds),
        isLiveBucket: range !== "yesterday" && index === bucketCount - 1,
      };
    }),
  };
}

const DASHBOARD_SUMMARIES = {
  today: buildSummary({
    totalCount: 3428,
    successCount: 3296,
    failureCount: 132,
    totalCost: 42.86,
    totalTokens: 18_764_200,
  }),
  yesterday: buildSummary({
    totalCount: 4876,
    successCount: 4718,
    failureCount: 158,
    totalCost: 61.72,
    totalTokens: 26_918_400,
  }),
  "1d": buildSummary({
    totalCount: 76_421,
    successCount: 70_115,
    failureCount: 6306,
    totalCost: 3128.74,
    totalTokens: 8_764_311_220,
  }),
  "7d": buildSummary({
    totalCount: 182_904,
    successCount: 171_240,
    failureCount: 11_664,
    totalCost: 8422.18,
    totalTokens: 21_640_351_742,
  }),
  previous7d: buildSummary({
    totalCount: 32_420,
    successCount: 31_310,
    failureCount: 1110,
    totalCost: 421.76,
    totalTokens: 180_246_000,
  }),
};

async function fulfillSummaryRoute(route: Route, url: URL): Promise<boolean> {
  if (url.pathname !== "/api/stats/summary") return false;
  const window = url.searchParams.get("window") ?? "today";
  const summary =
    window === "yesterday"
      ? DASHBOARD_SUMMARIES.yesterday
      : window === "previous7d"
        ? DASHBOARD_SUMMARIES.previous7d
        : window === "1d"
          ? DASHBOARD_SUMMARIES["1d"]
          : window === "7d"
            ? DASHBOARD_SUMMARIES["7d"]
            : DASHBOARD_SUMMARIES.today;
  await route.fulfill(jsonRouteBody(summary));
  return true;
}

async function fulfillActivityRoute(route: Route, url: URL): Promise<boolean> {
  if (url.pathname !== "/api/stats/dashboard-activity") return false;
  const range = (url.searchParams.get("range") ?? "today") as "today" | "yesterday" | "1d" | "7d";
  await route.fulfill(
    jsonRouteBody(buildDashboardActivityResponse({ range, summary: DASHBOARD_SUMMARIES[range] })),
  );
  return true;
}

async function fulfillTimeseriesRoute(route: Route, url: URL): Promise<boolean> {
  if (url.pathname !== "/api/stats/timeseries") return false;
  const range = url.searchParams.get("range");
  const body =
    range === "today"
      ? buildMinuteTimeseries({
          rangeStart: "2026-04-09T00:00:00.000Z",
          rangeEnd: "2026-04-09T12:20:00.000Z",
          summary: DASHBOARD_SUMMARIES.today,
        })
      : range === "yesterday"
        ? buildMinuteTimeseries({
            rangeStart: "2026-04-08T00:00:00.000Z",
            rangeEnd: "2026-04-08T18:36:00.000Z",
            summary: DASHBOARD_SUMMARIES.yesterday,
          })
        : range === "1d"
          ? buildFixedTimeseries({
              count: 24 * 60,
              bucketSeconds: 60,
              rangeStart: "2026-04-08T12:20:00.000Z",
              valueOffset: 9,
            })
          : range === "7d"
            ? buildFixedTimeseries({
                count: 7 * 24,
                bucketSeconds: 3600,
                rangeStart: "2026-04-02T12:20:00.000Z",
                valueOffset: 7,
              })
            : range === "6mo"
              ? buildFixedTimeseries({
                  count: 180,
                  bucketSeconds: 86400,
                  rangeStart: "2025-10-11T00:00:00.000Z",
                  valueOffset: 11,
                })
              : null;
  if (body == null) return false;
  await route.fulfill(jsonRouteBody(body));
  return true;
}

async function fulfillParallelWorkRoute(route: Route, url: URL): Promise<boolean> {
  if (url.pathname !== "/api/stats/parallel-work") return false;
  const range = url.searchParams.get("range") ?? "today";
  const body =
    range === "yesterday"
      ? buildParallelWorkResponse([6, 7, 8, 7], [5, 6, 7, 8], "2026-04-08T00:00:00.000Z")
      : buildParallelWorkResponse([8, 10, 12, 9], [6, 7, 8, 9], "2026-04-09T00:00:00.000Z");
  await route.fulfill(jsonRouteBody(body));
  return true;
}

async function fulfillNetworkTimeseriesRoute(route: Route, url: URL): Promise<boolean> {
  if (url.pathname !== "/api/stats/dashboard-network-timeseries") return false;
  const range = (url.searchParams.get("range") ?? "today") as "today" | "yesterday" | "1d";
  await route.fulfill(jsonRouteBody(buildDashboardNetworkTimeseries(range)));
  return true;
}

async function handleDashboardStatsRoute(route: Route) {
  const url = new URL(route.request().url());
  if (await fulfillSummaryRoute(route, url)) return;
  if (await fulfillActivityRoute(route, url)) return;
  if (await fulfillTimeseriesRoute(route, url)) return;
  if (await fulfillParallelWorkRoute(route, url)) return;
  if (await fulfillNetworkTimeseriesRoute(route, url)) return;
  await route.continue();
}

export async function installDashboardOverviewRoutes(page: Page) {
  await page.route("**/api/version", async (route) => {
    await route.fulfill(jsonRouteBody({ backend: "v0.2.0" }));
  });
  await page.route("**/api/stats/**", handleDashboardStatsRoute);
}
