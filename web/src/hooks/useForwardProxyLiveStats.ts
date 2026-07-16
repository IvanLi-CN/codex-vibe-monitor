import type { ForwardProxyLiveStatsResponse } from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

export const FORWARD_PROXY_SSE_REFRESH_THROTTLE_MS = 5_000;
export const FORWARD_PROXY_POLLING_REFRESH_INTERVAL_MS = 60_000;
export const FORWARD_PROXY_OPEN_RESYNC_COOLDOWN_MS = 3_000;

export function getForwardProxySseRefreshDelay(lastRefreshAt: number, now: number) {
  return Math.max(0, FORWARD_PROXY_SSE_REFRESH_THROTTLE_MS - (now - lastRefreshAt));
}

export function shouldTriggerForwardProxyOpenResync(
  lastResyncAt: number,
  now: number,
  force = false,
) {
  if (force) return true;
  return now - lastResyncAt >= FORWARD_PROXY_OPEN_RESYNC_COOLDOWN_MS;
}

export function useForwardProxyLiveStats() {
  const topic = buildTopicDescriptor("forward-proxy.live");
  const { data, isLoading, error, refresh } =
    useSubscriptionTopic<ForwardProxyLiveStatsResponse>(topic);

  return {
    stats: data,
    isLoading,
    error,
    refresh,
  };
}
