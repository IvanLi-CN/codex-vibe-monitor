import { useEffect, useMemo } from "react";
import {
  syncDashboardPerformanceDiagnosticsEnabled,
  useDashboardPerformanceDiagnosticsSnapshot,
} from "../lib/dashboardPerformanceDiagnostics";

function formatTimestamp(value: string | null) {
  if (!value) return "never";
  return value;
}

export function DashboardPerformanceDiagnostics() {
  const snapshot = useDashboardPerformanceDiagnosticsSnapshot();

  useEffect(() => {
    syncDashboardPerformanceDiagnosticsEnabled();
  }, []);

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
        key: "today-summary-refresh-count",
        label: "Today summary refreshes",
        value: String(snapshot.todaySummaryRefreshCount),
        updatedAt: snapshot.todaySummaryLastUpdatedAt,
      },
      {
        key: "today-chart-render-count",
        label: "Today chart renders",
        value: String(snapshot.todayChartRenderCount),
        updatedAt: snapshot.todayChartLastRenderedAt,
      },
    ],
    [snapshot],
  );

  if (!snapshot.enabled) {
    return null;
  }

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
