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
const diagnosticsEnabledListeners = new Set<() => void>();
const DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_EVENT =
  "dashboard-performance-diagnostics-storage-change";

let lastTodayChartRenderSignature: string | null = null;
let restoreStorageBridge: (() => void) | null = null;
let storageBridgeRefCount = 0;
let stopDiagnosticsEnabledSync: (() => void) | null = null;

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

export function isDashboardPerformanceDiagnosticsEnabled() {
  return readDiagnosticsEnabled();
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

function ensureDashboardDiagnosticsStorageBridge() {
  if (typeof window === "undefined") {
    return () => {};
  }
  storageBridgeRefCount += 1;
  if (restoreStorageBridge) {
    return () => {
      storageBridgeRefCount = Math.max(0, storageBridgeRefCount - 1);
      if (storageBridgeRefCount === 0) {
        restoreStorageBridge?.();
      }
    };
  }

  const storage = window.localStorage as Storage & {
    setItem: Storage["setItem"];
    removeItem: Storage["removeItem"];
    clear: Storage["clear"];
  };
  const storageTarget =
    Object.prototype.hasOwnProperty.call(storage, "setItem") ||
    Object.prototype.hasOwnProperty.call(storage, "removeItem") ||
    Object.prototype.hasOwnProperty.call(storage, "clear")
      ? storage
      : ((Object.getPrototypeOf(storage) as
          | typeof storage
          | null) ?? storage);
  const originalSetItem = storageTarget.setItem;
  const originalRemoveItem = storageTarget.removeItem;
  const originalClear = storageTarget.clear;

  const dispatchStorageEvent = (key: string | null) => {
    window.dispatchEvent(
      new CustomEvent(DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_EVENT, {
        detail: { key },
      }),
    );
  };

  storageTarget.setItem = function patchedSetItem(
    key: string,
    value: string,
  ) {
    originalSetItem.call(this, key, value);
    dispatchStorageEvent(key);
  };
  storageTarget.removeItem = function patchedRemoveItem(key: string) {
    originalRemoveItem.call(this, key);
    dispatchStorageEvent(key);
  };
  storageTarget.clear = function patchedClear() {
    originalClear.call(this);
    dispatchStorageEvent(null);
  };

  restoreStorageBridge = () => {
    storageTarget.setItem = originalSetItem;
    storageTarget.removeItem = originalRemoveItem;
    storageTarget.clear = originalClear;
    restoreStorageBridge = null;
  };

  return () => {
    storageBridgeRefCount = Math.max(0, storageBridgeRefCount - 1);
    if (storageBridgeRefCount === 0) {
      restoreStorageBridge?.();
    }
  };
}

function emitDiagnosticsEnabledListeners() {
  diagnosticsEnabledListeners.forEach((listener) => listener());
}

function ensureDashboardDiagnosticsEnabledSync() {
  if (typeof window === "undefined" || stopDiagnosticsEnabledSync) {
    return;
  }

  const sync = () => {
    syncDashboardPerformanceDiagnosticsEnabled();
    emitDiagnosticsEnabledListeners();
  };
  const releaseStorageBridge = ensureDashboardDiagnosticsStorageBridge();
  const onStorage = (event: StorageEvent) => {
    if (
      event.key !== null &&
      event.key !== DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY
    ) {
      return;
    }
    sync();
  };
  const onSameTabStorageChange = (event: Event) => {
    const customEvent = event as CustomEvent<{ key: string | null }>;
    if (
      customEvent.detail?.key !== null &&
      customEvent.detail?.key !== DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY
    ) {
      return;
    }
    sync();
  };

  window.addEventListener("storage", onStorage);
  window.addEventListener(
    DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_EVENT,
    onSameTabStorageChange,
  );
  sync();

  stopDiagnosticsEnabledSync = () => {
    window.removeEventListener("storage", onStorage);
    window.removeEventListener(
      DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_EVENT,
      onSameTabStorageChange,
    );
    releaseStorageBridge();
    stopDiagnosticsEnabledSync = null;
  };
}

function maybeStopDashboardDiagnosticsEnabledSync() {
  if (diagnosticsEnabledListeners.size === 0) {
    stopDiagnosticsEnabledSync?.();
  }
}

export function useDashboardPerformanceDiagnosticsEnabled() {
  return useSyncExternalStore(
    (listener) => {
      diagnosticsEnabledListeners.add(listener);
      ensureDashboardDiagnosticsEnabledSync();
      return () => {
        diagnosticsEnabledListeners.delete(listener);
        maybeStopDashboardDiagnosticsEnabledSync();
      };
    },
    isDashboardPerformanceDiagnosticsEnabled,
    () => false,
  );
}

export function useDashboardPerformanceDiagnosticsSnapshot() {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getDashboardPerformanceDiagnosticsSnapshot,
    getDashboardPerformanceDiagnosticsSnapshot,
  );
}
