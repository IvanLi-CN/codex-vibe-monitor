import { useEffect, useLayoutEffect, useState } from "react";
import { DashboardActivityOverview } from "../features/dashboard/DashboardActivityOverview";
import { DashboardInvocationDetailDrawer } from "../features/dashboard/DashboardInvocationDetailDrawer";
import { DashboardPerformanceDiagnostics } from "../features/dashboard/DashboardPerformanceDiagnostics";
import type {
  DashboardOpenUpstreamAccountOptions,
  DashboardWorkingConversationSelection,
} from "../features/dashboard/DashboardWorkingConversationsSection";
import { DashboardWorkingConversationsSection } from "../features/dashboard/DashboardWorkingConversationsSection";
import {
  DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
  type DashboardActivityRangeKey,
  persistDashboardActivityRange,
  readPersistedDashboardActivityRange,
} from "../features/dashboard/dashboardActivityRange";
import { PromptCacheConversationHistoryDrawer } from "../features/prompt-cache/PromptCacheConversationTable";
import { useDashboardActivitySnapshot } from "../hooks/useDashboardUpstreamAccountActivity";
import { useDashboardWorkingConversations } from "../hooks/useDashboardWorkingConversations";
import { useUpstreamAccountDetailRoute } from "../hooks/useUpstreamAccountDetailRoute";
import { useTranslation } from "../i18n";
import { resetDashboardPerformanceDiagnostics } from "../lib/dashboardPerformanceDiagnostics";
import {
  type DashboardWorkingConversationInvocationSelection,
  formatDashboardWorkingConversationSequenceId,
} from "../lib/dashboardWorkingConversations";
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
  const [includeUpstreamAccountActivity, setIncludeUpstreamAccountActivity] = useState(false);
  const { upstreamAccountId, upstreamAccountTab, openUpstreamAccount, closeUpstreamAccount } =
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
  const dashboardActivityEnabled = activeRange !== "usage";
  const {
    data: dashboardActivity,
    isLoading: dashboardActivityLoading,
    error: dashboardActivityError,
    recentInvocationLimit: upstreamAccountRecentPreviewLimit,
    reload: reloadDashboardActivity,
  } = useDashboardActivitySnapshot(
    activeRange,
    dashboardActivityEnabled,
    includeUpstreamAccountActivity,
    recentPreviewLimit,
  );

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

  const handleOpenUpstreamAccount = (
    accountId: number,
    _accountLabel: string,
    options?: DashboardOpenUpstreamAccountOptions,
  ) => {
    setSelectedInvocation(null);
    setSelectedConversation(null);
    openUpstreamAccount(accountId, { tab: options?.tab });
  };

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <DashboardActivityOverview
        activeRange={activeRange}
        onActiveRangeChange={setActiveRange}
        dashboardActivity={dashboardActivity}
        dashboardActivityLoading={dashboardActivityLoading}
        dashboardActivityError={dashboardActivityError}
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
        onOpenUpstreamAccount={handleOpenUpstreamAccount}
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
        upstreamAccountActivity={
          dashboardActivity?.accounts
            ? {
                range: dashboardActivity.range,
                rangeStart: dashboardActivity.rangeStart,
                rangeEnd: dashboardActivity.rangeEnd,
                accounts: dashboardActivity.accounts,
              }
            : null
        }
        upstreamAccountActivityLoading={dashboardActivityLoading}
        upstreamAccountActivityError={dashboardActivityError}
        upstreamAccountRecentPreviewLimit={upstreamAccountRecentPreviewLimit}
        onUpstreamAccountActivityEnabledChange={setIncludeUpstreamAccountActivity}
        onUpstreamAccountPolicyChanged={() => {
          void reloadDashboardActivity({ silent: true });
        }}
      />
      <DashboardInvocationDetailDrawer
        open={selectedInvocation != null}
        selection={selectedInvocation}
        onClose={() => setSelectedInvocation(null)}
        onOpenUpstreamAccount={handleOpenUpstreamAccount}
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
        onOpenUpstreamAccount={handleOpenUpstreamAccount}
      />
      {upstreamAccountId != null ? (
        <SharedUpstreamAccountDetailDrawer
          open
          accountId={upstreamAccountId}
          initialTab={upstreamAccountTab}
          onClose={closeUpstreamAccount}
        />
      ) : null}
    </div>
  );
}
