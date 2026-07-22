import { useEffect, useMemo, useState } from "react";
import type { DashboardRecentNetworkWindowResponse } from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

export const DASHBOARD_RECENT_NETWORK_STALE_MS = 5_000;

function buildDashboardRecentNetworkWindowTopic() {
  return buildTopicDescriptor("dashboard.network-recent.current", {});
}

export function useDashboardRecentNetworkWindow(enabled: boolean) {
  const topic = useMemo(
    () => (enabled ? buildDashboardRecentNetworkWindowTopic() : null),
    [enabled],
  );
  const subscription = useSubscriptionTopic<DashboardRecentNetworkWindowResponse>(topic, enabled);
  const [staleClock, setStaleClock] = useState(() => Date.now());

  useEffect(() => {
    if (!enabled || subscription.lastReceivedAt == null) {
      setStaleClock(Date.now());
      return undefined;
    }

    const elapsedMs = Date.now() - subscription.lastReceivedAt;
    const timeoutMs = Math.max(0, DASHBOARD_RECENT_NETWORK_STALE_MS - elapsedMs + 1);
    const timer = window.setTimeout(() => {
      setStaleClock(Date.now());
    }, timeoutMs);

    return () => window.clearTimeout(timer);
  }, [enabled, subscription.lastReceivedAt]);

  const isStale =
    enabled &&
    subscription.data != null &&
    subscription.lastReceivedAt != null &&
    staleClock - subscription.lastReceivedAt >= DASHBOARD_RECENT_NETWORK_STALE_MS;

  return {
    data: subscription.data,
    isLoading: subscription.isLoading,
    isRefreshing: false,
    isStale,
    lastReceivedAt: subscription.lastReceivedAt,
    error: subscription.error,
    reload: async () => undefined,
  };
}
