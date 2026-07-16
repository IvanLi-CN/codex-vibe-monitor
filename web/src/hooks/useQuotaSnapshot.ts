import type { QuotaSnapshot } from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

export function useQuotaSnapshot() {
  const topic = buildTopicDescriptor("quota.current");
  const { data, isLoading, error, refresh } = useSubscriptionTopic<QuotaSnapshot>(topic);

  return {
    snapshot: data,
    isLoading,
    error,
    refresh,
  };
}
