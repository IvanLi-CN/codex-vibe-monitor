import { useMemo } from "react";
import { usePromptCacheConversations } from "./usePromptCacheConversations";
import {
  DASHBOARD_WORKING_CONVERSATIONS_LIMIT,
  DASHBOARD_WORKING_CONVERSATIONS_SELECTION,
  mapPromptCacheConversationsToDashboardCards,
} from "../lib/dashboardWorkingConversations";

export function useDashboardWorkingConversations() {
  const { stats, isLoading, error, refresh } = usePromptCacheConversations(
    DASHBOARD_WORKING_CONVERSATIONS_SELECTION,
  );

  const cards = useMemo(
    () =>
      mapPromptCacheConversationsToDashboardCards(stats, {
        limit: DASHBOARD_WORKING_CONVERSATIONS_LIMIT,
      }),
    [stats],
  );

  return {
    cards,
    stats,
    isLoading,
    error,
    refresh,
  };
}
