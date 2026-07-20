import { useEffect, useMemo } from "react";
import type { DashboardRecentNetworkWindowResponse } from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

function buildDashboardRecentNetworkWindowTopic() {
  return buildTopicDescriptor("dashboard.network-recent.current", {});
}

export function useDashboardRecentNetworkWindow(enabled: boolean) {
  const topic = useMemo(
    () => (enabled ? buildDashboardRecentNetworkWindowTopic() : null),
    [enabled],
  );
  const subscription = useSubscriptionTopic<DashboardRecentNetworkWindowResponse>(topic, enabled);

  useEffect(() => {
    if (!enabled) {
      return undefined;
    }

    const timer = window.setInterval(() => {
      subscription.refresh();
    }, 1_000);

    return () => window.clearInterval(timer);
  }, [enabled, subscription.refresh]);

  return {
    data: subscription.data,
    isLoading: subscription.isLoading,
    isRefreshing: false,
    error: subscription.error,
    reload: async () => {
      subscription.refresh();
    },
  };
}
