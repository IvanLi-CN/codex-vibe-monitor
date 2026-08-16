import { useCallback, useEffect, useMemo, useState } from "react";
import type { FetchModelRoutingLiveQuery, ModelRoutingLiveResponse } from "../lib/api";
import { fetchModelRoutingLive } from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

export function useModelRoutingLive(query: FetchModelRoutingLiveQuery, enabled = true) {
  const [snapshot, setSnapshot] = useState<ModelRoutingLiveResponse | null>(null);
  const [isLoadingSnapshot, setIsLoadingSnapshot] = useState(enabled);
  const [error, setError] = useState<string | null>(null);
  const [snapshotReady, setSnapshotReady] = useState(false);
  const [refreshVersion, setRefreshVersion] = useState(0);

  const normalizedQuery = useMemo(
    () => ({
      window: query.window ?? "1h",
      model: query.model?.trim() || undefined,
      state: query.state?.trim() || undefined,
      limit: Math.max(1, Math.min(100, Math.trunc(query.limit ?? 100))),
    }),
    [query.limit, query.model, query.state, query.window],
  );

  useEffect(() => {
    if (!enabled) {
      setSnapshot(null);
      setError(null);
      setSnapshotReady(false);
      setIsLoadingSnapshot(false);
      return;
    }
    const controller = new AbortController();
    setSnapshotReady(false);
    setIsLoadingSnapshot(true);
    setError(null);
    void fetchModelRoutingLive({ ...normalizedQuery, signal: controller.signal })
      .then((response) => {
        if (controller.signal.aborted) return;
        setSnapshot(response);
        setSnapshotReady(true);
      })
      .catch((cause: unknown) => {
        if (controller.signal.aborted) return;
        setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setIsLoadingSnapshot(false);
      });
    return () => controller.abort();
  }, [enabled, normalizedQuery, refreshVersion]);

  const topic = useMemo(
    () =>
      enabled && snapshotReady
        ? buildTopicDescriptor("pool.model-routing-live", normalizedQuery)
        : null,
    [enabled, normalizedQuery, snapshotReady],
  );
  const subscription = useSubscriptionTopic<ModelRoutingLiveResponse>(
    topic,
    enabled && snapshotReady,
  );

  useEffect(() => {
    if (subscription.data) {
      setSnapshot(subscription.data);
      setError(null);
    }
  }, [subscription.data]);

  const refresh = useCallback(() => setRefreshVersion((value) => value + 1), []);

  return {
    data: snapshot,
    isLoading: isLoadingSnapshot || (snapshotReady && subscription.isLoading && snapshot == null),
    error,
    refresh,
  };
}
