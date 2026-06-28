import { useEffect, useLayoutEffect, useState } from "react";
import {
  DashboardActivityOverview,
} from "../components/DashboardActivityOverview";
import { DashboardInvocationDetailDrawer } from "../components/DashboardInvocationDetailDrawer";
import {
  DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
  persistDashboardActivityRange,
  readPersistedDashboardActivityRange,
  type DashboardActivityRangeKey,
} from "../components/dashboardActivityRange";
import { DashboardPerformanceDiagnostics } from "../components/DashboardPerformanceDiagnostics";
import { DashboardWorkingConversationsSection } from "../components/DashboardWorkingConversationsSection";
import { PromptCacheConversationHistoryDrawer } from "../components/PromptCacheConversationTable";
import { useDashboardWorkingConversations } from "../hooks/useDashboardWorkingConversations";
import { resetDashboardPerformanceDiagnostics } from "../lib/dashboardPerformanceDiagnostics";
import {
  formatDashboardWorkingConversationSequenceId,
  type DashboardWorkingConversationInvocationSelection,
} from "../lib/dashboardWorkingConversations";
import type { DashboardWorkingConversationSelection } from "../components/DashboardWorkingConversationsSection";
import { useTranslation } from "../i18n";
import { useUpstreamAccountDetailRoute } from "../hooks/useUpstreamAccountDetailRoute";
import { SharedUpstreamAccountDetailDrawer } from "./account-pool/UpstreamAccounts";

export default function DashboardPage() {
  const { t } = useTranslation();
  const [activeRange, setActiveRange] = useState<DashboardActivityRangeKey>(() =>
    readPersistedDashboardActivityRange(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY),
  );
  const [selectedInvocation, setSelectedInvocation] =
    useState<DashboardWorkingConversationInvocationSelection | null>(null);
  const [selectedConversation, setSelectedConversation] =
    useState<DashboardWorkingConversationSelection | null>(null);
  const { upstreamAccountId, openUpstreamAccount, closeUpstreamAccount } =
    useUpstreamAccountDetailRoute();
  const {
    cards,
    totalMatched,
    hasMore,
    isLoading: workingCardsLoading,
    isLoadingMore: workingCardsLoadingMore,
    error: workingCardsError,
    loadMore,
    recentPreviewLimit,
    setRefreshTargetCount,
  } = useDashboardWorkingConversations();

  useEffect(() => {
    if (upstreamAccountId != null) {
      setSelectedInvocation(null);
      setSelectedConversation(null);
    }
  }, [upstreamAccountId]);

  useLayoutEffect(() => {
    resetDashboardPerformanceDiagnostics();
  }, []);

  useEffect(() => {
    persistDashboardActivityRange(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, activeRange);
  }, [activeRange]);

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <DashboardActivityOverview
        activeRange={activeRange}
        onActiveRangeChange={setActiveRange}
      />
      <DashboardPerformanceDiagnostics />

      <DashboardWorkingConversationsSection
        activeRange={activeRange}
        cards={cards}
        totalMatched={totalMatched}
        hasMore={hasMore}
        recentPreviewLimit={recentPreviewLimit}
        isLoading={workingCardsLoading}
        isLoadingMore={workingCardsLoadingMore}
        error={workingCardsError}
        onLoadMore={loadMore}
        setRefreshTargetCount={setRefreshTargetCount}
        onOpenUpstreamAccount={(accountId) => {
          setSelectedInvocation(null);
          setSelectedConversation(null);
          openUpstreamAccount(accountId);
        }}
        onOpenConversation={(selection) => {
          closeUpstreamAccount({ replace: true });
          setSelectedInvocation(null);
          setSelectedConversation(selection);
        }}
        onOpenInvocation={(selection) => {
          closeUpstreamAccount({ replace: true });
          setSelectedConversation(null);
          setSelectedInvocation(selection);
        }}
      />
      <DashboardInvocationDetailDrawer
        open={selectedInvocation != null}
        selection={selectedInvocation}
        onClose={() => setSelectedInvocation(null)}
        onOpenUpstreamAccount={(accountId) => {
          setSelectedInvocation(null);
          setSelectedConversation(null);
          openUpstreamAccount(accountId);
        }}
      />
      <PromptCacheConversationHistoryDrawer
        open={selectedConversation != null}
        conversationKey={selectedConversation?.promptCacheKey ?? null}
        conversationLabel={
          selectedConversation
            ? formatDashboardWorkingConversationSequenceId(
                selectedConversation.conversationSequenceId,
              )
            : null
        }
        onClose={() => setSelectedConversation(null)}
        t={t}
        onOpenUpstreamAccount={(accountId) => {
          setSelectedInvocation(null);
          setSelectedConversation(null);
          openUpstreamAccount(accountId);
        }}
      />
      {upstreamAccountId != null ? (
        <SharedUpstreamAccountDetailDrawer
          open
          accountId={upstreamAccountId}
          onClose={closeUpstreamAccount}
        />
      ) : null}
    </div>
  );
}
