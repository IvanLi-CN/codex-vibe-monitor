import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { InvocationTimelineResponse, TimeseriesResponse } from "../lib/api";
import { fetchInvocationTimeline, releaseInvocationTimelineSnapshot } from "../lib/api";

const DEFAULT_WINDOW_MS = 30 * 60 * 1_000;
const LIVE_REFRESH_MS = 15_000;
const LIVE_REFRESH_TIMER_GUARD_MS = 50;
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
  void releaseInvocationTimelineSnapshot(asOf).catch((error: unknown) => {
    console.debug(
      "Invocation timeline snapshot release failed; the server TTL will reclaim it",
      error instanceof Error ? error.message : String(error),
    );
  });
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
  const pendingAutomaticRefreshRef = useRef(false);
  const automaticRefreshTimerRef = useRef<ReturnType<typeof globalThis.setTimeout> | null>(null);
  const refreshRef = useRef<(() => Promise<void>) | null>(null);
  const scheduleAutomaticRefreshRef = useRef<(() => void) | null>(null);
  const lastTraversalStartedAtRef = useRef<number | null>(null);
  const retryAttemptRef = useRef(0);
  const retryNotBeforeRef = useRef(0);
  const immediateRefreshRequiredRef = useRef(true);
  const previousLiveRevisionRef = useRef(liveRevision);
  const wasLiveRefreshAllowedRef = useRef(liveRefreshAllowed);
  const liveRefreshAllowedRef = useRef(liveRefreshAllowed);
  liveRefreshAllowedRef.current = liveRefreshAllowed;
  const hasDataRef = useRef(false);
  const requestContextRef = useRef({ bounds, closedNaturalDay, enabled, upstreamAccountId });
  requestContextRef.current = { bounds, closedNaturalDay, enabled, upstreamAccountId };
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
      pendingAutomaticRefreshRef.current = false;
      if (automaticRefreshTimerRef.current != null) {
        globalThis.clearTimeout(automaticRefreshTimerRef.current);
        automaticRefreshTimerRef.current = null;
      }
      setRequestedWindow(null);
      setCommittedSnapshot(null);
      immediateRefreshRequiredRef.current = true;
      retryAttemptRef.current = 0;
      retryNotBeforeRef.current = 0;
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
      pendingAutomaticRefreshRef.current = false;
      if (automaticRefreshTimerRef.current != null) {
        globalThis.clearTimeout(automaticRefreshTimerRef.current);
        automaticRefreshTimerRef.current = null;
      }
      previousBounds.current = bounds;
      const initialWindow = resolveInitialWindow(response, closedNaturalDay);
      requestedWindowRef.current = initialWindow;
      setRequestedWindow(initialWindow);
      setCommittedSnapshot(null);
      immediateRefreshRequiredRef.current = true;
      retryAttemptRef.current = 0;
      retryNotBeforeRef.current = 0;
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
    setRequestedWindow(next);
    if (followsLiveEnd && hasDataRef.current) {
      scheduleAutomaticRefreshRef.current?.();
    }
  }, [bounds, boundsContextKey, closedNaturalDay, enabled, requestedWindow, response]);

  useEffect(() => {
    if (!bounds || !requestedWindow) return;
    const next = clampWindow(requestedWindow, bounds);
    if (next.startMs === requestedWindow.startMs && next.endMs === requestedWindow.endMs) return;
    requestedWindowRef.current = next;
    setRequestedWindow(next);
  }, [bounds, requestedWindow]);

  const refresh = useCallback(async () => {
    const context = requestContextRef.current;
    const requestedTarget = requestedWindowRef.current;
    if (!context.enabled || !requestedTarget) return;
    if (inFlightRefreshRef.current) {
      pendingAutomaticRefreshRef.current = true;
      return inFlightRefreshRef.current;
    }
    const controller = new AbortController();
    const sequence = requestSequence.current;
    abortControllerRef.current = controller;
    lastTraversalStartedAtRef.current = Date.now();
    setIsLoading(!hasDataRef.current);
    setIsRefreshing(hasDataRef.current);
    const request = (async () => {
      try {
        const next = await fetchInvocationTimelineSnapshot({
          naturalDayStart: new Date(
            context.bounds?.startMs ?? requestedTarget.startMs,
          ).toISOString(),
          naturalDayEnd: new Date(context.bounds?.endMs ?? requestedTarget.endMs).toISOString(),
          from: new Date(requestedTarget.startMs).toISOString(),
          to: new Date(requestedTarget.endMs).toISOString(),
          upstreamAccountId: context.upstreamAccountId,
          includeLive: !context.closedNaturalDay,
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
          pendingAutomaticRefreshRef.current = true;
          return;
        }
        hasDataRef.current = true;
        retryAttemptRef.current = 0;
        retryNotBeforeRef.current = 0;
        setCommittedSnapshot({ data: next, window: requestedTarget });
        setError(null);
      } catch (nextError) {
        if (controller.signal.aborted) return;
        if (sequence !== requestSequence.current) return;
        setError(nextError instanceof Error ? nextError.message : String(nextError));
        retryAttemptRef.current = Math.min(retryAttemptRef.current + 1, 3);
        const retryBaseMs = 15_000 * 2 ** (retryAttemptRef.current - 1);
        const retryDelayMs = retryBaseMs * (0.8 + Math.random() * 0.4);
        retryNotBeforeRef.current = Date.now() + retryDelayMs;
        pendingAutomaticRefreshRef.current = true;
      } finally {
        if (sequence === requestSequence.current && abortControllerRef.current === controller) {
          abortControllerRef.current = null;
          inFlightRefreshRef.current = null;
          setIsLoading(!hasDataRef.current);
          setIsRefreshing(false);
          if (pendingAutomaticRefreshRef.current) {
            scheduleAutomaticRefreshRef.current?.();
          }
        }
      }
    })();
    inFlightRefreshRef.current = request;
    return request;
  }, []);

  refreshRef.current = refresh;

  const scheduleAutomaticRefresh = useCallback(() => {
    const context = requestContextRef.current;
    if (!context.enabled) return;
    const retryPending = retryAttemptRef.current > 0;
    if (context.closedNaturalDay && retryAttemptRef.current === 0) return;
    if (!context.closedNaturalDay && !liveRefreshAllowedRef.current && !retryPending) return;
    pendingAutomaticRefreshRef.current = true;
    if (inFlightRefreshRef.current || automaticRefreshTimerRef.current != null) return;

    const now = Date.now();
    const intervalDueAt =
      lastTraversalStartedAtRef.current == null
        ? now
        : lastTraversalStartedAtRef.current + LIVE_REFRESH_MS + LIVE_REFRESH_TIMER_GUARD_MS;
    const nextAttemptAt = Math.max(intervalDueAt, retryNotBeforeRef.current);
    const delayMs = Math.max(0, nextAttemptAt - now);
    const timer = globalThis.setTimeout(() => {
      if (automaticRefreshTimerRef.current !== timer) return;
      automaticRefreshTimerRef.current = null;
      if (!pendingAutomaticRefreshRef.current) return;
      const currentContext = requestContextRef.current;
      if (!currentContext.enabled) return;
      const retryIsPending = retryAttemptRef.current > 0;
      if (currentContext.closedNaturalDay && retryAttemptRef.current === 0) return;
      if (!currentContext.closedNaturalDay && !liveRefreshAllowedRef.current && !retryIsPending)
        return;
      if (inFlightRefreshRef.current) return;
      pendingAutomaticRefreshRef.current = false;
      void refreshRef.current?.();
    }, delayMs);
    automaticRefreshTimerRef.current = timer;
  }, []);

  scheduleAutomaticRefreshRef.current = scheduleAutomaticRefresh;

  const forceRefresh = useCallback(() => {
    pendingAutomaticRefreshRef.current = false;
    if (automaticRefreshTimerRef.current != null) {
      globalThis.clearTimeout(automaticRefreshTimerRef.current);
      automaticRefreshTimerRef.current = null;
    }
    return refreshRef.current?.() ?? Promise.resolve();
  }, []);

  useEffect(() => {
    if (!enabled || !requestedWindow) return;
    if (committedBoundsContextKey !== boundsContextKey) return;
    if (!immediateRefreshRequiredRef.current) return;
    immediateRefreshRequiredRef.current = false;
    void forceRefresh();
  }, [boundsContextKey, committedBoundsContextKey, enabled, forceRefresh, requestedWindow]);

  useEffect(() => {
    const revisionChanged = previousLiveRevisionRef.current !== liveRevision;
    const reconnected = !wasLiveRefreshAllowedRef.current && liveRefreshAllowed;
    previousLiveRevisionRef.current = liveRevision;
    wasLiveRefreshAllowedRef.current = liveRefreshAllowed;
    if (!enabled || closedNaturalDay || !liveRefreshAllowed) return;
    if (committedBoundsContextKey !== boundsContextKey) return;
    if (revisionChanged || reconnected) scheduleAutomaticRefresh();
  }, [
    boundsContextKey,
    closedNaturalDay,
    committedBoundsContextKey,
    enabled,
    liveRefreshAllowed,
    liveRevision,
    scheduleAutomaticRefresh,
  ]);

  useEffect(() => {
    if (!enabled || closedNaturalDay || !liveRefreshAllowed) return;
    const timer = globalThis.setInterval(scheduleAutomaticRefresh, LIVE_REFRESH_MS);
    return () => globalThis.clearInterval(timer);
  }, [closedNaturalDay, enabled, liveRefreshAllowed, scheduleAutomaticRefresh]);

  useEffect(() => {
    if (enabled) return;
    immediateRefreshRequiredRef.current = true;
    requestSequence.current += 1;
    abortControllerRef.current?.abort();
    abortControllerRef.current = null;
    inFlightRefreshRef.current = null;
    pendingAutomaticRefreshRef.current = false;
    if (automaticRefreshTimerRef.current != null) {
      globalThis.clearTimeout(automaticRefreshTimerRef.current);
      automaticRefreshTimerRef.current = null;
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
      pendingAutomaticRefreshRef.current = false;
      if (automaticRefreshTimerRef.current != null) {
        globalThis.clearTimeout(automaticRefreshTimerRef.current);
        automaticRefreshTimerRef.current = null;
      }
    },
    [],
  );

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
      requestSequence.current += 1;
      abortControllerRef.current?.abort();
      abortControllerRef.current = null;
      inFlightRefreshRef.current = null;
      pendingAutomaticRefreshRef.current = false;
      if (automaticRefreshTimerRef.current != null) {
        globalThis.clearTimeout(automaticRefreshTimerRef.current);
        automaticRefreshTimerRef.current = null;
      }
      retryAttemptRef.current = 0;
      retryNotBeforeRef.current = 0;
      immediateRefreshRequiredRef.current = true;
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
    refresh: forceRefresh,
  };
}
