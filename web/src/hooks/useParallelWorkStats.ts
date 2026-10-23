import { useCallback, useEffect, useRef, useState } from "react";
import {
  ApiRequestError,
  fetchParallelWorkStats,
  type ParallelWorkStatsResponse,
} from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { getBrowserTimeZone } from "../lib/timeZone";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

interface UseParallelWorkStatsOptions {
  range: string;
  bucket?: string;
  upstreamAccountId?: number;
  enabled?: boolean;
}

export const PARALLEL_WORK_REFRESH_THROTTLE_MS = 5_000;
export const PARALLEL_WORK_OPEN_RESYNC_COOLDOWN_MS = 5_000;

export function shouldRetryParallelWorkError(error: unknown) {
  if (!error) return false;
  if (error instanceof ApiRequestError) {
    return error.status === 429 || error.status >= 500;
  }
  if (error instanceof Error && error.name === "AbortError") {
    return false;
  }
  return true;
}

export function getParallelWorkRecordsResyncDelay(
  lastRefreshAt: number,
  now: number,
  throttleMs = PARALLEL_WORK_REFRESH_THROTTLE_MS,
) {
  return Math.max(0, throttleMs - (now - lastRefreshAt));
}

export function shouldTriggerParallelWorkOpenResync(
  lastResyncAt: number,
  now: number,
  force = false,
) {
  if (force) return true;
  return now - lastResyncAt >= PARALLEL_WORK_OPEN_RESYNC_COOLDOWN_MS;
}

export function useParallelWorkStats({
  range,
  bucket,
  upstreamAccountId,
  enabled = true,
}: UseParallelWorkStatsOptions) {
  const useHttp = enabled && (range === "previous1d" || range === "previous24h");
  const topic =
    enabled && !useHttp
      ? buildTopicDescriptor("stats.parallel-work.current", {
          range,
          bucket,
          upstreamAccountId,
          timeZone: getBrowserTimeZone(),
        })
      : null;
  const { data, isLoading, error, refresh } = useSubscriptionTopic<ParallelWorkStatsResponse>(
    topic,
    enabled && !useHttp,
  );
  const [httpData, setHttpData] = useState<ParallelWorkStatsResponse | null>(null);
  const [httpLoading, setHttpLoading] = useState(useHttp);
  const [httpError, setHttpError] = useState<string | null>(null);
  const httpRequestSeqRef = useRef(0);
  const loadHttp = useCallback(async () => {
    const requestSeq = httpRequestSeqRef.current + 1;
    httpRequestSeqRef.current = requestSeq;
    if (!useHttp) {
      setHttpLoading(false);
      return;
    }
    setHttpLoading(true);
    try {
      const response = await fetchParallelWorkStats({
        range,
        bucket,
        upstreamAccountId,
        timeZone: getBrowserTimeZone(),
      });
      if (requestSeq !== httpRequestSeqRef.current) return;
      setHttpData(response);
      setHttpError(null);
    } catch (error) {
      if (requestSeq !== httpRequestSeqRef.current) return;
      setHttpError(error instanceof Error ? error.message : String(error));
    } finally {
      if (requestSeq === httpRequestSeqRef.current) {
        setHttpLoading(false);
      }
    }
  }, [bucket, range, upstreamAccountId, useHttp]);
  useEffect(() => {
    void loadHttp();
  }, [loadHttp]);

  return {
    data: useHttp ? httpData : data,
    isLoading: useHttp ? httpLoading : isLoading,
    error: useHttp ? httpError : error,
    refresh: useHttp ? loadHttp : refresh,
  };
}
