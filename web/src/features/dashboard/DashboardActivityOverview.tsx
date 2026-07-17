import { memo, useEffect, useMemo, useRef, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import { SelectField } from "../../components/ui/select-field";
import { useDashboardNetworkTimeseries } from "../../hooks/useDashboardNetworkTimeseries";
import { useParallelWorkStats } from "../../hooks/useParallelWorkStats";
import { useSummary } from "../../hooks/useStats";
import { useTimeseries } from "../../hooks/useTimeseries";
import { useTranslation } from "../../i18n";
import type { DashboardActivityResponse } from "../../lib/api";
import { metricAccent } from "../../lib/chartTheme";
import { recordTodayChartDataCommit } from "../../lib/dashboardPerformanceDiagnostics";
import { useTheme } from "../../theme";
import { StatsCards } from "../stats/StatsCards";
import { DashboardNetworkActivityChart } from "./DashboardNetworkActivityChart";
import { DashboardTodayActivityChart } from "./DashboardTodayActivityChart";
import {
  DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
  type DashboardActivityRangeKey,
  persistDashboardActivityRange,
  readPersistedDashboardActivityRange,
} from "./dashboardActivityRange";
import type {
  DashboardOverviewSnapshotBundle,
  DashboardOverviewSnapshotStatus,
} from "./dashboardOverviewSnapshots";
import { Last24hTenMinuteHeatmap, type MetricKey } from "./Last24hTenMinuteHeatmap";
import { TodayStatsOverview } from "./TodayStatsOverview";
import { UsageCalendar } from "./UsageCalendar";
import { WeeklyHourlyHeatmap } from "./WeeklyHourlyHeatmap";

type OverviewMetricKey = MetricKey | "network";
type NaturalDayChartMetric = OverviewMetricKey | "trend";
type Dashboard24HourMetric = OverviewMetricKey;
type DashboardOverviewLocale = "zh-CN" | "en-US";

const LIVE_RATE_REFRESH_MS = 15_000;
export const DASHBOARD_TOP_CHART_DATA_COMMIT_INTERVAL_MS = 5_000;
const RANGE_OPTIONS: Array<{ key: DashboardActivityRangeKey; labelKey: string }> = [
  { key: "today", labelKey: "dashboard.activityOverview.rangeToday" },
  { key: "yesterday", labelKey: "dashboard.activityOverview.rangeYesterday" },
  { key: "1d", labelKey: "dashboard.activityOverview.range24h" },
  { key: "7d", labelKey: "dashboard.activityOverview.range7d" },
  { key: "usage", labelKey: "dashboard.activityOverview.rangeUsage" },
];

const METRIC_OPTIONS: Array<{ key: MetricKey; labelKey: string }> = [
  { key: "totalCount", labelKey: "metric.totalCount" },
  { key: "totalCost", labelKey: "metric.totalCost" },
  { key: "totalTokens", labelKey: "metric.totalTokens" },
];
const NETWORK_METRIC_OPTION = {
  key: "network" as const,
  labelKey: "dashboard.activityOverview.network",
};
const NATURAL_DAY_METRIC_OPTIONS: Array<{ key: NaturalDayChartMetric; labelKey: string }> = [
  ...METRIC_OPTIONS,
  NETWORK_METRIC_OPTION,
  { key: "trend", labelKey: "chart.trend" },
];
const RANGE_24H_METRIC_OPTIONS: Array<{ key: Dashboard24HourMetric; labelKey: string }> = [
  ...METRIC_OPTIONS,
  NETWORK_METRIC_OPTION,
];

function buildDashboardActivityRate(dashboardActivity: DashboardActivityResponse) {
  return {
    tokensPerMinute: dashboardActivity.summary.tokensPerMinute ?? 0,
    spendRate: dashboardActivity.summary.spendRate ?? 0,
    windowMinutes: dashboardActivity.rateWindow.windowMinutes,
    available: true,
    currentFirstResponseByteTotalAvgMs:
      dashboardActivity.summary.currentFirstResponseByteTotalAvgMs ?? null,
    currentAvgTotalMs: dashboardActivity.summary.currentAvgTotalMs ?? null,
  };
}

function useSnapshotRateNow(enabled: boolean) {
  const [rateNow, setRateNow] = useState(() => new Date());

  useEffect(() => {
    if (!enabled) return;
    setRateNow(new Date());
    const timer = window.setInterval(() => {
      setRateNow(new Date());
    }, LIVE_RATE_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [enabled]);

  return rateNow;
}

function formatDashboardSnapshotCachedAt(value: string | null, localeTag: DashboardOverviewLocale) {
  if (!value) return null;
  const epoch = Date.parse(value);
  if (!Number.isFinite(epoch)) return null;
  return new Intl.DateTimeFormat(localeTag, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(epoch));
}

function DashboardOverviewSnapshotBanner({
  mode,
  cachedAtLabel,
  readyRangeCount,
  totalRangeCount,
  t,
}: {
  mode: DashboardOverviewSnapshotStatus["mode"];
  cachedAtLabel: string | null;
  readyRangeCount: number;
  totalRangeCount: number;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  if (mode === "live") return null;

  if (mode === "cached-offline") {
    return (
      <Alert
        className="border-warning/35 bg-warning/10 text-base-content"
        data-testid="dashboard-overview-snapshot-banner"
      >
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-semibold">
              {t("dashboard.activityOverview.snapshotBannerTitle")}
            </span>
            <span className="rounded-full border border-warning/35 bg-base-100/70 px-2 py-0.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-base-content/75">
              {t("dashboard.activityOverview.snapshotReadyRanges", {
                count: readyRangeCount,
                total: totalRangeCount,
              })}
            </span>
          </div>
          <p className="text-sm leading-6 text-base-content/80">
            {t("dashboard.activityOverview.snapshotBannerDescription", {
              cachedAt: cachedAtLabel ?? t("dashboard.activityOverview.snapshotCachedAtUnknown"),
            })}
          </p>
        </div>
      </Alert>
    );
  }

  return (
    <div
      className="rounded-xl border border-base-300/70 bg-base-200/55 px-4 py-4"
      data-testid="dashboard-overview-snapshot-empty"
    >
      <div className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <h3 className="text-sm font-semibold text-base-content">
            {t("dashboard.activityOverview.snapshotNotReadyTitle")}
          </h3>
          <span className="rounded-full border border-base-300/80 bg-base-100/75 px-2 py-0.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-base-content/70">
            {t("dashboard.activityOverview.snapshotReadyRanges", {
              count: readyRangeCount,
              total: totalRangeCount,
            })}
          </span>
        </div>
        <p className="max-w-[72ch] text-sm leading-6 text-base-content/75">
          {t("dashboard.activityOverview.snapshotNotReadyDescription")}
        </p>
      </div>
    </div>
  );
}

function resolveDashboardMetricAccent(metric: OverviewMetricKey, themeMode: "light" | "dark") {
  if (metric === "network") {
    return themeMode === "dark" ? "#34d399" : "#059669";
  }
  return metricAccent(metric, themeMode);
}

function useScopedSummary(window: string, upstreamAccountId?: number) {
  return useSummary(window, upstreamAccountId == null ? undefined : { upstreamAccountId });
}

function useDashboardTopChartCommittedResponse(
  response: ReturnType<typeof useTimeseries>["data"],
  {
    summaryWindow,
    closedNaturalDay,
  }: {
    summaryWindow: "today" | "yesterday";
    closedNaturalDay: boolean;
  },
) {
  const [committedResponse, setCommittedResponse] = useState(response);
  const committedResponseRef = useRef(response);
  const latestResponseRef = useRef(response);
  const commitTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastCommitAtRef = useRef(response == null ? 0 : Date.now());

  useEffect(() => {
    committedResponseRef.current = committedResponse;
  }, [committedResponse]);

  useEffect(() => {
    latestResponseRef.current = response;

    const clearTimer = () => {
      if (!commitTimerRef.current) return;
      clearTimeout(commitTimerRef.current);
      commitTimerRef.current = null;
    };
    const commit = (nextResponse: typeof response) => {
      clearTimer();
      committedResponseRef.current = nextResponse;
      lastCommitAtRef.current = Date.now();
      setCommittedResponse(nextResponse);
      if (nextResponse != null) {
        recordTodayChartDataCommit(summaryWindow);
      }
    };

    if (closedNaturalDay || committedResponseRef.current == null) {
      commit(response);
      return clearTimer;
    }

    const delay = Math.max(
      0,
      DASHBOARD_TOP_CHART_DATA_COMMIT_INTERVAL_MS - (Date.now() - lastCommitAtRef.current),
    );
    if (delay === 0) {
      commit(response);
      return clearTimer;
    }
    if (!commitTimerRef.current) {
      commitTimerRef.current = setTimeout(() => {
        commitTimerRef.current = null;
        commit(latestResponseRef.current);
      }, delay);
    }

    return clearTimer;
  }, [closedNaturalDay, response, summaryWindow]);

  useEffect(
    () => () => {
      if (commitTimerRef.current) {
        clearTimeout(commitTimerRef.current);
      }
    },
    [],
  );

  return committedResponse;
}

function DashboardNaturalDayRangePanel({
  metric,
  summaryWindow,
  timeseriesRange,
  testId,
  upstreamAccountId,
  dashboardActivity,
  dashboardActivityLoading = false,
  dashboardActivityError = null,
}: {
  metric: NaturalDayChartMetric;
  summaryWindow: "today" | "yesterday";
  timeseriesRange: "today" | "yesterday";
  testId: string;
  upstreamAccountId?: number;
  dashboardActivity?: DashboardActivityResponse | null;
  dashboardActivityLoading?: boolean;
  dashboardActivityError?: string | null;
}) {
  const { data, isLoading, error } = useTimeseries(
    timeseriesRange,
    upstreamAccountId == null ? { bucket: "1m" } : { bucket: "1m", upstreamAccountId },
  );
  const {
    data: networkData,
    isLoading: networkLoading,
    error: networkError,
  } = useDashboardNetworkTimeseries(timeseriesRange, metric === "network", upstreamAccountId);
  const chartResponse = useDashboardTopChartCommittedResponse(data, {
    summaryWindow,
    closedNaturalDay: timeseriesRange === "yesterday",
  });

  return (
    <div className="flex flex-col gap-5" data-testid={testId} data-active="true">
      {summaryWindow === "today" ? (
        <DashboardNaturalDayTodaySummaryOverview
          response={data}
          closedNaturalDay={timeseriesRange === "yesterday"}
          upstreamAccountId={upstreamAccountId}
          dashboardActivity={dashboardActivity}
          dashboardActivityLoading={dashboardActivityLoading}
          dashboardActivityError={dashboardActivityError}
        />
      ) : (
        <DashboardNaturalDayYesterdaySummaryOverview
          response={data}
          closedNaturalDay={timeseriesRange === "yesterday"}
          upstreamAccountId={upstreamAccountId}
          dashboardActivity={dashboardActivity}
          dashboardActivityLoading={dashboardActivityLoading}
          dashboardActivityError={dashboardActivityError}
        />
      )}
      <DashboardNaturalDayChartSection
        response={chartResponse}
        loading={isLoading && chartResponse == null}
        error={error}
        metric={metric}
        closedNaturalDay={timeseriesRange === "yesterday"}
        networkResponse={networkData}
        networkLoading={networkLoading}
        networkError={networkError}
      />
    </div>
  );
}

function DashboardNaturalDayTodaySummaryOverview({
  response,
  closedNaturalDay,
  upstreamAccountId,
  dashboardActivity,
  dashboardActivityLoading = false,
  dashboardActivityError = null,
}: {
  response: ReturnType<typeof useTimeseries>["data"];
  closedNaturalDay: boolean;
  upstreamAccountId?: number;
  dashboardActivity?: DashboardActivityResponse | null;
  dashboardActivityLoading?: boolean;
  dashboardActivityError?: string | null;
}) {
  const snapshotActive = upstreamAccountId == null && dashboardActivity?.range === "today";
  if (snapshotActive && dashboardActivity != null) {
    return (
      <DashboardNaturalDayTodaySummaryOverviewSnapshotBacked
        response={response}
        closedNaturalDay={closedNaturalDay}
        dashboardActivity={dashboardActivity}
      />
    );
  }

  return (
    <DashboardNaturalDayTodaySummaryOverviewFallback
      response={response}
      closedNaturalDay={closedNaturalDay}
      upstreamAccountId={upstreamAccountId}
      dashboardActivity={dashboardActivity}
      dashboardActivityLoading={dashboardActivityLoading}
      dashboardActivityError={dashboardActivityError}
    />
  );
}

function DashboardNaturalDayTodaySummaryOverviewSnapshotBacked({
  response,
  closedNaturalDay,
  dashboardActivity,
}: {
  response: ReturnType<typeof useTimeseries>["data"];
  closedNaturalDay: boolean;
  dashboardActivity: DashboardActivityResponse;
}) {
  const { summary: comparisonSummary } = useScopedSummary("yesterday");
  const { summary: previous7dSummary } = useScopedSummary("previous7d");
  const { data: comparisonTimeseries } = useTimeseries("yesterday", { bucket: "1m" });
  const { data: parallelWorkStats } = useParallelWorkStats({
    range: "today",
    bucket: "1m",
  });
  const { data: comparisonParallelWorkStats } = useParallelWorkStats({
    range: "yesterday",
    bucket: "1m",
  });
  const [rateNow, setRateNow] = useState(() => new Date());
  useEffect(() => {
    if (closedNaturalDay) return;
    setRateNow(new Date());
    const timer = window.setInterval(() => {
      setRateNow(new Date());
    }, LIVE_RATE_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [closedNaturalDay]);

  return (
    <TodayStatsOverview
      stats={dashboardActivity.summary.stats}
      loading={false}
      error={null}
      rate={{
        tokensPerMinute: dashboardActivity.summary.tokensPerMinute ?? 0,
        spendRate: dashboardActivity.summary.spendRate ?? 0,
        windowMinutes: dashboardActivity.rateWindow.windowMinutes,
        available: true,
        currentFirstResponseByteTotalAvgMs:
          dashboardActivity.summary.currentFirstResponseByteTotalAvgMs ?? null,
        currentAvgTotalMs: dashboardActivity.summary.currentAvgTotalMs ?? null,
      }}
      rateLoading={false}
      rateError={null}
      now={rateNow}
      timeseries={response}
      comparisonStats={comparisonSummary}
      comparisonTimeseries={comparisonTimeseries}
      previous7dStats={previous7dSummary}
      parallelWorkStats={parallelWorkStats}
      comparisonParallelWorkStats={comparisonParallelWorkStats}
      showInProgressConversations
      dayKind="today"
      showSurface={false}
      showHeader={false}
      showDayBadge={false}
      modelPerformance={dashboardActivity.summary.modelPerformance}
    />
  );
}

function DashboardNaturalDayTodaySummaryOverviewFallback({
  response,
  closedNaturalDay,
  upstreamAccountId,
  dashboardActivity,
  dashboardActivityLoading = false,
  dashboardActivityError = null,
}: {
  response: ReturnType<typeof useTimeseries>["data"];
  closedNaturalDay: boolean;
  upstreamAccountId?: number;
  dashboardActivity?: DashboardActivityResponse | null;
  dashboardActivityLoading?: boolean;
  dashboardActivityError?: string | null;
}) {
  const {
    summary,
    isLoading: summaryLoading,
    error: summaryError,
  } = useScopedSummary("today", upstreamAccountId);
  const { summary: comparisonSummary } = useScopedSummary("yesterday", upstreamAccountId);
  const { summary: previous7dSummary } = useScopedSummary("previous7d", upstreamAccountId);
  const { data: comparisonTimeseries } = useTimeseries(
    "yesterday",
    upstreamAccountId == null ? { bucket: "1m" } : { bucket: "1m", upstreamAccountId },
  );
  const { data: parallelWorkStats } = useParallelWorkStats({
    range: "today",
    bucket: "1m",
    upstreamAccountId,
  });
  const { data: comparisonParallelWorkStats } = useParallelWorkStats({
    range: "yesterday",
    bucket: "1m",
    upstreamAccountId,
  });
  const [rateNow, setRateNow] = useState(() => new Date());

  useEffect(() => {
    if (closedNaturalDay) return;
    setRateNow(new Date());
    const timer = window.setInterval(() => {
      setRateNow(new Date());
    }, LIVE_RATE_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [closedNaturalDay]);

  const snapshotRate =
    upstreamAccountId == null && dashboardActivity?.range === "today"
      ? {
          tokensPerMinute: dashboardActivity.summary.tokensPerMinute ?? 0,
          spendRate: dashboardActivity.summary.spendRate ?? 0,
          windowMinutes: dashboardActivity.rateWindow.windowMinutes,
          available: true,
          currentFirstResponseByteTotalAvgMs:
            dashboardActivity.summary.currentFirstResponseByteTotalAvgMs ?? null,
          currentAvgTotalMs: dashboardActivity.summary.currentAvgTotalMs ?? null,
        }
      : null;
  const snapshotActive = upstreamAccountId == null && dashboardActivity?.range === "today";

  return (
    <TodayStatsOverview
      stats={summary}
      loading={summaryLoading || dashboardActivityLoading}
      error={summaryError ?? dashboardActivityError}
      rate={snapshotRate}
      rateLoading={dashboardActivityLoading}
      rateError={dashboardActivityError}
      now={rateNow}
      timeseries={response}
      comparisonStats={comparisonSummary}
      comparisonTimeseries={comparisonTimeseries}
      previous7dStats={previous7dSummary}
      parallelWorkStats={parallelWorkStats}
      comparisonParallelWorkStats={comparisonParallelWorkStats}
      showInProgressConversations
      dayKind="today"
      showSurface={false}
      showHeader={false}
      showDayBadge={false}
      modelPerformance={snapshotActive ? dashboardActivity?.summary.modelPerformance : null}
    />
  );
}

function DashboardNaturalDayYesterdaySummaryOverview({
  response,
  closedNaturalDay,
  upstreamAccountId,
  dashboardActivity,
  dashboardActivityLoading = false,
  dashboardActivityError = null,
}: {
  response: ReturnType<typeof useTimeseries>["data"];
  closedNaturalDay: boolean;
  upstreamAccountId?: number;
  dashboardActivity?: DashboardActivityResponse | null;
  dashboardActivityLoading?: boolean;
  dashboardActivityError?: string | null;
}) {
  const snapshotActive = upstreamAccountId == null && dashboardActivity?.range === "yesterday";
  if (snapshotActive && dashboardActivity != null) {
    return (
      <DashboardNaturalDayYesterdaySummaryOverviewSnapshotBacked
        response={response}
        closedNaturalDay={closedNaturalDay}
        dashboardActivity={dashboardActivity}
      />
    );
  }

  return (
    <DashboardNaturalDayYesterdaySummaryOverviewFallback
      response={response}
      closedNaturalDay={closedNaturalDay}
      upstreamAccountId={upstreamAccountId}
      dashboardActivity={dashboardActivity}
      dashboardActivityLoading={dashboardActivityLoading}
      dashboardActivityError={dashboardActivityError}
    />
  );
}

function DashboardNaturalDayYesterdaySummaryOverviewSnapshotBacked({
  response,
  closedNaturalDay,
  dashboardActivity,
}: {
  response: ReturnType<typeof useTimeseries>["data"];
  closedNaturalDay: boolean;
  dashboardActivity: DashboardActivityResponse;
}) {
  const { summary: previous7dSummary } = useScopedSummary("previous7d");
  const { data: parallelWorkStats } = useParallelWorkStats({
    range: "yesterday",
    bucket: "1m",
  });
  const [rateNow, setRateNow] = useState(() => new Date());

  useEffect(() => {
    if (closedNaturalDay) return;
    setRateNow(new Date());
    const timer = window.setInterval(() => {
      setRateNow(new Date());
    }, LIVE_RATE_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [closedNaturalDay]);

  return (
    <TodayStatsOverview
      stats={dashboardActivity.summary.stats}
      loading={false}
      error={null}
      rate={{
        tokensPerMinute: dashboardActivity.summary.tokensPerMinute ?? 0,
        spendRate: dashboardActivity.summary.spendRate ?? 0,
        windowMinutes: dashboardActivity.rateWindow.windowMinutes,
        available: true,
        currentFirstResponseByteTotalAvgMs:
          dashboardActivity.summary.currentFirstResponseByteTotalAvgMs ?? null,
        currentAvgTotalMs: dashboardActivity.summary.currentAvgTotalMs ?? null,
      }}
      rateLoading={false}
      rateError={null}
      now={rateNow}
      timeseries={response}
      comparisonStats={null}
      comparisonTimeseries={null}
      previous7dStats={previous7dSummary}
      parallelWorkStats={parallelWorkStats}
      comparisonParallelWorkStats={null}
      showInProgressConversations
      dayKind="yesterday"
      showSurface={false}
      showHeader={false}
      showDayBadge={false}
      modelPerformance={dashboardActivity.summary.modelPerformance}
    />
  );
}

function DashboardNaturalDayYesterdaySummaryOverviewFallback({
  response,
  closedNaturalDay,
  upstreamAccountId,
  dashboardActivity,
  dashboardActivityLoading = false,
  dashboardActivityError = null,
}: {
  response: ReturnType<typeof useTimeseries>["data"];
  closedNaturalDay: boolean;
  upstreamAccountId?: number;
  dashboardActivity?: DashboardActivityResponse | null;
  dashboardActivityLoading?: boolean;
  dashboardActivityError?: string | null;
}) {
  const {
    summary,
    isLoading: summaryLoading,
    error: summaryError,
  } = useScopedSummary("yesterday", upstreamAccountId);
  const { summary: previous7dSummary } = useScopedSummary("previous7d", upstreamAccountId);
  const { data: parallelWorkStats } = useParallelWorkStats({
    range: "yesterday",
    bucket: "1m",
    upstreamAccountId,
  });
  const [rateNow, setRateNow] = useState(() => new Date());

  useEffect(() => {
    if (closedNaturalDay) return;
    setRateNow(new Date());
    const timer = window.setInterval(() => {
      setRateNow(new Date());
    }, LIVE_RATE_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [closedNaturalDay]);

  const snapshotRate =
    upstreamAccountId == null && dashboardActivity?.range === "yesterday"
      ? {
          tokensPerMinute: dashboardActivity.summary.tokensPerMinute ?? 0,
          spendRate: dashboardActivity.summary.spendRate ?? 0,
          windowMinutes: dashboardActivity.rateWindow.windowMinutes,
          available: true,
          currentFirstResponseByteTotalAvgMs:
            dashboardActivity.summary.currentFirstResponseByteTotalAvgMs ?? null,
          currentAvgTotalMs: dashboardActivity.summary.currentAvgTotalMs ?? null,
        }
      : null;
  const snapshotActive = upstreamAccountId == null && dashboardActivity?.range === "yesterday";

  return (
    <TodayStatsOverview
      stats={summary}
      loading={summaryLoading || dashboardActivityLoading}
      error={summaryError ?? dashboardActivityError}
      rate={snapshotRate}
      rateLoading={dashboardActivityLoading}
      rateError={dashboardActivityError}
      now={rateNow}
      timeseries={response}
      comparisonStats={null}
      comparisonTimeseries={null}
      previous7dStats={previous7dSummary}
      parallelWorkStats={parallelWorkStats}
      comparisonParallelWorkStats={null}
      showInProgressConversations
      dayKind="yesterday"
      showSurface={false}
      showHeader={false}
      showDayBadge={false}
      modelPerformance={snapshotActive ? dashboardActivity?.summary.modelPerformance : null}
    />
  );
}

const DashboardNaturalDayChartSection = memo(function DashboardNaturalDayChartSection({
  response,
  loading,
  error,
  metric,
  closedNaturalDay,
  networkResponse,
  networkLoading,
  networkError,
}: {
  response: ReturnType<typeof useTimeseries>["data"];
  loading: boolean;
  error: ReturnType<typeof useTimeseries>["error"];
  metric: NaturalDayChartMetric;
  closedNaturalDay: boolean;
  networkResponse: ReturnType<typeof useDashboardNetworkTimeseries>["data"];
  networkLoading: boolean;
  networkError: string | null;
}) {
  if (metric === "network") {
    return (
      <DashboardNetworkActivityChart
        response={networkResponse}
        loading={networkLoading && networkResponse == null}
        error={networkError}
      />
    );
  }
  return (
    <DashboardTodayActivityChart
      response={response}
      loading={loading}
      error={error}
      metric={metric}
      closedNaturalDay={closedNaturalDay}
    />
  );
});

function DashboardTodayRangePanel({
  metric,
  upstreamAccountId,
  dashboardActivity,
  dashboardActivityLoading,
  dashboardActivityError,
}: {
  metric: NaturalDayChartMetric;
  upstreamAccountId?: number;
  dashboardActivity?: DashboardActivityResponse | null;
  dashboardActivityLoading?: boolean;
  dashboardActivityError?: string | null;
}) {
  return (
    <DashboardNaturalDayRangePanel
      metric={metric}
      summaryWindow="today"
      timeseriesRange="today"
      testId="dashboard-activity-range-today"
      upstreamAccountId={upstreamAccountId}
      dashboardActivity={dashboardActivity}
      dashboardActivityLoading={dashboardActivityLoading}
      dashboardActivityError={dashboardActivityError}
    />
  );
}

function DashboardYesterdayRangePanel({
  metric,
  upstreamAccountId,
  dashboardActivity,
  dashboardActivityLoading,
  dashboardActivityError,
}: {
  metric: NaturalDayChartMetric;
  upstreamAccountId?: number;
  dashboardActivity?: DashboardActivityResponse | null;
  dashboardActivityLoading?: boolean;
  dashboardActivityError?: string | null;
}) {
  return (
    <DashboardNaturalDayRangePanel
      metric={metric}
      summaryWindow="yesterday"
      timeseriesRange="yesterday"
      testId="dashboard-activity-range-yesterday"
      upstreamAccountId={upstreamAccountId}
      dashboardActivity={dashboardActivity}
      dashboardActivityLoading={dashboardActivityLoading}
      dashboardActivityError={dashboardActivityError}
    />
  );
}

function Dashboard24HourRangePanel({
  metric,
  upstreamAccountId,
  dashboardActivity,
  dashboardActivityLoading,
  dashboardActivityError,
}: {
  metric: Dashboard24HourMetric;
  upstreamAccountId?: number;
  dashboardActivity?: DashboardActivityResponse | null;
  dashboardActivityLoading?: boolean;
  dashboardActivityError?: string | null;
}) {
  const { summary, isLoading, error } = useScopedSummary("1d", upstreamAccountId);
  const snapshotActive = upstreamAccountId == null && dashboardActivity?.range === "1d";
  const {
    data: networkData,
    isLoading: networkLoading,
    error: networkError,
  } = useDashboardNetworkTimeseries("1d", metric === "network", upstreamAccountId);

  return (
    <div
      className="flex flex-col gap-5"
      data-testid="dashboard-activity-range-1d"
      data-active="true"
    >
      <StatsCards
        stats={snapshotActive ? dashboardActivity.summary.stats : summary}
        loading={snapshotActive ? false : isLoading || dashboardActivityLoading === true}
        error={snapshotActive ? null : (error ?? dashboardActivityError)}
      />
      {metric === "network" ? (
        <DashboardNetworkActivityChart
          response={networkData}
          loading={networkLoading && networkData == null}
          error={networkError}
        />
      ) : (
        <Last24hTenMinuteHeatmap
          metric={metric}
          showHeader={false}
          upstreamAccountId={upstreamAccountId}
        />
      )}
    </div>
  );
}

function Dashboard7DayRangePanel({
  metric,
  upstreamAccountId,
  dashboardActivity,
  dashboardActivityLoading,
  dashboardActivityError,
}: {
  metric: MetricKey;
  upstreamAccountId?: number;
  dashboardActivity?: DashboardActivityResponse | null;
  dashboardActivityLoading?: boolean;
  dashboardActivityError?: string | null;
}) {
  const { summary, isLoading, error } = useScopedSummary("7d", upstreamAccountId);
  const snapshotActive = upstreamAccountId == null && dashboardActivity?.range === "7d";

  return (
    <div
      className="flex flex-col gap-5"
      data-testid="dashboard-activity-range-7d"
      data-active="true"
    >
      <StatsCards
        stats={snapshotActive ? dashboardActivity.summary.stats : summary}
        loading={snapshotActive ? false : isLoading || dashboardActivityLoading === true}
        error={snapshotActive ? null : (error ?? dashboardActivityError)}
      />
      <WeeklyHourlyHeatmap
        metric={metric}
        showHeader={false}
        showSurface={false}
        upstreamAccountId={upstreamAccountId}
      />
    </div>
  );
}

function DashboardUsageRangePanel({
  metric,
  upstreamAccountId,
}: {
  metric: MetricKey;
  upstreamAccountId?: number;
}) {
  return (
    <div data-testid="dashboard-activity-range-usage" data-active="true">
      <UsageCalendar
        metric={metric}
        showSurface={false}
        showMetricToggle={false}
        showMeta={false}
        upstreamAccountId={upstreamAccountId}
      />
    </div>
  );
}

function DashboardTodaySnapshotRangePanel({
  metric,
  bundle,
}: {
  metric: NaturalDayChartMetric;
  bundle: DashboardOverviewSnapshotBundle;
}) {
  const dashboardActivity = bundle.dashboardActivity;
  const rateNow = useSnapshotRateNow(true);

  return (
    <div
      className="flex flex-col gap-5"
      data-testid="dashboard-activity-range-today"
      data-active="true"
    >
      <TodayStatsOverview
        stats={dashboardActivity?.summary.stats ?? null}
        loading={false}
        error={null}
        rate={dashboardActivity ? buildDashboardActivityRate(dashboardActivity) : null}
        rateLoading={false}
        rateError={null}
        now={rateNow}
        timeseries={bundle.timeseries ?? null}
        comparisonStats={bundle.comparisonSummary ?? null}
        comparisonTimeseries={bundle.comparisonTimeseries ?? null}
        previous7dStats={bundle.previous7dSummary ?? null}
        parallelWorkStats={bundle.parallelWorkStats ?? null}
        comparisonParallelWorkStats={bundle.comparisonParallelWorkStats ?? null}
        showInProgressConversations
        dayKind="today"
        showSurface={false}
        showHeader={false}
        showDayBadge={false}
        modelPerformance={dashboardActivity?.summary.modelPerformance ?? null}
      />
      <DashboardNaturalDayChartSection
        response={bundle.timeseries ?? null}
        loading={false}
        error={null}
        metric={metric}
        closedNaturalDay={false}
        networkResponse={bundle.networkTimeseries ?? null}
        networkLoading={false}
        networkError={null}
      />
    </div>
  );
}

function DashboardYesterdaySnapshotRangePanel({
  metric,
  bundle,
}: {
  metric: NaturalDayChartMetric;
  bundle: DashboardOverviewSnapshotBundle;
}) {
  const dashboardActivity = bundle.dashboardActivity;
  const rateNow = useSnapshotRateNow(true);

  return (
    <div
      className="flex flex-col gap-5"
      data-testid="dashboard-activity-range-yesterday"
      data-active="true"
    >
      <TodayStatsOverview
        stats={dashboardActivity?.summary.stats ?? null}
        loading={false}
        error={null}
        rate={dashboardActivity ? buildDashboardActivityRate(dashboardActivity) : null}
        rateLoading={false}
        rateError={null}
        now={rateNow}
        timeseries={bundle.timeseries ?? null}
        comparisonStats={null}
        comparisonTimeseries={null}
        previous7dStats={bundle.previous7dSummary ?? null}
        parallelWorkStats={bundle.parallelWorkStats ?? null}
        comparisonParallelWorkStats={null}
        showInProgressConversations
        dayKind="yesterday"
        showSurface={false}
        showHeader={false}
        showDayBadge={false}
        modelPerformance={dashboardActivity?.summary.modelPerformance ?? null}
      />
      <DashboardNaturalDayChartSection
        response={bundle.timeseries ?? null}
        loading={false}
        error={null}
        metric={metric}
        closedNaturalDay
        networkResponse={bundle.networkTimeseries ?? null}
        networkLoading={false}
        networkError={null}
      />
    </div>
  );
}

function Dashboard24HourSnapshotRangePanel({
  metric,
  bundle,
}: {
  metric: Dashboard24HourMetric;
  bundle: DashboardOverviewSnapshotBundle;
}) {
  return (
    <div
      className="flex flex-col gap-5"
      data-testid="dashboard-activity-range-1d"
      data-active="true"
    >
      <StatsCards
        stats={bundle.dashboardActivity?.summary.stats ?? bundle.summary ?? null}
        loading={false}
        error={null}
      />
      {metric === "network" ? (
        <DashboardNetworkActivityChart
          response={bundle.networkTimeseries ?? null}
          loading={false}
          error={null}
        />
      ) : (
        <Last24hTenMinuteHeatmap
          metric={metric}
          showHeader={false}
          timeseriesResponse={bundle.timeseries ?? null}
        />
      )}
    </div>
  );
}

function Dashboard7DaySnapshotRangePanel({
  metric,
  bundle,
}: {
  metric: MetricKey;
  bundle: DashboardOverviewSnapshotBundle;
}) {
  return (
    <div
      className="flex flex-col gap-5"
      data-testid="dashboard-activity-range-7d"
      data-active="true"
    >
      <StatsCards
        stats={bundle.dashboardActivity?.summary.stats ?? bundle.summary ?? null}
        loading={false}
        error={null}
      />
      <WeeklyHourlyHeatmap
        metric={metric}
        showHeader={false}
        showSurface={false}
        timeseriesResponse={bundle.timeseries ?? null}
      />
    </div>
  );
}

function DashboardUsageSnapshotRangePanel({
  metric,
  bundle,
}: {
  metric: MetricKey;
  bundle: DashboardOverviewSnapshotBundle;
}) {
  return (
    <div data-testid="dashboard-activity-range-usage" data-active="true">
      <UsageCalendar
        metric={metric}
        showSurface={false}
        showMetricToggle={false}
        showMeta={false}
        timeseriesResponse={bundle.timeseries ?? null}
      />
    </div>
  );
}

export interface DashboardActivityOverviewProps {
  title?: string;
  storageKey?: string;
  testId?: string;
  upstreamAccountId?: number;
  className?: string;
  activeRange?: DashboardActivityRangeKey;
  onActiveRangeChange?: (range: DashboardActivityRangeKey) => void;
  dashboardActivity?: DashboardActivityResponse | null;
  dashboardActivityLoading?: boolean;
  dashboardActivityError?: string | null;
  snapshotStatus?: DashboardOverviewSnapshotStatus | null;
  snapshotBundle?: DashboardOverviewSnapshotBundle | null;
}

export function DashboardActivityOverview({
  title,
  storageKey = DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
  testId = "dashboard-activity-overview",
  upstreamAccountId,
  className = "surface-panel overflow-visible",
  activeRange: controlledActiveRange,
  onActiveRangeChange,
  dashboardActivity,
  dashboardActivityLoading = false,
  dashboardActivityError = null,
  snapshotStatus = null,
  snapshotBundle = null,
}: DashboardActivityOverviewProps) {
  const { t, locale } = useTranslation();
  const { themeMode } = useTheme();
  const [uncontrolledActiveRange, setUncontrolledActiveRange] = useState<DashboardActivityRangeKey>(
    () => readPersistedDashboardActivityRange(storageKey),
  );
  const [metricToday, setMetricToday] = useState<NaturalDayChartMetric>("totalCount");
  const [metricYesterday, setMetricYesterday] = useState<NaturalDayChartMetric>("totalCount");
  const [metric24h, setMetric24h] = useState<Dashboard24HourMetric>("totalCount");
  const [metric7d, setMetric7d] = useState<MetricKey>("totalCount");
  const [metricUsage, setMetricUsage] = useState<MetricKey>("totalCount");

  const activeRange = controlledActiveRange ?? uncontrolledActiveRange;
  const setActiveRange = (range: DashboardActivityRangeKey) => {
    if (controlledActiveRange == null) {
      setUncontrolledActiveRange(range);
    }
    onActiveRangeChange?.(range);
  };

  const rangeOptions = useMemo(
    () => RANGE_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  );
  const metricOptions = useMemo(() => {
    const source =
      activeRange === "today" || activeRange === "yesterday"
        ? NATURAL_DAY_METRIC_OPTIONS
        : activeRange === "1d"
          ? RANGE_24H_METRIC_OPTIONS
          : METRIC_OPTIONS;
    return source.map((option) => ({ ...option, label: t(option.labelKey) }));
  }, [activeRange, t]);

  const activeMetric =
    activeRange === "today"
      ? metricToday
      : activeRange === "yesterday"
        ? metricYesterday
        : activeRange === "1d"
          ? metric24h
          : activeRange === "7d"
            ? metric7d
            : metricUsage;
  const snapshotMode = upstreamAccountId == null ? (snapshotStatus?.mode ?? "live") : "live";
  const snapshotReadyRanges = snapshotStatus?.readyRanges ?? [];
  const snapshotCachedAtLabel = formatDashboardSnapshotCachedAt(
    snapshotStatus?.cachedAt ?? null,
    locale === "zh" ? "zh-CN" : "en-US",
  );
  const showSnapshotEmptyState = upstreamAccountId == null && snapshotMode === "not-cached-yet";
  const showSnapshotRange =
    upstreamAccountId == null &&
    snapshotMode === "cached-offline" &&
    snapshotBundle != null &&
    snapshotBundle.range === activeRange;

  useEffect(() => {
    if (controlledActiveRange == null) {
      persistDashboardActivityRange(storageKey, activeRange);
    }
  }, [activeRange, controlledActiveRange, storageKey]);

  const setActiveMetric = (metric: NaturalDayChartMetric | Dashboard24HourMetric | MetricKey) => {
    if (activeRange === "today") {
      setMetricToday(metric as NaturalDayChartMetric);
      return;
    }
    if (activeRange === "yesterday") {
      setMetricYesterday(metric as NaturalDayChartMetric);
      return;
    }
    if (metric === "trend") return;
    if (activeRange === "1d") {
      setMetric24h(metric as Dashboard24HourMetric);
      return;
    }
    if (activeRange === "7d") {
      setMetric7d(metric as MetricKey);
      return;
    }
    setMetricUsage(metric as MetricKey);
  };

  return (
    <section
      className={className}
      data-testid={testId}
      data-snapshot-mode={snapshotMode}
      data-snapshot-ready-ranges={snapshotReadyRanges.join(",")}
      data-snapshot-cached-at={snapshotStatus?.cachedAt ?? ""}
    >
      <div className="surface-panel-body gap-6">
        <div className="space-y-3 min-[769px]:flex min-[769px]:items-start min-[769px]:justify-between min-[769px]:gap-3 min-[769px]:space-y-0">
          <div className="flex max-w-full items-center gap-3">
            <div className="section-heading">
              <h2 className="section-title">{title ?? t("dashboard.activityOverview.title")}</h2>
            </div>
            <SegmentedControl
              className="hidden max-w-full flex-wrap min-[769px]:flex"
              role="tablist"
              aria-label={t("dashboard.activityOverview.rangeToggleAria")}
            >
              {rangeOptions.map((option) => {
                const active = option.key === activeRange;
                return (
                  <SegmentedControlItem
                    key={option.key}
                    active={active}
                    role="tab"
                    aria-selected={active}
                    onClick={() => setActiveRange(option.key)}
                  >
                    {option.label}
                  </SegmentedControlItem>
                );
              })}
            </SegmentedControl>
          </div>
          <SegmentedControl
            className="hidden min-[769px]:flex"
            size="compact"
            role="tablist"
            aria-label={t("heatmap.metricsToggleAria")}
          >
            {metricOptions.map((option) => {
              const active = option.key === activeMetric;
              return (
                <SegmentedControlItem
                  key={option.key}
                  active={active}
                  role="tab"
                  aria-selected={active}
                  style={
                    active && option.key !== "trend"
                      ? {
                          color: resolveDashboardMetricAccent(
                            option.key as OverviewMetricKey,
                            themeMode,
                          ),
                        }
                      : undefined
                  }
                  onClick={() => setActiveMetric(option.key)}
                >
                  {option.label}
                </SegmentedControlItem>
              );
            })}
          </SegmentedControl>
          <div
            className="grid grid-cols-2 gap-2 min-[769px]:hidden"
            data-testid="dashboard-activity-mobile-selects"
          >
            <SelectField
              aria-label={t("dashboard.activityOverview.rangeToggleAria")}
              data-testid="dashboard-activity-range-select"
              options={rangeOptions.map((option) => ({ value: option.key, label: option.label }))}
              value={activeRange}
              onValueChange={(value) => {
                const nextRange = rangeOptions.find((option) => option.key === value);
                if (nextRange) setActiveRange(nextRange.key);
              }}
              triggerClassName="h-11 min-w-0 px-3 text-sm font-medium shadow-none"
            />
            <SelectField
              aria-label={t("heatmap.metricsToggleAria")}
              data-testid="dashboard-activity-metric-select"
              options={metricOptions.map((option) => ({ value: option.key, label: option.label }))}
              value={activeMetric}
              onValueChange={(value) => {
                const nextMetric = metricOptions.find((option) => option.key === value);
                if (nextMetric) setActiveMetric(nextMetric.key);
              }}
              triggerClassName="h-11 min-w-0 px-3 text-sm font-medium shadow-none"
            />
          </div>
        </div>
        <DashboardOverviewSnapshotBanner
          mode={snapshotMode}
          cachedAtLabel={snapshotCachedAtLabel}
          readyRangeCount={snapshotReadyRanges.length}
          totalRangeCount={RANGE_OPTIONS.length}
          t={t}
        />
        {showSnapshotEmptyState ? null : activeRange === "today" &&
          showSnapshotRange &&
          snapshotBundle ? (
          <DashboardTodaySnapshotRangePanel metric={metricToday} bundle={snapshotBundle} />
        ) : activeRange === "today" ? (
          <DashboardTodayRangePanel
            metric={metricToday}
            upstreamAccountId={upstreamAccountId}
            dashboardActivity={dashboardActivity}
            dashboardActivityLoading={dashboardActivityLoading}
            dashboardActivityError={dashboardActivityError}
          />
        ) : null}
        {showSnapshotEmptyState ? null : activeRange === "yesterday" &&
          showSnapshotRange &&
          snapshotBundle ? (
          <DashboardYesterdaySnapshotRangePanel metric={metricYesterday} bundle={snapshotBundle} />
        ) : activeRange === "yesterday" ? (
          <DashboardYesterdayRangePanel
            metric={metricYesterday}
            upstreamAccountId={upstreamAccountId}
            dashboardActivity={dashboardActivity}
            dashboardActivityLoading={dashboardActivityLoading}
            dashboardActivityError={dashboardActivityError}
          />
        ) : null}
        {showSnapshotEmptyState ? null : activeRange === "1d" &&
          showSnapshotRange &&
          snapshotBundle ? (
          <Dashboard24HourSnapshotRangePanel metric={metric24h} bundle={snapshotBundle} />
        ) : activeRange === "1d" ? (
          <Dashboard24HourRangePanel
            metric={metric24h}
            upstreamAccountId={upstreamAccountId}
            dashboardActivity={dashboardActivity}
            dashboardActivityLoading={dashboardActivityLoading}
            dashboardActivityError={dashboardActivityError}
          />
        ) : null}
        {showSnapshotEmptyState ? null : activeRange === "7d" &&
          showSnapshotRange &&
          snapshotBundle ? (
          <Dashboard7DaySnapshotRangePanel metric={metric7d} bundle={snapshotBundle} />
        ) : activeRange === "7d" ? (
          <Dashboard7DayRangePanel
            metric={metric7d}
            upstreamAccountId={upstreamAccountId}
            dashboardActivity={dashboardActivity}
            dashboardActivityLoading={dashboardActivityLoading}
            dashboardActivityError={dashboardActivityError}
          />
        ) : null}
        {showSnapshotEmptyState ? null : activeRange === "usage" &&
          showSnapshotRange &&
          snapshotBundle ? (
          <DashboardUsageSnapshotRangePanel metric={metricUsage} bundle={snapshotBundle} />
        ) : activeRange === "usage" ? (
          <DashboardUsageRangePanel metric={metricUsage} upstreamAccountId={upstreamAccountId} />
        ) : null}
      </div>
    </section>
  );
}

export default DashboardActivityOverview;
