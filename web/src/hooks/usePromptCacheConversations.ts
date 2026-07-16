import {
  type PromptCacheConversationSelection,
  type PromptCacheConversationsResponse,
} from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

export const PROMPT_CACHE_SSE_REFRESH_THROTTLE_MS = 5_000;
export const PROMPT_CACHE_POLLING_REFRESH_INTERVAL_MS = 60_000;
export const PROMPT_CACHE_OPEN_RESYNC_COOLDOWN_MS = 3_000;

function buildPromptCacheTopic(selection: PromptCacheConversationSelection) {
  return buildTopicDescriptor("prompt-cache.window", {
    ...(selection.mode === "count"
      ? { limit: selection.limit }
      : "activityMinutes" in selection
        ? { activityMinutes: selection.activityMinutes }
        : { activityHours: selection.activityHours }),
    detail: "full",
    recentInvocationLimit: 16,
  });
}

export function usePromptCacheConversations(selection: PromptCacheConversationSelection) {
  const topic = buildPromptCacheTopic(selection);
  const { data, isLoading, error, refresh } =
    useSubscriptionTopic<PromptCacheConversationsResponse>(topic);

  return {
    stats: data,
    isLoading,
    error,
    refresh,
  };
}
