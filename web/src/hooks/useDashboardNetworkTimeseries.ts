import { useEffect, useRef, useState } from "react";
import {
  type DashboardNetworkTimeseriesResponse,
  fetchDashboardNetworkTimeseries,
} from "../lib/api";

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
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
  const refreshTimerRef = useRef<number | null>(null);
  const dataRef = useRef<DashboardNetworkTimeseriesResponse | null>(null);

  useEffect(() => {
    dataRef.current = data;
  }, [data]);

  useEffect(() => {
    if (!enabled) {
      abortControllerRef.current?.abort();
      abortControllerRef.current = null;
      if (refreshTimerRef.current != null) {
        window.clearInterval(refreshTimerRef.current);
        refreshTimerRef.current = null;
      }
      return;
    }

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
        setData(nextData);
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

    void load(false);
    if (range !== "yesterday") {
      refreshTimerRef.current = window.setInterval(() => {
        void load(true);
      }, 1_000);
    }

    return () => {
      disposed = true;
      abortControllerRef.current?.abort();
      abortControllerRef.current = null;
      if (refreshTimerRef.current != null) {
        window.clearInterval(refreshTimerRef.current);
        refreshTimerRef.current = null;
      }
    };
  }, [enabled, range, upstreamAccountId]);

  return {
    data,
    isLoading,
    isRefreshing,
    error,
    reload: async () => {
      const nextData = await fetchDashboardNetworkTimeseries(range, { upstreamAccountId });
      setData(nextData);
    },
  };
}
