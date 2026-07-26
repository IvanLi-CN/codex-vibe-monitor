import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  LongTermStatsDimension,
  LongTermStatsOverviewResponse,
  LongTermStatsRange,
  LongTermStatsSeriesResponse,
} from "../lib/api";
import { fetchLongTermStatsOverview, fetchLongTermStatsSeries } from "../lib/api";

export const LONG_TERM_STATS_REFRESH_INTERVAL_MS = 60_000;

export interface UseLongTermStatsResult {
  overview: LongTermStatsOverviewResponse | null;
  series: LongTermStatsSeriesResponse | null;
  isLoading: boolean;
  isSeriesLoading: boolean;
  error: string | null;
  seriesError: string | null;
  refresh: () => Promise<void>;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function useLongTermStats(
  range: LongTermStatsRange,
  dimension: LongTermStatsDimension,
  selectedKeys: string[],
  enabled = true,
  sharedOverview?: LongTermStatsOverviewResponse | null,
): UseLongTermStatsResult {
  const shouldFetch = enabled && import.meta.env.MODE !== "storybook";
  const [overview, setOverview] = useState<LongTermStatsOverviewResponse | null>(null);
  const [series, setSeries] = useState<LongTermStatsSeriesResponse | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isSeriesLoading, setIsSeriesLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [seriesError, setSeriesError] = useState<string | null>(null);
  const overviewRequestId = useRef(0);
  const previousRange = useRef(range);
  const stableKeys = useMemo(() => selectedKeys.filter(Boolean).slice(0, 8), [selectedKeys]);

  const refresh = useCallback(async () => {
    const requestId = ++overviewRequestId.current;
    try {
      setError(null);
      const next = await fetchLongTermStatsOverview(range);
      if (requestId === overviewRequestId.current) setOverview(next);
    } catch (cause) {
      if (requestId === overviewRequestId.current) setError(errorMessage(cause));
    } finally {
      if (requestId === overviewRequestId.current) setIsLoading(false);
    }
  }, [range]);

  useEffect(() => {
    if (previousRange.current === range) return;
    previousRange.current = range;
    if (sharedOverview === undefined) {
      setOverview(null);
      setIsLoading(true);
      setError(null);
    }
    setSeries(null);
    setSeriesError(null);
  }, [range, sharedOverview]);

  useEffect(() => {
    if (sharedOverview !== undefined) {
      setOverview(sharedOverview);
      setIsLoading(false);
      return;
    }
    if (!shouldFetch) {
      setIsLoading(false);
      return;
    }
    let active = true;
    const load = async () => {
      if (!active || (typeof document !== "undefined" && document.hidden)) return;
      await refresh();
    };
    void load();
    const timer = window.setInterval(load, LONG_TERM_STATS_REFRESH_INTERVAL_MS);
    document.addEventListener("visibilitychange", load);
    return () => {
      active = false;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", load);
    };
  }, [refresh, sharedOverview, shouldFetch]);

  useEffect(() => {
    let active = true;
    if (!shouldFetch || stableKeys.length === 0 || overview?.status !== "ready") {
      setSeries(null);
      setIsSeriesLoading(false);
      setSeriesError(null);
      return () => {
        active = false;
      };
    }
    setIsSeriesLoading(true);
    setSeries(null);
    setSeriesError(null);
    fetchLongTermStatsSeries(range, dimension, stableKeys)
      .then((next) => {
        if (active) setSeries(next);
      })
      .catch((cause) => {
        if (active) setSeriesError(errorMessage(cause));
      })
      .finally(() => {
        if (active) setIsSeriesLoading(false);
      });
    return () => {
      active = false;
    };
  }, [dimension, overview, range, shouldFetch, stableKeys]);

  return { overview, series, isLoading, isSeriesLoading, error, seriesError, refresh };
}
