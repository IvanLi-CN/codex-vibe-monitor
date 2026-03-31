import { useCallback, useMemo } from "react";
import { useTranslation } from "../i18n";
import type {
  ApiInvocation,
  PromptCacheConversation,
  PromptCacheConversationsResponse,
  PromptCacheConversationUpstreamAccount,
  UpstreamStickyConversationsResponse,
} from "../lib/api";
import { PromptCacheConversationTable } from "./PromptCacheConversationTable";

interface StickyKeyConversationTableProps {
  accountId: number | null;
  accountDisplayName?: string | null;
  stats: UpstreamStickyConversationsResponse | null;
  isLoading: boolean;
  error?: string | null;
  expandedStickyKeys?: string[];
  onToggleExpandedStickyKey?: (stickyKey: string) => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}

function buildStickyConversationUpstreamAccounts(
  conversation: UpstreamStickyConversationsResponse["conversations"][number],
  accountId: number | null,
  accountDisplayName?: string | null,
): PromptCacheConversationUpstreamAccount[] {
  if (accountId == null) return [];
  const trimmedDisplayName = accountDisplayName?.trim();
  return [
    {
      upstreamAccountId: accountId,
      upstreamAccountName: trimmedDisplayName || null,
      requestCount: conversation.requestCount,
      totalTokens: conversation.totalTokens,
      totalCost: conversation.totalCost,
      lastActivityAt: conversation.lastActivityAt,
    },
  ];
}

function adaptStickyConversation(
  conversation: UpstreamStickyConversationsResponse["conversations"][number],
  accountId: number | null,
  accountDisplayName?: string | null,
): PromptCacheConversation {
  return {
    promptCacheKey: conversation.stickyKey,
    requestCount: conversation.requestCount,
    totalTokens: conversation.totalTokens,
    totalCost: conversation.totalCost,
    createdAt: conversation.createdAt,
    lastActivityAt: conversation.lastActivityAt,
    upstreamAccounts: buildStickyConversationUpstreamAccounts(
      conversation,
      accountId,
      accountDisplayName,
    ),
    recentInvocations: conversation.recentInvocations,
    last24hRequests: conversation.last24hRequests,
  };
}

function adaptStickyStats(
  stats: UpstreamStickyConversationsResponse | null,
  accountId: number | null,
  accountDisplayName?: string | null,
): PromptCacheConversationsResponse | null {
  if (!stats) return null;
  return {
    rangeStart: stats.rangeStart,
    rangeEnd: stats.rangeEnd,
    selectionMode: stats.selectionMode,
    selectedLimit: stats.selectedLimit,
    selectedActivityHours: stats.selectedActivityHours,
    implicitFilter: stats.implicitFilter,
    conversations: stats.conversations.map((conversation) =>
      adaptStickyConversation(conversation, accountId, accountDisplayName),
    ),
  };
}

export function StickyKeyConversationTable({
  accountId,
  accountDisplayName,
  stats,
  isLoading,
  error,
  expandedStickyKeys,
  onToggleExpandedStickyKey,
  onOpenUpstreamAccount,
}: StickyKeyConversationTableProps) {
  const { t } = useTranslation();

  const promptCacheStats = useMemo(
    () => adaptStickyStats(stats, accountId, accountDisplayName),
    [accountDisplayName, accountId, stats],
  );
  const historyQueryForConversationKey = useCallback(
    (stickyKey: string) => ({
      stickyKey,
      ...(accountId != null ? { upstreamAccountId: accountId } : {}),
    }),
    [accountId],
  );
  const historyRecordMatchesConversationKey = useCallback(
    (record: ApiInvocation, stickyKey: string) => {
      const resolvedStickyKey =
        record.stickyKey?.trim() || record.promptCacheKey?.trim() || "";
      if (resolvedStickyKey !== stickyKey) return false;
      if (accountId == null) return true;
      return record.upstreamAccountId === accountId;
    },
    [accountId],
  );

  return (
    <PromptCacheConversationTable
      stats={promptCacheStats}
      isLoading={isLoading}
      error={error}
      expandedPromptCacheKeys={expandedStickyKeys}
      onToggleExpandedPromptCacheKey={onToggleExpandedStickyKey}
      onOpenUpstreamAccount={onOpenUpstreamAccount}
      keyColumnLabel={t(
        "accountPool.upstreamAccounts.stickyConversations.table.stickyKey",
      )}
      emptyLabel={t("accountPool.upstreamAccounts.stickyConversations.empty")}
      historyQueryForConversationKey={historyQueryForConversationKey}
      historyRecordMatchesConversationKey={historyRecordMatchesConversationKey}
    />
  );
}
