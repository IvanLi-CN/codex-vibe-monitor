import { useMemo } from "react";
import { useForwardProxyLiveStats } from "../hooks/useForwardProxyLiveStats";
import { useInvocationStream } from "../hooks/useInvocations";
import { useModelRoutingLive } from "../hooks/useModelRoutingLive";
import { usePromptCacheConversations } from "../hooks/usePromptCacheConversations";
import { useSummary } from "../hooks/useStats";
import type { PromptCacheConversationSelection } from "../lib/api";
import { resolveInvocationDisplayStatus } from "../lib/invocationStatus";
import type { LiveTab } from "./useLivePageState";

export function useLivePageData({
  activeTab,
  limit,
  summaryWindow,
  conversationSelection,
  routingWindow,
}: {
  activeTab: LiveTab;
  limit: number;
  summaryWindow: string;
  conversationSelection: PromptCacheConversationSelection;
  routingWindow: "15m" | "1h" | "6h" | "24h";
}) {
  const summary = useSummary(summaryWindow, summaryWindow === "current" ? { limit } : undefined);
  const stream = useInvocationStream(limit, undefined, undefined, {
    enableStream: activeTab === "records",
  });
  const chartRecords = useMemo(
    () =>
      stream.records.filter((record) => {
        const status = resolveInvocationDisplayStatus(record)?.trim().toLowerCase() ?? "";
        return status !== "running" && status !== "pending";
      }),
    [stream.records],
  );
  const conversations = usePromptCacheConversations(
    conversationSelection,
    activeTab === "conversations",
  );
  const routing = useModelRoutingLive(
    { window: routingWindow, limit: 100 },
    activeTab === "routing",
  );
  const proxy = useForwardProxyLiveStats(activeTab === "proxy");

  return { summary, stream, chartRecords, conversations, routing, proxy };
}
