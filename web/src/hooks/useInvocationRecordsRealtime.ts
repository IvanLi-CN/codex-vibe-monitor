import { useCallback, useEffect, useRef } from "react";
import type {
  ApiInvocation,
  BroadcastPayload,
  InvocationRecordsQuery,
  InvocationSortBy,
  InvocationSortOrder,
} from "../lib/api";
import { invocationStableKey } from "../lib/invocation";
import { mergeInvocationWindowRecords } from "../lib/invocationRecordsLive";
import { subscribeToSse, subscribeToSseOpen } from "../lib/sse";

type RealtimeFilters = Pick<
  InvocationRecordsQuery,
  | "from"
  | "to"
  | "status"
  | "model"
  | "endpoint"
  | "requestId"
  | "failureClass"
  | "failureKind"
  | "promptCacheKey"
  | "stickyKey"
  | "requesterIp"
  | "upstreamAccountId"
  | "keyword"
  | "minTotalTokens"
  | "maxTotalTokens"
  | "minTotalMs"
  | "maxTotalMs"
>;

interface UseInvocationRecordsRealtimeOptions {
  enabled: boolean;
  isHydrated: boolean;
  filters?: RealtimeFilters;
  sortBy: InvocationSortBy;
  sortOrder: InvocationSortOrder;
  limit: number;
  allowVisibleInsertions?: boolean;
  getRecords: () => ApiInvocation[];
  onRecordsChange: (
    next: ApiInvocation[],
    meta: { visibleInsertedKeys: string[]; payload: BroadcastPayload & { type: "records" } },
  ) => void;
  onOpenResync: () => void;
  openResyncCooldownMs?: number;
}

function recordsChanged(next: ApiInvocation[], current: ApiInvocation[]) {
  return (
    next.length !== current.length ||
    next.some((record, index) => {
      const existing = current[index];
      return !existing || JSON.stringify(existing) !== JSON.stringify(record);
    })
  );
}

export function useInvocationRecordsRealtime({
  enabled,
  isHydrated,
  filters,
  sortBy,
  sortOrder,
  limit,
  allowVisibleInsertions = true,
  getRecords,
  onRecordsChange,
  onOpenResync,
  openResyncCooldownMs = 3000,
}: UseInvocationRecordsRealtimeOptions) {
  const getRecordsRef = useRef(getRecords);
  const onRecordsChangeRef = useRef(onRecordsChange);
  const onOpenResyncRef = useRef(onOpenResync);
  const pendingOpenResyncRef = useRef(false);
  const lastOpenResyncAtRef = useRef(0);

  useEffect(() => {
    getRecordsRef.current = getRecords;
  }, [getRecords]);

  useEffect(() => {
    onRecordsChangeRef.current = onRecordsChange;
  }, [onRecordsChange]);

  useEffect(() => {
    onOpenResyncRef.current = onOpenResync;
  }, [onOpenResync]);

  const requestOpenResync = useCallback(
    (force = false) => {
      if (!enabled) return;
      if (!isHydrated) {
        pendingOpenResyncRef.current = true;
        return;
      }
      const now = Date.now();
      if (!force && now - lastOpenResyncAtRef.current < openResyncCooldownMs) {
        return;
      }
      lastOpenResyncAtRef.current = now;
      pendingOpenResyncRef.current = false;
      onOpenResyncRef.current();
    },
    [enabled, isHydrated, openResyncCooldownMs],
  );

  useEffect(() => {
    if (!enabled || !isHydrated || !pendingOpenResyncRef.current) return;
    requestOpenResync(true);
  }, [enabled, isHydrated, requestOpenResync]);

  useEffect(() => {
    if (!enabled) {
      pendingOpenResyncRef.current = false;
      return;
    }

    const unsubscribe = subscribeToSse((payload: BroadcastPayload) => {
      if (payload.type !== "records") return;
      const current = getRecordsRef.current();
      const currentKeySet = new Set(current.map((record) => invocationStableKey(record)));
      const incoming = allowVisibleInsertions
        ? payload.records
        : payload.records.filter((record) => currentKeySet.has(invocationStableKey(record)));
      const mergedPayload = { ...payload, records: incoming } as BroadcastPayload & {
        type: "records";
      };
      const mergedNext = mergeInvocationWindowRecords(current, incoming, {
        filters,
        sortBy,
        sortOrder,
        limit,
      });
      if (!recordsChanged(mergedNext, current)) {
        return;
      }
      const visibleInsertedKeys = mergedNext
        .map((record) => invocationStableKey(record))
        .filter((key) => !currentKeySet.has(key));
      onRecordsChangeRef.current(mergedNext, { visibleInsertedKeys, payload: mergedPayload });
    });

    return unsubscribe;
  }, [allowVisibleInsertions, enabled, filters, limit, sortBy, sortOrder]);

  useEffect(() => {
    if (!enabled) return;
    const unsubscribe = subscribeToSseOpen(() => {
      requestOpenResync();
    });
    return unsubscribe;
  }, [enabled, requestOpenResync]);
}
