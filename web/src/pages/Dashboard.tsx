import { useEffect, useLayoutEffect, useState } from "react";
import { DashboardActivityOverview } from "../features/dashboard/DashboardActivityOverview";
import { DashboardInvocationDetailDrawer } from "../features/dashboard/DashboardInvocationDetailDrawer";
import {
  DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
  persistDashboardActivityRange,
  readPersistedDashboardActivityRange,
  type DashboardActivityRangeKey,
} from "../features/dashboard/dashboardActivityRange";
import { DashboardPerformanceDiagnostics } from "../features/dashboard/DashboardPerformanceDiagnostics";
import { DashboardWorkingConversationsSection } from "../features/dashboard/DashboardWorkingConversationsSection";
import { PromptCacheConversationHistoryDrawer } from "../features/prompt-cache/PromptCacheConversationTable";
import { useDashboardWorkingConversations } from "../hooks/useDashboardWorkingConversations";
import { useDashboardActivitySnapshot } from "../hooks/useDashboardUpstreamAccountActivity";
import { resetDashboardPerformanceDiagnostics } from "../lib/dashboardPerformanceDiagnostics";
import {
  formatDashboardWorkingConversationSequenceId,
  type DashboardWorkingConversationInvocationSelection,
} from "../lib/dashboardWorkingConversations";
import type {
  DashboardOpenUpstreamAccountOptions,
} from "../features/dashboard/DashboardWorkingConversationsSection";
import { useTranslation } from "../i18n";
import { useCompactViewport } from "../hooks/useCompactViewport";
import { usePromptCacheConversationRoute } from "../hooks/usePromptCacheConversationRoute";
import { useUpstreamAccountDetailRoute } from "../hooks/useUpstreamAccountDetailRoute";
import { SharedUpstreamAccountDetailDrawer } from "./account-pool/UpstreamAccounts";

export default function DashboardPage() {
  const { t } = useTranslation();
  const isCompactViewport = useCompactViewport();
  const [activeRange, setActiveRange] = useState<DashboardActivityRangeKey>(
    () =>
      readPersistedDashboardActivityRange(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY),
  );
  const [selectedInvocation, setSelectedInvocation] =
    useState<DashboardWorkingConversationInvocationSelection | null>(null);
  const [selectedConversation, setSelectedConversation] = useState<{
    key: string;
    label: string | null;
  } | null>(null);
  const [includeUpstreamAccountActivity, setIncludeUpstreamAccountActivity] =
    useState(false);
  const {
    upstreamAccountId,
    upstreamAccountTab,
    openUpstreamAccount,
    closeUpstreamAccount,
  } = useUpstreamAccountDetailRoute();
  const {
    promptCacheConversationKey,
    promptCacheConversationTab,
    openPromptCacheConversation,
    closePromptCacheConversation,
  } = usePromptCacheConversationRoute();
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

  useEffect(() => {
    if (promptCacheConversationKey == null) {
      setSelectedConversation(null);
      return;
    }
    setSelectedConversation((current) =>
      current?.key === promptCacheConversationKey ? current : null,
    );
  }, [promptCacheConversationKey]);

  useLayoutEffect(() => {
    resetDashboardPerformanceDiagnostics();
  }, []);

  useEffect(() => {
    persistDashboardActivityRange(
      DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
      activeRange,
    );
  }, [activeRange]);

  const handleOpenUpstreamAccount = (
    accountId: number,
    _accountLabel: string,
    options?: DashboardOpenUpstreamAccountOptions,
  ) => {
    setSelectedInvocation(null);
    setSelectedConversation(null);
    openUpstreamAccount(accountId, {
      tab: options?.tab,
      clearPromptCacheConversation: true,
    });
  };

  if (isCompactViewport && promptCacheConversationKey != null) {
    return (
      <div className="mx-auto flex w-full max-w-full flex-col gap-6">
        <PromptCacheConversationHistoryDrawer
          open
          presentation="page"
          conversationKey={promptCacheConversationKey}
          conversationLabel={selectedConversation?.label ?? null}
          initialTab={promptCacheConversationTab}
          onTabChange={(tab) =>
            openPromptCacheConversation(promptCacheConversationKey, {
              replace: true,
              tab,
            })
          }
          onClose={() => closePromptCacheConversation()}
          t={t}
          onOpenUpstreamAccount={handleOpenUpstreamAccount}
        />
      </div>
    );
  }

  if (isCompactViewport && upstreamAccountId != null) {
    return (
      <div className="mx-auto flex w-full max-w-full flex-col gap-6">
        <SharedUpstreamAccountDetailDrawer
          open
          presentation="page"
          accountId={upstreamAccountId}
          initialTab={upstreamAccountTab}
          onClose={closeUpstreamAccount}
        />
      </div>
    );
  }

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
          const conversationLabel = formatDashboardWorkingConversationSequenceId(
            selection.conversationSequenceId,
          );
          setSelectedConversation({
            key: selection.promptCacheKey,
            label: conversationLabel,
          });
          openPromptCacheConversation(selection.promptCacheKey, {
            clearUpstreamAccount: true,
          });
        }}
        onOpenInvocation={(selection) => {
          closeUpstreamAccount({ replace: true });
          closePromptCacheConversation({ replace: true });
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
        onUpstreamAccountActivityEnabledChange={
          setIncludeUpstreamAccountActivity
        }
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
        open={promptCacheConversationKey != null && upstreamAccountId == null}
        conversationKey={promptCacheConversationKey}
        conversationLabel={selectedConversation?.label ?? null}
        initialTab={promptCacheConversationTab}
        onTabChange={(tab) => {
          if (promptCacheConversationKey == null) return;
          openPromptCacheConversation(promptCacheConversationKey, {
            replace: true,
            tab,
          });
        }}
        onClose={() => closePromptCacheConversation()}
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
