import { useCallback, useEffect, useState } from "react";
import {
  getCachedTopicState,
  getTopicDescriptorKey,
  requestTopicRefresh,
  type SubscriptionTopicDescriptor,
  type SubscriptionTopicEnvelope,
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
  const [lastKind, setLastKind] = useState<SubscriptionTopicEnvelope["type"] | null>(() =>
    descriptor && enabled ? (getCachedTopicState<T>(descriptor)?.lastKind ?? null) : null,
  );
  const [deliverySource, setDeliverySource] = useState<"cache" | "network" | null>(() =>
    descriptor && enabled && getCachedTopicState<T>(descriptor)?.payload != null ? "cache" : null,
  );
  const [isLoading, setIsLoading] = useState(() =>
    Boolean(
      descriptor &&
        enabled &&
        getCachedTopicState<T>(descriptor)?.payload == null &&
        getCachedTopicState<T>(descriptor)?.error == null,
    ),
  );
  const [error, setError] = useState<string | null>(() =>
    descriptor && enabled ? (getCachedTopicState<T>(descriptor)?.error ?? null) : null,
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: descriptorKey is the canonical topic identity and avoids redundant subscriptions for equivalent descriptors.
  useEffect(() => {
    if (!descriptor || !enabled) {
      setData(null);
      setDataDescriptorKey(null);
      setLastReceivedAt(null);
      setLastKind(null);
      setDeliverySource(null);
      setIsLoading(false);
      setError(null);
      return;
    }
    const cached = getCachedTopicState<T>(descriptor);
    setData(cached?.payload ?? null);
    setDataDescriptorKey(descriptorKey);
    setLastReceivedAt(cached?.receivedAt ?? null);
    setLastKind(cached?.lastKind ?? null);
    setDeliverySource(cached?.payload != null ? "cache" : null);
    setIsLoading(cached?.payload == null && cached?.error == null);
    setError(cached?.error ?? null);
    const unsubscribe = subscribeToTopic<T>(descriptor, (event) => {
      if (event.type === "unavailable") {
        setData(null);
        setDataDescriptorKey(descriptorKey);
        setLastReceivedAt(null);
        setLastKind(null);
        setDeliverySource(null);
        setError(event.errorCode);
        setIsLoading(false);
        return;
      }
      const nextCached = getCachedTopicState<T>(descriptor);
      setData(event.payload);
      setDataDescriptorKey(descriptorKey);
      setLastReceivedAt(nextCached?.receivedAt ?? Date.now());
      setLastKind(event.type);
      setDeliverySource(event.deliverySource ?? "network");
      setError(null);
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
    lastKind: isCurrentDescriptor ? lastKind : null,
    deliverySource: isCurrentDescriptor ? deliverySource : null,
    isLoading: enabled ? (isCurrentDescriptor ? isLoading : true) : false,
    error: isCurrentDescriptor ? error : null,
    refresh,
  };
}
