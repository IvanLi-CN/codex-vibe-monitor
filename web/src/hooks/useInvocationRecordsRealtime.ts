import { useEffect, useRef } from "react";
import type {
  ApiInvocation,
  BroadcastPayload,
  InvocationRecordsQuery,
  InvocationSortBy,
  InvocationSortOrder,
  ListResponse,
} from "../lib/api";
import { invocationStableKey } from "../lib/invocation";
import { buildTopicDescriptor, subscribeToTopic } from "../lib/sse";

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

function supportsRealtimeTopic(
  filters: RealtimeFilters | undefined,
  sortBy: InvocationSortBy,
  sortOrder: InvocationSortOrder,
) {
  if (sortBy !== "occurredAt" || sortOrder !== "desc") {
    return false;
  }
  const {
    from,
    to,
    endpoint,
    requestId,
    failureClass,
    failureKind,
    promptCacheKey,
    stickyKey,
    requesterIp,
    upstreamAccountId,
    keyword,
    minTotalTokens,
    maxTotalTokens,
    minTotalMs,
    maxTotalMs,
  } = filters ?? {};
  return !(
    from ||
    to ||
    endpoint ||
    requestId ||
    failureClass ||
    failureKind ||
    promptCacheKey ||
    stickyKey ||
    requesterIp ||
    upstreamAccountId != null ||
    keyword ||
    minTotalTokens != null ||
    maxTotalTokens != null ||
    minTotalMs != null ||
    maxTotalMs != null
  );
}

function buildInvocationsTopic(limit: number, filters?: RealtimeFilters) {
  return buildTopicDescriptor("invocations.window", {
    limit,
    model: filters?.model,
    status: filters?.status,
  });
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
}: UseInvocationRecordsRealtimeOptions) {
  const getRecordsRef = useRef(getRecords);
  const onRecordsChangeRef = useRef(onRecordsChange);

  useEffect(() => {
    getRecordsRef.current = getRecords;
  }, [getRecords]);

  useEffect(() => {
    onRecordsChangeRef.current = onRecordsChange;
  }, [onRecordsChange]);

  useEffect(() => {
    if (!enabled || !isHydrated) return;
    if (!supportsRealtimeTopic(filters, sortBy, sortOrder)) return;

    const topic = buildInvocationsTopic(limit, filters);
    const unsubscribe = subscribeToTopic<ListResponse>(topic, (event) => {
      const current = getRecordsRef.current();
      const currentKeySet = new Set(current.map((record) => invocationStableKey(record)));
      const nextRecords = allowVisibleInsertions
        ? event.payload.records
        : event.payload.records.filter((record) => currentKeySet.has(invocationStableKey(record)));
      if (!recordsChanged(nextRecords, current)) {
        return;
      }
      const visibleInsertedKeys = nextRecords
        .map((record) => invocationStableKey(record))
        .filter((key) => !currentKeySet.has(key));
      onRecordsChangeRef.current(nextRecords, {
        visibleInsertedKeys,
        payload: {
          type: "records",
          records: nextRecords,
        } as BroadcastPayload & { type: "records" },
      });
    });

    return unsubscribe;
  }, [allowVisibleInsertions, enabled, filters, isHydrated, limit, sortBy, sortOrder]);
}
