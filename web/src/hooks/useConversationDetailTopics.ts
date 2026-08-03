import { useMemo } from "react";
import type {
  ApiInvocation,
  InvocationRecordsResponse,
  InvocationRecordsSummaryResponse,
  PromptCacheConversationBindingResponse,
  PromptCacheConversationOperationEventListResponse,
} from "../lib/api";
import { buildTopicDescriptor } from "../lib/sse";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

export type ConversationDetailScope =
  | { promptCacheKey: string }
  | { stickyKey: string; upstreamAccountId: number };

export interface InvocationHistoryOverviewTopicPayload {
  summary: InvocationRecordsSummaryResponse;
  records: ApiInvocation[];
  chartTotal: number;
  chartIsSampled: boolean;
  chartRangeStart: string | null;
  chartRangeEnd: string | null;
}

export function resolveConversationDetailScope(
  conversationKey: string | null,
  query?: {
    promptCacheKey?: string;
    stickyKey?: string;
    upstreamAccountId?: number;
  },
): ConversationDetailScope | null {
  if (!conversationKey) return null;
  if (query?.stickyKey) {
    if (query.upstreamAccountId != null && query.upstreamAccountId > 0) {
      return {
        stickyKey: query.stickyKey,
        upstreamAccountId: query.upstreamAccountId,
      };
    }
    return null;
  }
  return { promptCacheKey: query?.promptCacheKey ?? conversationKey };
}

function scopeParams(scope: ConversationDetailScope | null) {
  if (!scope) return null;
  if ("promptCacheKey" in scope) return { promptCacheKey: scope.promptCacheKey };
  return {
    stickyKey: scope.stickyKey,
    upstreamAccountId: scope.upstreamAccountId,
  };
}

export function useConversationDetailTopics({
  open,
  activeTab,
  scope,
  operationsInfoType,
}: {
  open: boolean;
  activeTab: "overview" | "calls" | "settings" | "operations";
  scope: ConversationDetailScope | null;
  operationsInfoType?: string;
}) {
  const params = useMemo(() => scopeParams(scope), [scope]);
  const callsDescriptor = useMemo(
    () => (params ? buildTopicDescriptor("invocation-history.window", params) : null),
    [params],
  );
  const overviewDescriptor = useMemo(
    () => (params ? buildTopicDescriptor("invocation-history.overview", params) : null),
    [params],
  );
  const bindingDescriptor = useMemo(
    () =>
      params ? buildTopicDescriptor("prompt-cache.conversation-binding.current", params) : null,
    [params],
  );
  const operationsDescriptor = useMemo(
    () =>
      params
        ? buildTopicDescriptor("prompt-cache.conversation-operations.window", {
            ...params,
            infoType: operationsInfoType,
          })
        : null,
    [operationsInfoType, params],
  );

  const calls = useSubscriptionTopic<InvocationRecordsResponse>(
    callsDescriptor,
    open && activeTab === "calls",
  );
  const overview = useSubscriptionTopic<InvocationHistoryOverviewTopicPayload>(
    overviewDescriptor,
    open && activeTab === "overview",
  );
  const binding = useSubscriptionTopic<PromptCacheConversationBindingResponse>(
    bindingDescriptor,
    open && activeTab === "settings",
  );
  const operations = useSubscriptionTopic<PromptCacheConversationOperationEventListResponse>(
    operationsDescriptor,
    open && activeTab === "operations",
  );

  return { calls, overview, binding, operations };
}
