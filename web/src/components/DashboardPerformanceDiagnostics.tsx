import { useMemo } from "react";
import {
  useDashboardPerformanceDiagnosticsEnabled,
  useDashboardPerformanceDiagnosticsSnapshot,
} from "../lib/dashboardPerformanceDiagnostics";

function formatTimestamp(value: string | null) {
  if (!value) return "never";
  return value;
}

export function DashboardPerformanceDiagnostics() {
  const enabled = useDashboardPerformanceDiagnosticsEnabled();

  if (!enabled) {
    return null;
  }

  return <DashboardPerformanceDiagnosticsPanel />;
}

function DashboardPerformanceDiagnosticsPanel() {
  const snapshot = useDashboardPerformanceDiagnosticsSnapshot();

  const rows = useMemo(
    () => [
      {
        key: "working-conversations-patch-bucket-count",
        label: "Working conversation patch buckets",
        value: String(snapshot.workingConversationPatchBucketCount),
        updatedAt: snapshot.workingConversationPatchLastUpdatedAt,
      },
      {
        key: "working-conversations-patch-entry-count",
        label: "Working conversation patch entries",
        value: String(snapshot.workingConversationPatchEntryCount),
        updatedAt: snapshot.workingConversationPatchLastUpdatedAt,
      },
      {
        key: "working-conversations-head-fetch-count",
        label: "Working conversation head fetches",
        value: String(snapshot.workingConversationHeadFetchCount),
        updatedAt: snapshot.workingConversationHeadFetchLastUpdatedAt,
      },
      {
        key: "today-summary-refresh-count",
        label: "Today summary HTTP reconciles",
        value: String(snapshot.todaySummaryRefreshCount),
        updatedAt: snapshot.todaySummaryLastUpdatedAt,
      },
      {
        key: "current-summary-refresh-count",
        label: "Current summary HTTP reconciles",
        value: String(snapshot.currentSummaryRefreshCount),
        updatedAt: snapshot.currentSummaryLastUpdatedAt,
      },
      {
        key: "current-summary-open-resync-count",
        label: "Current summary open resyncs",
        value: String(snapshot.currentSummaryOpenResyncCount),
        updatedAt: snapshot.currentSummaryOpenResyncLastUpdatedAt,
      },
      {
        key: "today-summary-sse-commit-count",
        label: "Today summary SSE commits",
        value: String(snapshot.todaySummarySseCommitCount),
        updatedAt: snapshot.todaySummarySseCommitLastUpdatedAt,
      },
      {
        key: "today-chart-data-commit-count",
        label: "Today chart data commits",
        value: String(snapshot.todayChartDataCommitCount),
        updatedAt: snapshot.todayChartDataCommitLastUpdatedAt,
      },
      {
        key: "today-chart-render-count",
        label: "Today chart renders",
        value: String(snapshot.todayChartRenderCount),
        updatedAt: snapshot.todayChartLastRenderedAt,
      },
      {
        key: "parallel-work-full-fetch-count",
        label: "Parallel-work full fetches",
        value: String(snapshot.parallelWorkFullFetchCount),
        updatedAt: snapshot.parallelWorkLastUpdatedAt,
      },
      {
        key: "upstream-account-activity-refresh-count",
        label: "Upstream account activity HTTP reconciles",
        value: String(snapshot.upstreamAccountActivityRefreshCount),
        updatedAt: snapshot.upstreamAccountActivityRefreshLastUpdatedAt,
      },
      {
        key: "upstream-account-activity-open-resync-count",
        label: "Upstream account activity open resyncs",
        value: String(snapshot.upstreamAccountActivityOpenResyncCount),
        updatedAt: snapshot.upstreamAccountActivityOpenResyncLastUpdatedAt,
      },
      {
        key: "parallel-work-not-modified-count",
        label: "Parallel-work 304 hits",
        value: String(snapshot.parallelWorkNotModifiedCount),
        updatedAt: snapshot.parallelWorkLastUpdatedAt,
      },
    ],
    [snapshot],
  );

  return (
    <section
      className="surface-panel border border-warning/30 bg-warning/5"
      data-testid="dashboard-performance-diagnostics"
    >
      <div className="surface-panel-body gap-3">
        <div className="section-heading">
          <h2 className="section-title text-sm">
            Dashboard diagnostics (debug)
          </h2>
        </div>
        <dl className="grid gap-3 md:grid-cols-2">
          {rows.map((row) => (
            <div
              key={row.key}
              className="rounded-lg border border-base-300/80 bg-base-100/80 px-3 py-2"
            >
              <dt className="text-xs font-medium text-base-content/70">
                {row.label}
              </dt>
              <dd
                className="mt-1 text-lg font-semibold text-base-content"
                data-testid={`dashboard-performance-diagnostics-${row.key}`}
              >
                {row.value}
              </dd>
              <div
                className="mt-1 text-[11px] text-base-content/55"
                data-testid={`dashboard-performance-diagnostics-${row.key}-updated-at`}
              >
                {formatTimestamp(row.updatedAt)}
              </div>
            </div>
          ))}
        </dl>
      </div>
    </section>
  );
}

export default DashboardPerformanceDiagnostics;
