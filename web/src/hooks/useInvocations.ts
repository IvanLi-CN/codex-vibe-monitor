import { useEffect, useMemo, useRef } from "react";
import type { ApiInvocation, ListResponse } from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

export interface InvocationFilters {
  model?: string;
  status?: string;
}

function recordsChanged(next: ApiInvocation[], current: ApiInvocation[]) {
  return (
    next.length !== current.length ||
    next.some((record, index) => {
      const existing = current[index];
      return (
        !existing ||
        existing.invokeId !== record.invokeId ||
        existing.occurredAt !== record.occurredAt
      );
    })
  );
}

function buildInvocationsTopic(limit: number, filters?: InvocationFilters) {
  return buildTopicDescriptor("invocations.window", {
    limit,
    model: filters?.model,
    status: filters?.status,
  });
}

export function useInvocationStream(
  limit: number,
  filters?: InvocationFilters,
  onNewRecords?: (records: ApiInvocation[]) => void,
  options?: { enableStream?: boolean },
) {
  const enableStream = options?.enableStream ?? true;
  const topic = enableStream ? buildInvocationsTopic(limit, filters) : null;
  const { data, isLoading, error, refresh } = useSubscriptionTopic<ListResponse>(
    topic,
    enableStream,
  );
  const records = useMemo(() => data?.records ?? [], [data]);
  const previousRecordsRef = useRef<ApiInvocation[]>([]);

  useEffect(() => {
    if (!onNewRecords) {
      previousRecordsRef.current = records;
      return;
    }
    const previous = previousRecordsRef.current;
    if (recordsChanged(records, previous)) {
      onNewRecords(records);
    }
    previousRecordsRef.current = records;
  }, [onNewRecords, records]);

  return {
    records,
    isLoading,
    error,
    hasData: records.length > 0,
    refresh,
  };
}
