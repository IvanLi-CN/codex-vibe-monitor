import { useCallback, useSyncExternalStore } from "react";

type DashboardLiveClockListener = () => void;

let snapshot = Date.now();
let timer: number | null = null;
const listeners = new Set<DashboardLiveClockListener>();

function isDocumentVisible() {
  return typeof document === "undefined" || document.visibilityState !== "hidden";
}

function refreshSnapshot() {
  snapshot = Date.now();
}

function publishSnapshot() {
  refreshSnapshot();
  for (const listener of listeners) {
    listener();
  }
}

function stopTimer() {
  if (timer == null || typeof window === "undefined") return;
  window.clearInterval(timer);
  timer = null;
}

function startTimer() {
  if (timer != null || typeof window === "undefined" || !isDocumentVisible()) return;
  timer = window.setInterval(publishSnapshot, 1000);
}

function handleVisibilityChange() {
  if (!isDocumentVisible()) {
    stopTimer();
    return;
  }

  publishSnapshot();
  startTimer();
}

function subscribe(listener: DashboardLiveClockListener) {
  listeners.add(listener);
  if (listeners.size === 1 && typeof document !== "undefined") {
    document.addEventListener("visibilitychange", handleVisibilityChange);
  }
  refreshSnapshot();
  startTimer();

  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      stopTimer();
      if (typeof document !== "undefined") {
        document.removeEventListener("visibilitychange", handleVisibilityChange);
      }
    }
  };
}

function getSnapshot() {
  if (listeners.size === 0 && timer == null) {
    refreshSnapshot();
  }
  return snapshot;
}

export function useDashboardLiveClock(enabled: boolean) {
  const subscribeWhenEnabled = useCallback(
    (listener: DashboardLiveClockListener) => (enabled ? subscribe(listener) : () => undefined),
    [enabled],
  );
  const getSnapshotWhenEnabled = useCallback(() => (enabled ? getSnapshot() : 0), [enabled]);

  return useSyncExternalStore(subscribeWhenEnabled, getSnapshotWhenEnabled, getSnapshotWhenEnabled);
}
