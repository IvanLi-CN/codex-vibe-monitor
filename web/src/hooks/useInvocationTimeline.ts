import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { InvocationTimelineResponse, TimeseriesResponse } from "../lib/api";
import { fetchInvocationTimeline } from "../lib/api";

const DEFAULT_WINDOW_MS = 30 * 60 * 1_000;
const LIVE_REFRESH_MS = 15_000;
const INVOCATION_TIMELINE_PAGE_SIZE = 500;
const INVOCATION_TIMELINE_MAX_PAGES = 1_000;

export interface InvocationTimelineWindow {
  startMs: number;
  endMs: number;
}

interface UseInvocationTimelineOptions {
  response: TimeseriesResponse | null;
  closedNaturalDay: boolean;
  upstreamAccountId?: number;
  liveRevision?: number;
  liveRefreshAllowed?: boolean;
  enabled?: boolean;
}

function parseEpoch(value: string | null | undefined) {
  if (!value) return null;
  const epoch = Date.parse(value);
  return Number.isFinite(epoch) ? epoch : null;
}

function canonicalTimelineEpoch(value: number) {
  return Math.floor(value / 1_000) * 1_000;
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
  const startMs = parseEpoch(response?.rangeStart);
  const endMs = parseEpoch(response?.rangeEnd);
  if (startMs == null || endMs == null || endMs <= startMs) return null;
  const bounds = {
    startMs,
    endMs,
  };
  if (!closedNaturalDay) {
    return clampWindow({ startMs: bounds.endMs - DEFAULT_WINDOW_MS, endMs: bounds.endMs }, bounds);
  }
  const lastActivity = [...(response?.points ?? [])]
    .reverse()
    .find((point) => point.totalCount > 0);
  const activityEnd = parseEpoch(lastActivity?.bucketEnd) ?? bounds.endMs;
  return clampWindow({ startMs: activityEnd - DEFAULT_WINDOW_MS, endMs: activityEnd }, bounds);
}

function timelineRecordKey(record: InvocationTimelineResponse["records"][number]) {
  return `${record.invokeId}\u0000${record.occurredAt}`;
}

function preferTimelineRecord(
  current: InvocationTimelineResponse["records"][number],
  next: InvocationTimelineResponse["records"][number],
) {
  if (current.isInFlight !== next.isInFlight) return current.isInFlight ? next : current;
  return next.id >= current.id ? next : current;
}

async function fetchInvocationTimelineSnapshot(options: {
  naturalDayStart: string;
  naturalDayEnd: string;
  from: string;
  to: string;
  upstreamAccountId?: number;
  includeLive: boolean;
  signal: AbortSignal;
}) {
  let cursor: string | undefined;
  let asOf: string | undefined;
  let firstPage: InvocationTimelineResponse | null = null;
  const records = new Map<string, InvocationTimelineResponse["records"][number]>();

  for (let pageIndex = 0; pageIndex < INVOCATION_TIMELINE_MAX_PAGES; pageIndex += 1) {
    const page = await fetchInvocationTimeline({
      ...options,
      limit: INVOCATION_TIMELINE_PAGE_SIZE,
      cursor,
      asOf,
    });
    firstPage ??= page;
    if (asOf == null) asOf = page.asOf;
    if (page.asOf !== asOf) {
      throw new Error("Invocation timeline snapshot changed while paging");
    }
    for (const record of page.records) {
      const key = timelineRecordKey(record);
      const current = records.get(key);
      records.set(key, current ? preferTimelineRecord(current, record) : record);
    }
    if (!page.hasMore) break;
    if (!page.nextCursor) throw new Error("Invocation timeline page is missing nextCursor");
    cursor = page.nextCursor;
    if (pageIndex === INVOCATION_TIMELINE_MAX_PAGES - 1) {
      throw new Error("Invocation timeline has too many pages");
    }
  }

  if (!firstPage || !asOf) throw new Error("Invocation timeline returned no snapshot");
  if (!Number.isSafeInteger(firstPage.total) || firstPage.total < 0) {
    throw new Error("Invocation timeline returned an invalid total");
  }
  const mergedRecords = [...records.values()].sort(
    (left, right) =>
      Date.parse(left.occurredAt) - Date.parse(right.occurredAt) || left.id - right.id,
  );
  if (mergedRecords.length !== firstPage.total) {
    throw new Error("Invocation timeline snapshot is incomplete");
  }
  return {
    ...firstPage,
    asOf,
    snapshotAtMs: Date.now(),
    total: firstPage.total,
    hasMore: false,
    nextCursor: null,
    records: mergedRecords,
  } satisfies InvocationTimelineResponse;
}

export function useInvocationTimeline({
  response,
  closedNaturalDay,
  upstreamAccountId,
  liveRevision,
  liveRefreshAllowed = true,
  enabled = true,
}: UseInvocationTimelineOptions) {
  const bounds = useMemo<InvocationTimelineWindow | null>(() => {
    const startMs = parseEpoch(response?.rangeStart);
    const endMs = parseEpoch(response?.rangeEnd);
    return startMs != null && endMs != null && endMs > startMs ? { startMs, endMs } : null;
  }, [response?.rangeEnd, response?.rangeStart]);
  const boundsContextKey = bounds
    ? `${bounds.startMs}:${closedNaturalDay}:${upstreamAccountId ?? "all"}`
    : "empty";
  const [requestedWindow, setRequestedWindow] = useState<InvocationTimelineWindow | null>(() =>
    resolveInitialWindow(response, closedNaturalDay),
  );
  const [committedSnapshot, setCommittedSnapshot] = useState<{
    data: InvocationTimelineResponse;
    window: InvocationTimelineWindow;
  } | null>(null);
  const [isLoading, setIsLoading] = useState(enabled);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestSequence = useRef(0);
  const abortControllerRef = useRef<AbortController | null>(null);
  const inFlightRefreshRef = useRef<Promise<void> | null>(null);
  const pendingRefreshRef = useRef(false);
  const pendingRefreshTimerRef = useRef<ReturnType<typeof globalThis.setTimeout> | null>(null);
  const refreshRef = useRef<(() => Promise<void>) | null>(null);
  const liveRefreshAllowedRef = useRef(liveRefreshAllowed);
  liveRefreshAllowedRef.current = liveRefreshAllowed;
  const hasDataRef = useRef(false);
  const autoWindowAdvanceRef = useRef(false);
  const [committedBoundsContextKey, setCommittedBoundsContextKey] = useState(boundsContextKey);
  const previousBoundsContextKey = useRef(boundsContextKey);
  const previousBounds = useRef<InvocationTimelineWindow | null>(bounds);
  const requestedWindowRef = useRef(requestedWindow);
  requestedWindowRef.current = requestedWindow;

  useEffect(() => {
    if (!bounds) {
      previousBoundsContextKey.current = boundsContextKey;
      previousBounds.current = null;
      setCommittedBoundsContextKey(boundsContextKey);
      requestSequence.current += 1;
      abortControllerRef.current?.abort();
      abortControllerRef.current = null;
      inFlightRefreshRef.current = null;
      pendingRefreshRef.current = false;
      if (pendingRefreshTimerRef.current != null) {
        globalThis.clearTimeout(pendingRefreshTimerRef.current);
        pendingRefreshTimerRef.current = null;
      }
      setRequestedWindow(null);
      setCommittedSnapshot(null);
      autoWindowAdvanceRef.current = false;
      hasDataRef.current = false;
      setError(null);
      setIsLoading(false);
      setIsRefreshing(false);
      return;
    }
    const contextChanged = previousBoundsContextKey.current !== boundsContextKey;
    previousBoundsContextKey.current = boundsContextKey;
    if (contextChanged) {
      setCommittedBoundsContextKey(boundsContextKey);
      requestSequence.current += 1;
      abortControllerRef.current?.abort();
      abortControllerRef.current = null;
      inFlightRefreshRef.current = null;
      pendingRefreshRef.current = false;
      if (pendingRefreshTimerRef.current != null) {
        globalThis.clearTimeout(pendingRefreshTimerRef.current);
        pendingRefreshTimerRef.current = null;
      }
      previousBounds.current = bounds;
      const initialWindow = resolveInitialWindow(response, closedNaturalDay);
      requestedWindowRef.current = initialWindow;
      setRequestedWindow(initialWindow);
      setCommittedSnapshot(null);
      autoWindowAdvanceRef.current = false;
      hasDataRef.current = false;
      setError(null);
      setIsLoading(enabled);
      setIsRefreshing(false);
      return;
    }
    const priorBounds = previousBounds.current;
    previousBounds.current = bounds;
    const current = requestedWindow;
    if (!current) {
      setRequestedWindow(resolveInitialWindow(response, closedNaturalDay));
      return;
    }
    const liveDelta = priorBounds ? bounds.endMs - priorBounds.endMs : 0;
    const followsLiveEnd =
      liveDelta > 0 && priorBounds != null && Math.abs(current.endMs - priorBounds.endMs) <= 1;
    const next = clampWindow(
      followsLiveEnd
        ? { startMs: current.startMs + liveDelta, endMs: current.endMs + liveDelta }
        : current,
      bounds,
    );
    if (next.startMs === current.startMs && next.endMs === current.endMs) return;
    requestedWindowRef.current = next;
    autoWindowAdvanceRef.current = true;
    setRequestedWindow(next);
  }, [bounds, boundsContextKey, closedNaturalDay, enabled, requestedWindow, response]);

  useEffect(() => {
    if (!bounds || !requestedWindow) return;
    const next = clampWindow(requestedWindow, bounds);
    if (next.startMs === requestedWindow.startMs && next.endMs === requestedWindow.endMs) return;
    requestedWindowRef.current = next;
    setRequestedWindow(next);
  }, [bounds, requestedWindow]);

  const refresh = useCallback(async () => {
    if (!enabled || !requestedWindow) return;
    if (inFlightRefreshRef.current) {
      pendingRefreshRef.current = true;
      return inFlightRefreshRef.current;
    }
    const controller = new AbortController();
    const sequence = requestSequence.current;
    const requestedTarget = requestedWindow;
    abortControllerRef.current = controller;
    setIsLoading(!hasDataRef.current);
    setIsRefreshing(hasDataRef.current);
    const request = (async () => {
      try {
        const next = await fetchInvocationTimelineSnapshot({
          naturalDayStart: new Date(bounds?.startMs ?? requestedTarget.startMs).toISOString(),
          naturalDayEnd: new Date(bounds?.endMs ?? requestedTarget.endMs).toISOString(),
          from: new Date(requestedTarget.startMs).toISOString(),
          to: new Date(requestedTarget.endMs).toISOString(),
          upstreamAccountId,
          includeLive: !closedNaturalDay,
          signal: controller.signal,
        });
        if (sequence !== requestSequence.current) return;
        const returnedStart = parseEpoch(next.rangeStart);
        const returnedEnd = parseEpoch(next.rangeEnd);
        if (
          returnedStart == null ||
          returnedEnd == null ||
          canonicalTimelineEpoch(returnedStart) !==
            canonicalTimelineEpoch(requestedTarget.startMs) ||
          canonicalTimelineEpoch(returnedEnd) !== canonicalTimelineEpoch(requestedTarget.endMs)
        ) {
          throw new Error("Invocation timeline snapshot does not match the requested window");
        }
        const latestTarget = requestedWindowRef.current;
        if (
          !latestTarget ||
          latestTarget.startMs !== requestedTarget.startMs ||
          latestTarget.endMs !== requestedTarget.endMs
        ) {
          pendingRefreshRef.current = true;
          return;
        }
        hasDataRef.current = true;
        setCommittedSnapshot({ data: next, window: requestedTarget });
        setError(null);
      } catch (nextError) {
        if (controller.signal.aborted) return;
        if (sequence !== requestSequence.current) return;
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      } finally {
        if (sequence === requestSequence.current && abortControllerRef.current === controller) {
          abortControllerRef.current = null;
          inFlightRefreshRef.current = null;
          setIsLoading(!hasDataRef.current);
          setIsRefreshing(false);
          if (pendingRefreshRef.current) {
            pendingRefreshRef.current = false;
            const followUpSequence = sequence;
            let followUpTimer: ReturnType<typeof globalThis.setTimeout>;
            followUpTimer = globalThis.setTimeout(() => {
              if (pendingRefreshTimerRef.current !== followUpTimer) return;
              pendingRefreshTimerRef.current = null;
              if (followUpSequence !== requestSequence.current) return;
              if (!closedNaturalDay && !liveRefreshAllowedRef.current) {
                pendingRefreshRef.current = false;
                return;
              }
              void refreshRef.current?.();
            }, 0);
            pendingRefreshTimerRef.current = followUpTimer;
          }
        }
      }
    })();
    inFlightRefreshRef.current = request;
    return request;
  }, [bounds, closedNaturalDay, enabled, requestedWindow, upstreamAccountId]);

  refreshRef.current = refresh;

  useEffect(() => {
    if (enabled) return;
    requestSequence.current += 1;
    abortControllerRef.current?.abort();
    abortControllerRef.current = null;
    inFlightRefreshRef.current = null;
    pendingRefreshRef.current = false;
    if (pendingRefreshTimerRef.current != null) {
      globalThis.clearTimeout(pendingRefreshTimerRef.current);
      pendingRefreshTimerRef.current = null;
    }
    setIsLoading(false);
    setIsRefreshing(false);
    setError(null);
  }, [enabled]);

  useEffect(
    () => () => {
      requestSequence.current += 1;
      abortControllerRef.current?.abort();
      abortControllerRef.current = null;
      inFlightRefreshRef.current = null;
      pendingRefreshRef.current = false;
      if (pendingRefreshTimerRef.current != null) {
        globalThis.clearTimeout(pendingRefreshTimerRef.current);
        pendingRefreshTimerRef.current = null;
      }
    },
    [],
  );

  // liveRevision is an explicit SSE-driven refresh trigger for the stable callback.
  const liveRefreshRevision = closedNaturalDay ? undefined : liveRevision;
  // biome-ignore lint/correctness/useExhaustiveDependencies: liveRefreshRevision intentionally retriggers the fetch.
  useEffect(() => {
    if (!enabled) return;
    if (committedBoundsContextKey !== boundsContextKey) return;
    if (autoWindowAdvanceRef.current) {
      autoWindowAdvanceRef.current = false;
      if (
        requestedWindow &&
        requestedWindowRef.current &&
        (requestedWindow.startMs !== requestedWindowRef.current.startMs ||
          requestedWindow.endMs !== requestedWindowRef.current.endMs)
      ) {
        return;
      }
    }
    if (!closedNaturalDay && !liveRefreshAllowed && hasDataRef.current) return;
    void refresh();
  }, [
    boundsContextKey,
    closedNaturalDay,
    committedBoundsContextKey,
    enabled,
    liveRefreshAllowed,
    refresh,
    liveRefreshRevision,
  ]);

  useEffect(() => {
    if (!enabled || closedNaturalDay || !liveRefreshAllowed) return;
    const timer = globalThis.setInterval(() => void refresh(), LIVE_REFRESH_MS);
    return () => globalThis.clearInterval(timer);
  }, [closedNaturalDay, enabled, liveRefreshAllowed, refresh]);

  const updateWindow = useCallback(
    (next: InvocationTimelineWindow) => {
      if (!bounds) return;
      const normalized = clampWindow(next, bounds);
      if (
        requestedWindow?.startMs === normalized.startMs &&
        requestedWindow.endMs === normalized.endMs
      ) {
        return;
      }
      requestedWindowRef.current = normalized;
      setRequestedWindow(normalized);
      setCommittedSnapshot(null);
      hasDataRef.current = false;
      setError(null);
      setIsLoading(true);
      setIsRefreshing(false);
    },
    [bounds, requestedWindow],
  );

  const contextReady = committedBoundsContextKey === boundsContextKey;
  const data = committedSnapshot?.data ?? null;
  const committedWindow = committedSnapshot?.window ?? null;
  const dataMatchesCommittedWindow =
    data != null &&
    committedWindow != null &&
    parseEpoch(data.rangeStart) != null &&
    parseEpoch(data.rangeEnd) != null &&
    canonicalTimelineEpoch(parseEpoch(data.rangeStart) as number) ===
      canonicalTimelineEpoch(committedWindow.startMs) &&
    canonicalTimelineEpoch(parseEpoch(data.rangeEnd) as number) ===
      canonicalTimelineEpoch(committedWindow.endMs);
  const visibleData = contextReady && dataMatchesCommittedWindow ? data : null;

  return {
    data: visibleData,
    error,
    isStale: data != null && error != null,
    isFrozen: !closedNaturalDay && !liveRefreshAllowed,
    isLoading: contextReady ? (visibleData ? false : isLoading) : enabled,
    isRefreshing: contextReady && isRefreshing && visibleData != null,
    window: contextReady
      ? visibleData && committedWindow
        ? committedWindow
        : requestedWindow
      : null,
    bounds,
    setWindow: updateWindow,
    refresh,
  };
}
