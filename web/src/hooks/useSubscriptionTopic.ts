import { useCallback, useEffect, useState } from "react";
import {
  getCachedTopicState,
  getTopicDescriptorKey,
  requestTopicRefresh,
  type SubscriptionTopicDescriptor,
  subscribeToTopic,
} from "../lib/sse";

export function useSubscriptionTopic<T>(
  descriptor: SubscriptionTopicDescriptor | null,
  enabled = true,
) {
  const descriptorKey = descriptor ? getTopicDescriptorKey(descriptor) : null;
  const [data, setData] = useState<T | null>(() =>
    descriptor && enabled ? (getCachedTopicState<T>(descriptor)?.payload ?? null) : null,
  );
  const [lastReceivedAt, setLastReceivedAt] = useState<number | null>(() =>
    descriptor && enabled ? (getCachedTopicState<T>(descriptor)?.receivedAt ?? null) : null,
  );
  const [isLoading, setIsLoading] = useState(() =>
    Boolean(descriptor && enabled && getCachedTopicState<T>(descriptor)?.payload == null),
  );

  useEffect(() => {
    if (!descriptor || !enabled) {
      setData(null);
      setLastReceivedAt(null);
      setIsLoading(false);
      return;
    }
    const cached = getCachedTopicState<T>(descriptor);
    setData(cached?.payload ?? null);
    setLastReceivedAt(cached?.receivedAt ?? null);
    setIsLoading(cached?.payload == null);
    const unsubscribe = subscribeToTopic<T>(descriptor, (event) => {
      const nextCached = getCachedTopicState<T>(descriptor);
      setData(event.payload);
      setLastReceivedAt(nextCached?.receivedAt ?? Date.now());
      setIsLoading(false);
    });
    return unsubscribe;
  }, [descriptorKey, enabled]);

  const refresh = useCallback(() => {
    if (!descriptor || !enabled) return;
    setIsLoading(true);
    requestTopicRefresh(descriptor);
  }, [descriptorKey, enabled]);

  return {
    data,
    lastReceivedAt,
    isLoading,
    error: null as string | null,
    refresh,
  };
}
