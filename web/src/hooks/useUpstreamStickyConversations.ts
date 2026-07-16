import type {
  StickyKeyConversationSelection,
  UpstreamStickyConversationsResponse,
} from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

export const UPSTREAM_STICKY_SSE_REFRESH_THROTTLE_MS = 5_000;
export const UPSTREAM_STICKY_POLLING_REFRESH_INTERVAL_MS = 60_000;
export const UPSTREAM_STICKY_OPEN_RESYNC_COOLDOWN_MS = 3_000;

export function getUpstreamStickySseRefreshDelay(lastRefreshAt: number, now: number) {
  return Math.max(0, UPSTREAM_STICKY_SSE_REFRESH_THROTTLE_MS - (now - lastRefreshAt));
}

export function shouldTriggerUpstreamStickyOpenResync(
  lastResyncAt: number,
  now: number,
  force = false,
) {
  if (force) return true;
  return now - lastResyncAt >= UPSTREAM_STICKY_OPEN_RESYNC_COOLDOWN_MS;
}

function buildStickyTopic(accountId: number, selection: StickyKeyConversationSelection) {
  return buildTopicDescriptor("prompt-cache.sticky.window", {
    accountId,
    ...(selection.mode === "count"
      ? { limit: selection.limit }
      : { activityHours: selection.activityHours }),
  });
}

export function useUpstreamStickyConversations(
  accountId: number | null,
  selection: StickyKeyConversationSelection,
  enabled = true,
) {
  const topic = enabled && accountId != null ? buildStickyTopic(accountId, selection) : null;
  const { data, isLoading, error, refresh } =
    useSubscriptionTopic<UpstreamStickyConversationsResponse>(topic, enabled && accountId != null);

  return {
    stats: data,
    isLoading,
    error,
    refresh,
  };
}
