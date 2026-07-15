import { memo, useEffect, useMemo, useRef, useState } from "react";
import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import { SelectField } from "../../components/ui/select-field";
import { useParallelWorkStats } from "../../hooks/useParallelWorkStats";
import { useSummary } from "../../hooks/useStats";
import { useTimeseries } from "../../hooks/useTimeseries";
import { useTranslation } from "../../i18n";
import type { DashboardActivityResponse } from "../../lib/api";
import { metricAccent } from "../../lib/chartTheme";
import { recordTodayChartDataCommit } from "../../lib/dashboardPerformanceDiagnostics";
import { useTheme } from "../../theme";
import { StatsCards } from "../stats/StatsCards";
import { DashboardTodayActivityChart } from "./DashboardTodayActivityChart";
import {
  DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
  type DashboardActivityRangeKey,
  persistDashboardActivityRange,
  readPersistedDashboardActivityRange,
} from "./dashboardActivityRange";
import { Last24hTenMinuteHeatmap, type MetricKey } from "./Last24hTenMinuteHeatmap";
import { TodayStatsOverview } from "./TodayStatsOverview";
import { UsageCalendar } from "./UsageCalendar";
import { WeeklyHourlyHeatmap } from "./WeeklyHourlyHeatmap";

type NaturalDayChartMetric = MetricKey | "trend";

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
const NATURAL_DAY_METRIC_OPTIONS: Array<{ key: NaturalDayChartMetric; labelKey: string }> = [
  ...METRIC_OPTIONS,
  { key: "trend", labelKey: "chart.trend" },
];

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
}: {
  response: ReturnType<typeof useTimeseries>["data"];
  loading: boolean;
  error: ReturnType<typeof useTimeseries>["error"];
  metric: NaturalDayChartMetric;
  closedNaturalDay: boolean;
}) {
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
  metric: MetricKey;
  upstreamAccountId?: number;
  dashboardActivity?: DashboardActivityResponse | null;
  dashboardActivityLoading?: boolean;
  dashboardActivityError?: string | null;
}) {
  const { summary, isLoading, error } = useScopedSummary("1d", upstreamAccountId);
  const snapshotActive = upstreamAccountId == null && dashboardActivity?.range === "1d";

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
      <Last24hTenMinuteHeatmap
        metric={metric}
        showHeader={false}
        upstreamAccountId={upstreamAccountId}
      />
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
}: DashboardActivityOverviewProps) {
  const { t } = useTranslation();
  const { themeMode } = useTheme();
  const [uncontrolledActiveRange, setUncontrolledActiveRange] = useState<DashboardActivityRangeKey>(
    () => readPersistedDashboardActivityRange(storageKey),
  );
  const [metricToday, setMetricToday] = useState<NaturalDayChartMetric>("totalCount");
  const [metricYesterday, setMetricYesterday] = useState<NaturalDayChartMetric>("totalCount");
  const [metric24h, setMetric24h] = useState<MetricKey>("totalCount");
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

  useEffect(() => {
    if (controlledActiveRange == null) {
      persistDashboardActivityRange(storageKey, activeRange);
    }
  }, [activeRange, controlledActiveRange, storageKey]);

  const setActiveMetric = (metric: NaturalDayChartMetric) => {
    if (activeRange === "today") {
      setMetricToday(metric);
      return;
    }
    if (activeRange === "yesterday") {
      setMetricYesterday(metric);
      return;
    }
    if (metric === "trend") return;
    if (activeRange === "1d") {
      setMetric24h(metric);
      return;
    }
    if (activeRange === "7d") {
      setMetric7d(metric);
      return;
    }
    setMetricUsage(metric);
  };

  return (
    <section className={className} data-testid={testId}>
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
                      ? { color: metricAccent(option.key, themeMode) }
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
        {activeRange === "today" ? (
          <DashboardTodayRangePanel
            metric={metricToday}
            upstreamAccountId={upstreamAccountId}
            dashboardActivity={dashboardActivity}
            dashboardActivityLoading={dashboardActivityLoading}
            dashboardActivityError={dashboardActivityError}
          />
        ) : null}
        {activeRange === "yesterday" ? (
          <DashboardYesterdayRangePanel
            metric={metricYesterday}
            upstreamAccountId={upstreamAccountId}
            dashboardActivity={dashboardActivity}
            dashboardActivityLoading={dashboardActivityLoading}
            dashboardActivityError={dashboardActivityError}
          />
        ) : null}
        {activeRange === "1d" ? (
          <Dashboard24HourRangePanel
            metric={metric24h}
            upstreamAccountId={upstreamAccountId}
            dashboardActivity={dashboardActivity}
            dashboardActivityLoading={dashboardActivityLoading}
            dashboardActivityError={dashboardActivityError}
          />
        ) : null}
        {activeRange === "7d" ? (
          <Dashboard7DayRangePanel
            metric={metric7d}
            upstreamAccountId={upstreamAccountId}
            dashboardActivity={dashboardActivity}
            dashboardActivityLoading={dashboardActivityLoading}
            dashboardActivityError={dashboardActivityError}
          />
        ) : null}
        {activeRange === "usage" ? (
          <DashboardUsageRangePanel metric={metricUsage} upstreamAccountId={upstreamAccountId} />
        ) : null}
      </div>
    </section>
  );
}

export default DashboardActivityOverview;
