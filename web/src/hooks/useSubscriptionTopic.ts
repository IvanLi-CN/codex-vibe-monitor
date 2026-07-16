import { useCallback, useEffect, useState } from "react";
import {
  getCachedTopicState,
  requestTopicRefresh,
  type SubscriptionTopicDescriptor,
  subscribeToTopic,
} from "../lib/sse";

export function useSubscriptionTopic<T>(
  descriptor: SubscriptionTopicDescriptor | null,
  enabled = true,
) {
  const [data, setData] = useState<T | null>(() =>
    descriptor && enabled ? (getCachedTopicState<T>(descriptor)?.payload ?? null) : null,
  );
  const [isLoading, setIsLoading] = useState(() =>
    Boolean(descriptor && enabled && getCachedTopicState<T>(descriptor)?.payload == null),
  );

  useEffect(() => {
    if (!descriptor || !enabled) {
      setData(null);
      setIsLoading(false);
      return;
    }
    const cached = getCachedTopicState<T>(descriptor);
    setData(cached?.payload ?? null);
    setIsLoading(cached?.payload == null);
    const unsubscribe = subscribeToTopic<T>(descriptor, (event) => {
      setData(event.payload);
      setIsLoading(false);
    });
    return unsubscribe;
  }, [descriptor, enabled]);

  const refresh = useCallback(() => {
    if (!descriptor || !enabled) return;
    setIsLoading(true);
    requestTopicRefresh(descriptor);
  }, [descriptor, enabled]);

  return {
    data,
    isLoading,
    error: null as string | null,
    refresh,
  };
}
