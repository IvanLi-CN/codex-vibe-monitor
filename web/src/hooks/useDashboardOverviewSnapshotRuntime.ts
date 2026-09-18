import { useEffect, useMemo, useRef, useState } from "react";
import {
  createDashboardOverviewSnapshotEntry,
  type DashboardOverviewSnapshotBundle,
  type DashboardOverviewSnapshotMode,
  type DashboardOverviewSnapshotRange,
  type DashboardOverviewSnapshotStatus,
  fetchDashboardOverviewSnapshotBundle,
  getDashboardOverviewSnapshotPrefetchOrder,
  isDashboardOverviewSnapshotNetworkError,
  listDashboardOverviewSnapshotRanges,
  readDashboardOverviewSnapshotEntry,
  sortDashboardOverviewSnapshotRanges,
  writeDashboardOverviewSnapshotEntry,
} from "../features/dashboard/dashboardOverviewSnapshots";

export interface DashboardOverviewSnapshotRuntime {
  status: DashboardOverviewSnapshotStatus;
  bundle: DashboardOverviewSnapshotBundle | null;
}

function getInitialOnlineState() {
  if (typeof navigator === "undefined") return true;
  return navigator.onLine;
}

function mergeReadyRanges(
  current: DashboardOverviewSnapshotRange[],
  range: DashboardOverviewSnapshotRange,
) {
  return sortDashboardOverviewSnapshotRanges([...current, range]);
}

async function readDashboardOverviewSnapshotState(activeRange: DashboardOverviewSnapshotRange) {
  const [readyRanges, entry] = await Promise.all([
    listDashboardOverviewSnapshotRanges(),
    readDashboardOverviewSnapshotEntry(activeRange),
  ]);
  return { readyRanges, entry };
}

async function prefetchDashboardOverviewSnapshots({
  activeRange,
  controller,
  forcePrefetchAll,
  isCurrent,
  readyRanges,
  onReadyRange,
}: {
  activeRange: DashboardOverviewSnapshotRange;
  controller: AbortController;
  forcePrefetchAll: boolean;
  isCurrent: () => boolean;
  readyRanges: DashboardOverviewSnapshotRange[];
  onReadyRange: (range: DashboardOverviewSnapshotRange) => void;
}) {
  const targets = getDashboardOverviewSnapshotPrefetchOrder(activeRange).filter(
    (range) => range !== activeRange && (forcePrefetchAll || !readyRanges.includes(range)),
  );
  for (const range of targets) {
    if (controller.signal.aborted || !isCurrent()) return;
    try {
      const bundle = await fetchDashboardOverviewSnapshotBundle(range, {
        signal: controller.signal,
      });
      if (!isCurrent()) return;
      const entry = createDashboardOverviewSnapshotEntry(range, bundle);
      await writeDashboardOverviewSnapshotEntry(entry);
      if (!isCurrent()) return;
      onReadyRange(range);
    } catch (error) {
      if (controller.signal.aborted) return;
      if (!isDashboardOverviewSnapshotNetworkError(error)) continue;
      return;
    }
  }
}

function useDashboardOverviewOnlineState(forcePrefetchAllRef: { current: boolean }) {
  const [isOnline, setIsOnline] = useState(getInitialOnlineState);
  useEffect(() => {
    if (typeof window === "undefined") return undefined;
    const handleOnline = () => {
      forcePrefetchAllRef.current = true;
      setIsOnline(true);
    };
    const handleOffline = () => setIsOnline(false);
    window.addEventListener("online", handleOnline);
    window.addEventListener("offline", handleOffline);
    return () => {
      window.removeEventListener("online", handleOnline);
      window.removeEventListener("offline", handleOffline);
    };
  }, [forcePrefetchAllRef]);
  return isOnline;
}

function buildDashboardOverviewSnapshotRuntime({
  bundle,
  cachedAt,
  mode,
  readyRanges,
}: {
  bundle: DashboardOverviewSnapshotBundle | null;
  cachedAt: string | null;
  mode: DashboardOverviewSnapshotMode;
  readyRanges: DashboardOverviewSnapshotRange[];
}): DashboardOverviewSnapshotRuntime {
  return {
    status: { mode, cachedAt, readyRanges },
    bundle: mode === "not-cached-yet" ? null : bundle,
  };
}

export function useDashboardOverviewSnapshotRuntime(
  activeRange: DashboardOverviewSnapshotRange,
): DashboardOverviewSnapshotRuntime {
  const [mode, setMode] = useState<DashboardOverviewSnapshotMode>(() =>
    getInitialOnlineState() ? "live" : "not-cached-yet",
  );
  const [readyRanges, setReadyRanges] = useState<DashboardOverviewSnapshotRange[]>([]);
  const [bundle, setBundle] = useState<DashboardOverviewSnapshotBundle | null>(null);
  const [cachedAt, setCachedAt] = useState<string | null>(null);
  const requestSeqRef = useRef(0);
  const forcePrefetchAllRef = useRef(true);
  const isOnline = useDashboardOverviewOnlineState(forcePrefetchAllRef);

  useEffect(() => {
    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    const controller = new AbortController();
    let disposed = false;

    const applyCachedState = async () => {
      const { readyRanges: nextReadyRanges, entry } =
        await readDashboardOverviewSnapshotState(activeRange);
      if (disposed || requestSeq !== requestSeqRef.current) return;
      setReadyRanges(nextReadyRanges);
      setBundle(entry?.payload ?? null);
      setCachedAt(entry?.cachedAt ?? null);
      setMode(entry ? "cached-offline" : "not-cached-yet");
    };

    const refreshSnapshots = async () => {
      const { readyRanges: nextReadyRanges, entry: cachedEntry } =
        await readDashboardOverviewSnapshotState(activeRange);
      if (disposed || requestSeq !== requestSeqRef.current) return;

      setReadyRanges(nextReadyRanges);
      setBundle(cachedEntry?.payload ?? null);
      setCachedAt(cachedEntry?.cachedAt ?? null);

      if (!isOnline) {
        setMode(cachedEntry ? "cached-offline" : "not-cached-yet");
        return;
      }

      const forcePrefetchAll = forcePrefetchAllRef.current;
      forcePrefetchAllRef.current = false;

      try {
        const activeBundle = await fetchDashboardOverviewSnapshotBundle(activeRange, {
          signal: controller.signal,
        });
        if (disposed || requestSeq !== requestSeqRef.current) return;
        const activeEntry = createDashboardOverviewSnapshotEntry(activeRange, activeBundle);
        await writeDashboardOverviewSnapshotEntry(activeEntry);
        if (disposed || requestSeq !== requestSeqRef.current) return;

        setMode("live");
        setBundle(activeEntry.payload);
        setCachedAt(activeEntry.cachedAt);
        setReadyRanges((current) => mergeReadyRanges(current, activeRange));
      } catch (error) {
        if (controller.signal.aborted || disposed || requestSeq !== requestSeqRef.current) return;
        if (isDashboardOverviewSnapshotNetworkError(error) && cachedEntry) {
          setMode("cached-offline");
          setBundle(cachedEntry.payload);
          setCachedAt(cachedEntry.cachedAt);
        } else if (isDashboardOverviewSnapshotNetworkError(error) && !navigator.onLine) {
          setMode("not-cached-yet");
          setBundle(null);
          setCachedAt(null);
        } else {
          setMode("live");
        }
        return;
      }

      await prefetchDashboardOverviewSnapshots({
        activeRange,
        controller,
        forcePrefetchAll,
        isCurrent: () => !disposed && requestSeq === requestSeqRef.current,
        readyRanges: nextReadyRanges,
        onReadyRange: (range) => setReadyRanges((current) => mergeReadyRanges(current, range)),
      });
    };

    void (isOnline ? refreshSnapshots() : applyCachedState());

    return () => {
      disposed = true;
      controller.abort();
    };
  }, [activeRange, isOnline]);

  return useMemo(
    () => buildDashboardOverviewSnapshotRuntime({ bundle, cachedAt, mode, readyRanges }),
    [bundle, cachedAt, mode, readyRanges],
  );
}

export default useDashboardOverviewSnapshotRuntime;
