import { useMemo } from "react";
import type { DashboardNetworkTimeseriesResponse } from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { getBrowserTimeZone } from "../lib/timeZone";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

function buildDashboardNetworkTimeseriesTopic(
  range: "today" | "yesterday" | "1d",
  upstreamAccountId?: number,
) {
  return buildTopicDescriptor("dashboard.network-timeseries.window", {
    range,
    timeZone: getBrowserTimeZone(),
    upstreamAccountId,
  });
}

export function useDashboardNetworkTimeseries(
  range: "today" | "yesterday" | "1d",
  enabled: boolean,
  upstreamAccountId?: number,
) {
  const topic = useMemo(
    () => (enabled ? buildDashboardNetworkTimeseriesTopic(range, upstreamAccountId) : null),
    [enabled, range, upstreamAccountId],
  );
  const subscription = useSubscriptionTopic<DashboardNetworkTimeseriesResponse>(topic, enabled);

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
