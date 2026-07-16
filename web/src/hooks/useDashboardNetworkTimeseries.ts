import { useEffect, useRef, useState } from "react";
import {
  type DashboardActivityLiveSnapshot,
  type DashboardNetworkTimeseriesPoint,
  type DashboardNetworkTimeseriesResponse,
  fetchDashboardNetworkTimeseries,
} from "../lib/api";
import { subscribeToSse, subscribeToSseOpen } from "../lib/sse";

const DASHBOARD_NETWORK_OPEN_RESYNC_COOLDOWN_MS = 5_000;

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

function resolveDashboardNetworkLiveBucket(
  snapshot: DashboardActivityLiveSnapshot,
  upstreamAccountId?: number,
): DashboardNetworkTimeseriesPoint | null {
  if (upstreamAccountId == null) {
    return snapshot.networkLiveBucket ?? null;
  }
  return (
    snapshot.accounts.find((account) => account.upstreamAccountId === upstreamAccountId)
      ?.networkLiveBucket ?? null
  );
}

function mergeDashboardNetworkLiveSnapshot(
  response: DashboardNetworkTimeseriesResponse,
  snapshot: DashboardActivityLiveSnapshot,
  upstreamAccountId?: number,
): DashboardNetworkTimeseriesResponse | null {
  const liveBucket = resolveDashboardNetworkLiveBucket(snapshot, upstreamAccountId);
  if (!liveBucket) {
    return response;
  }

  const pointIndex = response.points.findIndex(
    (point) =>
      point.bucketStart === liveBucket.bucketStart && point.bucketEnd === liveBucket.bucketEnd,
  );
  if (pointIndex === -1) {
    return null;
  }

  const nextPoints = response.points.slice();
  nextPoints[pointIndex] = liveBucket;
  return {
    ...response,
    snapshotId: Math.max(
      response.snapshotId,
      Number.isFinite(Date.parse(snapshot.generatedAt))
        ? Date.parse(snapshot.generatedAt)
        : response.snapshotId,
    ),
    points: nextPoints,
  };
}

export function useDashboardNetworkTimeseries(
  range: "today" | "yesterday" | "1d",
  enabled: boolean,
  upstreamAccountId?: number,
) {
  const [data, setData] = useState<DashboardNetworkTimeseriesResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestSeqRef = useRef(0);
  const abortControllerRef = useRef<AbortController | null>(null);
  const dataRef = useRef<DashboardNetworkTimeseriesResponse | null>(null);
  const rangeRef = useRef(range);
  const upstreamAccountIdRef = useRef(upstreamAccountId);
  const latestLiveSnapshotRef = useRef<DashboardActivityLiveSnapshot | null>(null);
  const hasHydratedRef = useRef(false);
  const lastOpenResyncAtRef = useRef(0);
  const loadRef = useRef<(silent: boolean) => Promise<void>>(async () => {});

  useEffect(() => {
    dataRef.current = data;
  }, [data]);

  useEffect(() => {
    rangeRef.current = range;
  }, [range]);

  useEffect(() => {
    upstreamAccountIdRef.current = upstreamAccountId;
  }, [upstreamAccountId]);

  useEffect(() => {
    let disposed = false;

    const load = async (silent: boolean) => {
      abortControllerRef.current?.abort();
      const controller = new AbortController();
      abortControllerRef.current = controller;
      const requestSeq = requestSeqRef.current + 1;
      requestSeqRef.current = requestSeq;
      if (silent && dataRef.current != null) {
        setIsRefreshing(true);
      } else {
        setIsLoading(true);
      }

      try {
        const nextData = await fetchDashboardNetworkTimeseries(range, {
          upstreamAccountId,
          signal: controller.signal,
        });
        if (disposed || requestSeq !== requestSeqRef.current) return;
        const mergedData =
          range === "yesterday" || latestLiveSnapshotRef.current == null
            ? nextData
            : (mergeDashboardNetworkLiveSnapshot(
                nextData,
                latestLiveSnapshotRef.current,
                upstreamAccountId,
              ) ?? nextData);
        hasHydratedRef.current = true;
        setData(mergedData);
        setError(null);
      } catch (err) {
        if (disposed || isAbortError(err) || requestSeq !== requestSeqRef.current) return;
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (!disposed && requestSeq === requestSeqRef.current) {
          setIsLoading(false);
          setIsRefreshing(false);
          if (abortControllerRef.current === controller) {
            abortControllerRef.current = null;
          }
        }
      }
    };

    loadRef.current = load;
    if (!enabled) {
      abortControllerRef.current?.abort();
      abortControllerRef.current = null;
      setIsLoading(false);
      setIsRefreshing(false);
      return () => {
        disposed = true;
      };
    }

    hasHydratedRef.current = false;
    void load(false);

    return () => {
      disposed = true;
      abortControllerRef.current?.abort();
      abortControllerRef.current = null;
      loadRef.current = async () => {};
    };
  }, [enabled, range, upstreamAccountId]);

  useEffect(() => {
    if (!enabled) return;
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type === "version") {
        latestLiveSnapshotRef.current = null;
        return;
      }
      if (payload.type !== "dashboardActivityLive") return;
      const current = latestLiveSnapshotRef.current;
      if (current && payload.snapshot.revision <= current.revision) return;
      latestLiveSnapshotRef.current = payload.snapshot;
      if (rangeRef.current === "yesterday") return;
      setData((currentData) => {
        if (!currentData || currentData.range !== rangeRef.current) {
          return currentData;
        }
        const merged = mergeDashboardNetworkLiveSnapshot(
          currentData,
          payload.snapshot,
          upstreamAccountIdRef.current,
        );
        if (merged == null) {
          void loadRef.current(true);
          return currentData;
        }
        return merged;
      });
    });
    return unsubscribe;
  }, [enabled]);

  useEffect(() => {
    if (!enabled) return;
    const unsubscribe = subscribeToSseOpen(() => {
      if (!hasHydratedRef.current) return;
      const now = Date.now();
      if (now - lastOpenResyncAtRef.current < DASHBOARD_NETWORK_OPEN_RESYNC_COOLDOWN_MS) {
        return;
      }
      lastOpenResyncAtRef.current = now;
      void loadRef.current(true);
    });
    return unsubscribe;
  }, [enabled]);

  return {
    data,
    isLoading,
    isRefreshing,
    error,
    reload: async () => {
      await loadRef.current(false);
    },
  };
}
