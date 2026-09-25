import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { InvocationTimelineResponse, TimeseriesResponse } from "../lib/api";
import { fetchInvocationTimeline } from "../lib/api";

const DEFAULT_WINDOW_MS = 30 * 60 * 1_000;
const LIVE_REFRESH_MS = 15_000;

export interface InvocationTimelineWindow {
  startMs: number;
  endMs: number;
}

interface UseInvocationTimelineOptions {
  response: TimeseriesResponse | null;
  closedNaturalDay: boolean;
  upstreamAccountId?: number;
  liveRevision?: number;
  liveConnected?: boolean;
}

function parseEpoch(value: string | null | undefined) {
  if (!value) return null;
  const epoch = Date.parse(value);
  return Number.isFinite(epoch) ? epoch : null;
}

function clampWindow(window: InvocationTimelineWindow, bounds: InvocationTimelineWindow) {
  const boundSpan = Math.max(1, bounds.endMs - bounds.startMs);
  const span = Math.min(boundSpan, Math.max(60_000, window.endMs - window.startMs));
  const endMs = Math.min(bounds.endMs, Math.max(bounds.startMs + span, window.endMs));
  return {
    startMs: Math.max(bounds.startMs, endMs - span),
    endMs,
  };
}

function resolveInitialWindow(
  response: TimeseriesResponse | null,
  closedNaturalDay: boolean,
): InvocationTimelineWindow | null {
  const bounds = {
    startMs: parseEpoch(response?.rangeStart) ?? Date.now() - 24 * 60 * 60 * 1_000,
    endMs: parseEpoch(response?.rangeEnd) ?? Date.now(),
  };
  if (bounds.endMs <= bounds.startMs) return null;
  if (!closedNaturalDay) {
    return clampWindow({ startMs: bounds.endMs - DEFAULT_WINDOW_MS, endMs: bounds.endMs }, bounds);
  }
  const lastActivity = [...(response?.points ?? [])]
    .reverse()
    .find((point) => point.totalCount > 0);
  const activityEnd = parseEpoch(lastActivity?.bucketEnd) ?? bounds.endMs;
  return clampWindow({ startMs: activityEnd - DEFAULT_WINDOW_MS, endMs: activityEnd }, bounds);
}

export function useInvocationTimeline({
  response,
  closedNaturalDay,
  upstreamAccountId,
  liveRevision,
  liveConnected = true,
}: UseInvocationTimelineOptions) {
  const bounds = useMemo<InvocationTimelineWindow | null>(() => {
    const startMs = parseEpoch(response?.rangeStart);
    const endMs = parseEpoch(response?.rangeEnd);
    return startMs != null && endMs != null && endMs > startMs ? { startMs, endMs } : null;
  }, [response?.rangeEnd, response?.rangeStart]);
  const boundsKey = bounds ? `${bounds.startMs}:${bounds.endMs}:${closedNaturalDay}` : "empty";
  const [viewportWindow, setViewportWindow] = useState<InvocationTimelineWindow | null>(() =>
    resolveInitialWindow(response, closedNaturalDay),
  );
  const [data, setData] = useState<InvocationTimelineResponse | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const requestSequence = useRef(0);
  const abortControllerRef = useRef<AbortController | null>(null);
  const previousBoundsKey = useRef(boundsKey);

  useEffect(() => {
    if (previousBoundsKey.current === boundsKey) return;
    previousBoundsKey.current = boundsKey;
    setViewportWindow(resolveInitialWindow(response, closedNaturalDay));
    setData(null);
  }, [boundsKey, closedNaturalDay, response]);

  useEffect(() => {
    if (!bounds || !viewportWindow) return;
    setViewportWindow((current) => {
      if (!current) return current;
      const next = clampWindow(current, bounds);
      return next.startMs === current.startMs && next.endMs === current.endMs ? current : next;
    });
  }, [bounds, viewportWindow]);

  const refresh = useCallback(async () => {
    if (!viewportWindow) return;
    abortControllerRef.current?.abort();
    const controller = new AbortController();
    const sequence = requestSequence.current + 1;
    requestSequence.current = sequence;
    abortControllerRef.current = controller;
    setIsLoading(true);
    try {
      const next = await fetchInvocationTimeline({
        from: new Date(viewportWindow.startMs).toISOString(),
        to: new Date(viewportWindow.endMs).toISOString(),
        upstreamAccountId,
        signal: controller.signal,
      });
      if (sequence !== requestSequence.current) return;
      setData(next);
      setError(null);
    } catch (nextError) {
      if (controller.signal.aborted) return;
      if (sequence !== requestSequence.current) return;
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    } finally {
      if (sequence === requestSequence.current && abortControllerRef.current === controller) {
        abortControllerRef.current = null;
        setIsLoading(false);
      }
    }
  }, [upstreamAccountId, viewportWindow]);

  useEffect(
    () => () => {
      abortControllerRef.current?.abort();
      abortControllerRef.current = null;
    },
    [],
  );

  // liveRevision is an explicit SSE-driven refresh trigger for the stable callback.
  // biome-ignore lint/correctness/useExhaustiveDependencies: liveRevision intentionally retriggers the fetch.
  useEffect(() => {
    if (!closedNaturalDay && !liveConnected) return;
    void refresh();
  }, [closedNaturalDay, liveConnected, refresh, liveRevision]);

  useEffect(() => {
    if (closedNaturalDay || !liveConnected) return;
    const timer = globalThis.setInterval(() => void refresh(), LIVE_REFRESH_MS);
    return () => globalThis.clearInterval(timer);
  }, [closedNaturalDay, liveConnected, refresh]);

  const updateWindow = useCallback(
    (next: InvocationTimelineWindow) => {
      if (!bounds) return;
      setViewportWindow(clampWindow(next, bounds));
    },
    [bounds],
  );

  return {
    data,
    error,
    isLoading,
    isRefreshing: isLoading && data != null,
    window: viewportWindow,
    bounds,
    setWindow: updateWindow,
    refresh,
  };
}
