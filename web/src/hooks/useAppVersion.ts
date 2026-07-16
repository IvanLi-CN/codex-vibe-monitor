import type { VersionResponse } from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

export function useAppVersion() {
  const topic = buildTopicDescriptor("app.version");
  const { data, isLoading, error, refresh } = useSubscriptionTopic<VersionResponse>(topic);

  return {
    versionInfo: data,
    isLoading,
    error,
    refresh,
  };
}
