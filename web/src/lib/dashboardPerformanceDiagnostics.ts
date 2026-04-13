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

interface DashboardPerformanceDiagnosticsMetrics {
  workingConversationPatchBucketCount: number;
  workingConversationPatchEntryCount: number;
  workingConversationPatchLastUpdatedAt: string | null;
  todaySummaryRefreshCount: number;
  todaySummaryLastUpdatedAt: string | null;
  todayChartRenderCount: number;
  todayChartLastRenderedAt: string | null;
}

declare global {
  interface Window {
    __dashboardPerformanceDiagnostics__?: DashboardPerformanceDiagnosticsSnapshot;
  }
}

const listeners = new Set<() => void>();
const DIAGNOSTICS_ENABLE_SYNC_INTERVAL_MS = 1_000;

let lastTodayChartRenderSignature: string | null = null;
let enableSyncCleanup: (() => void) | null = null;

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

function createEmptyMetrics(): DashboardPerformanceDiagnosticsMetrics {
  return {
    workingConversationPatchBucketCount: 0,
    workingConversationPatchEntryCount: 0,
    workingConversationPatchLastUpdatedAt: null,
    todaySummaryRefreshCount: 0,
    todaySummaryLastUpdatedAt: null,
    todayChartRenderCount: 0,
    todayChartLastRenderedAt: null,
  };
}

function createSnapshot(
  enabled = readDiagnosticsEnabled(),
): DashboardPerformanceDiagnosticsSnapshot {
  return {
    enabled,
    ...metrics,
  };
}

let metrics = createEmptyMetrics();
let snapshot = createSnapshot();

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
  snapshot = createSnapshot(enabled);
  emitDiagnosticsSnapshot();
  return snapshot;
}

function updateMetricsSnapshot(
  updater: (
    current: DashboardPerformanceDiagnosticsMetrics,
    timestamp: string,
  ) => DashboardPerformanceDiagnosticsMetrics,
) {
  metrics = updater(metrics, new Date(Date.now()).toISOString());
  snapshot = createSnapshot();
  if (!snapshot.enabled) {
    syncWindowSnapshot();
    return;
  }
  emitDiagnosticsSnapshot();
}

export function resetDashboardPerformanceDiagnostics() {
  lastTodayChartRenderSignature = null;
  metrics = createEmptyMetrics();
  snapshot = createSnapshot();
  emitDiagnosticsSnapshot();
}

export function getDashboardPerformanceDiagnosticsSnapshot() {
  return snapshot;
}

export function publishWorkingConversationPatchMetrics(
  patchMetrics: DashboardPatchMetrics,
) {
  updateMetricsSnapshot((current, timestamp) => {
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
  updateMetricsSnapshot((current, timestamp) => ({
    ...current,
    todaySummaryRefreshCount: current.todaySummaryRefreshCount + 1,
    todaySummaryLastUpdatedAt: timestamp,
  }));
}

export function recordTodayChartRender(signature?: string | null) {
  updateMetricsSnapshot((current, timestamp) => {
    if (signature != null && signature === lastTodayChartRenderSignature) {
      return current;
    }
    lastTodayChartRenderSignature = signature ?? null;
    return {
      ...current,
      todayChartRenderCount: current.todayChartRenderCount + 1,
      todayChartLastRenderedAt: timestamp,
    };
  });
}

function ensureDiagnosticsEnableSync() {
  if (typeof window === "undefined" || enableSyncCleanup) {
    return;
  }

  const sync = () => {
    syncDashboardPerformanceDiagnosticsEnabled();
  };
  const intervalHandle = window.setInterval(
    sync,
    DIAGNOSTICS_ENABLE_SYNC_INTERVAL_MS,
  );
  const onStorage = (event: StorageEvent) => {
    if (
      event.key !== null &&
      event.key !== DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY
    ) {
      return;
    }
    sync();
  };
  const onFocus = () => {
    sync();
  };

  window.addEventListener("storage", onStorage);
  window.addEventListener("focus", onFocus);
  if (typeof document !== "undefined") {
    document.addEventListener("visibilitychange", onFocus);
  }

  enableSyncCleanup = () => {
    window.clearInterval(intervalHandle);
    window.removeEventListener("storage", onStorage);
    window.removeEventListener("focus", onFocus);
    if (typeof document !== "undefined") {
      document.removeEventListener("visibilitychange", onFocus);
    }
    enableSyncCleanup = null;
  };
}

function maybeStopDiagnosticsEnableSync() {
  if (listeners.size === 0) {
    enableSyncCleanup?.();
  }
}

export function useDashboardPerformanceDiagnosticsSnapshot() {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      ensureDiagnosticsEnableSync();
      return () => {
        listeners.delete(listener);
        maybeStopDiagnosticsEnableSync();
      };
    },
    getDashboardPerformanceDiagnosticsSnapshot,
    getDashboardPerformanceDiagnosticsSnapshot,
  );
}
