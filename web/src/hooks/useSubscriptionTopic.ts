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
  const [dataDescriptorKey, setDataDescriptorKey] = useState<string | null>(() =>
    descriptor && enabled ? descriptorKey : null,
  );
  const [lastReceivedAt, setLastReceivedAt] = useState<number | null>(() =>
    descriptor && enabled ? (getCachedTopicState<T>(descriptor)?.receivedAt ?? null) : null,
  );
  const [isLoading, setIsLoading] = useState(() =>
    Boolean(descriptor && enabled && getCachedTopicState<T>(descriptor)?.payload == null),
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: descriptorKey is the canonical topic identity and avoids redundant subscriptions for equivalent descriptors.
  useEffect(() => {
    if (!descriptor || !enabled) {
      setData(null);
      setDataDescriptorKey(null);
      setLastReceivedAt(null);
      setIsLoading(false);
      return;
    }
    const cached = getCachedTopicState<T>(descriptor);
    setData(cached?.payload ?? null);
    setDataDescriptorKey(descriptorKey);
    setLastReceivedAt(cached?.receivedAt ?? null);
    setIsLoading(cached?.payload == null);
    const unsubscribe = subscribeToTopic<T>(descriptor, (event) => {
      const nextCached = getCachedTopicState<T>(descriptor);
      setData(event.payload);
      setDataDescriptorKey(descriptorKey);
      setLastReceivedAt(nextCached?.receivedAt ?? Date.now());
      setIsLoading(false);
    });
    return unsubscribe;
  }, [descriptorKey, enabled]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: refresh follows the same canonical descriptor identity as the subscription effect.
  const refresh = useCallback(() => {
    if (!descriptor || !enabled) return;
    setIsLoading(true);
    requestTopicRefresh(descriptor);
  }, [descriptorKey, enabled]);

  const isCurrentDescriptor = enabled && dataDescriptorKey === descriptorKey;

  return {
    // A descriptor change renders before its subscription effect runs. Do not
    // expose the previous descriptor's cached payload during that render.
    data: isCurrentDescriptor ? data : null,
    descriptorKey,
    lastReceivedAt: isCurrentDescriptor ? lastReceivedAt : null,
    isLoading: enabled ? (isCurrentDescriptor ? isLoading : true) : false,
    error: null as string | null,
    refresh,
  };
}
