import { useSyncExternalStore } from "react";

export const DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY =
  "dashboard.performanceDiagnostics.enabled.v1";

export interface DashboardPerformanceDiagnosticsSnapshot {
  enabled: boolean;
  workingConversationPatchBucketCount: number;
  workingConversationPatchEntryCount: number;
  workingConversationPatchLastUpdatedAt: string | null;
  todaySummaryRefreshCount: number;
  todaySummaryLastUpdatedAt: string | null;
  todayChartRenderCount: number;
  todayChartLastRenderedAt: string | null;
}

type DashboardPatchMetrics = Map<
  string,
  Map<string, { totalTokens: number; cost: number }>
>;

declare global {
  interface Window {
    __dashboardPerformanceDiagnostics__?: DashboardPerformanceDiagnosticsSnapshot;
  }
}

const listeners = new Set<() => void>();

function readDiagnosticsEnabled() {
  if (typeof window === "undefined") return false;
  try {
    const rawValue = window.localStorage.getItem(
      DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY,
    );
    return rawValue === "1" || rawValue === "true";
  } catch {
    return false;
  }
}

function createEmptySnapshot(
  enabled = readDiagnosticsEnabled(),
): DashboardPerformanceDiagnosticsSnapshot {
  return {
    enabled,
    workingConversationPatchBucketCount: 0,
    workingConversationPatchEntryCount: 0,
    workingConversationPatchLastUpdatedAt: null,
    todaySummaryRefreshCount: 0,
    todaySummaryLastUpdatedAt: null,
    todayChartRenderCount: 0,
    todayChartLastRenderedAt: null,
  };
}

let snapshot = createEmptySnapshot();

function syncWindowSnapshot() {
  if (typeof window === "undefined") return;
  window.__dashboardPerformanceDiagnostics__ = snapshot;
}

function emitDiagnosticsSnapshot() {
  syncWindowSnapshot();
  listeners.forEach((listener) => listener());
}

export function syncDashboardPerformanceDiagnosticsEnabled() {
  const enabled = readDiagnosticsEnabled();
  if (snapshot.enabled === enabled) {
    return snapshot;
  }
  snapshot = enabled
    ? { ...createEmptySnapshot(true), ...snapshot, enabled: true }
    : createEmptySnapshot(false);
  emitDiagnosticsSnapshot();
  return snapshot;
}

function updateSnapshotWhenEnabled(
  updater: (
    current: DashboardPerformanceDiagnosticsSnapshot,
    timestamp: string,
  ) => DashboardPerformanceDiagnosticsSnapshot,
) {
  const current = syncDashboardPerformanceDiagnosticsEnabled();
  if (!current.enabled) {
    syncWindowSnapshot();
    return;
  }
  snapshot = updater(current, new Date(Date.now()).toISOString());
  emitDiagnosticsSnapshot();
}

export function resetDashboardPerformanceDiagnostics() {
  snapshot = createEmptySnapshot();
  emitDiagnosticsSnapshot();
}

export function getDashboardPerformanceDiagnosticsSnapshot() {
  return snapshot;
}

export function publishWorkingConversationPatchMetrics(
  patchMetrics: DashboardPatchMetrics,
) {
  updateSnapshotWhenEnabled((current, timestamp) => {
    let patchEntryCount = 0;
    patchMetrics.forEach((invocations) => {
      patchEntryCount += invocations.size;
    });
    return {
      ...current,
      workingConversationPatchBucketCount: patchMetrics.size,
      workingConversationPatchEntryCount: patchEntryCount,
      workingConversationPatchLastUpdatedAt: timestamp,
    };
  });
}

export function recordTodaySummaryRefresh(window: string) {
  if (window !== "today") return;
  updateSnapshotWhenEnabled((current, timestamp) => ({
    ...current,
    todaySummaryRefreshCount: current.todaySummaryRefreshCount + 1,
    todaySummaryLastUpdatedAt: timestamp,
  }));
}

export function recordTodayChartRender() {
  updateSnapshotWhenEnabled((current, timestamp) => ({
    ...current,
    todayChartRenderCount: current.todayChartRenderCount + 1,
    todayChartLastRenderedAt: timestamp,
  }));
}

export function useDashboardPerformanceDiagnosticsSnapshot() {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    getDashboardPerformanceDiagnosticsSnapshot,
    getDashboardPerformanceDiagnosticsSnapshot,
  );
}
