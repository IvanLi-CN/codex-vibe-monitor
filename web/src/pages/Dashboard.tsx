import { useEffect, useLayoutEffect, useState } from "react";
import { useMatch, useNavigate } from "react-router-dom";
import { DashboardActivityOverview } from "../features/dashboard/DashboardActivityOverview";
import { DashboardInvocationDetailDrawer } from "../features/dashboard/DashboardInvocationDetailDrawer";
import { DashboardPerformanceDiagnostics } from "../features/dashboard/DashboardPerformanceDiagnostics";
import type { DashboardOpenUpstreamAccountOptions } from "../features/dashboard/DashboardWorkingConversationsSection";
import { DashboardWorkingConversationsSection } from "../features/dashboard/DashboardWorkingConversationsSection";
import {
  DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY,
  type DashboardActivityRangeKey,
  persistDashboardActivityRange,
  readPersistedDashboardActivityRange,
} from "../features/dashboard/dashboardActivityRange";
import { PromptCacheConversationHistoryDrawer } from "../features/prompt-cache/PromptCacheConversationTable";
import { useCompactViewport } from "../hooks/useCompactViewport";
import useDashboardOverviewSnapshotRuntime from "../hooks/useDashboardOverviewSnapshotRuntime";
import { useDashboardActivitySnapshot } from "../hooks/useDashboardUpstreamAccountActivity";
import { useDashboardWorkingConversations } from "../hooks/useDashboardWorkingConversations";
import { usePromptCacheConversationRoute } from "../hooks/usePromptCacheConversationRoute";
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
  const navigate = useNavigate();
  const invocationRouteMatch = useMatch("/dashboard/invocations/:invokeId");
  const routeInvokeId = invocationRouteMatch?.params.invokeId;
  const isCompactViewport = useCompactViewport();
  const [activeRange, setActiveRange] = useState<DashboardActivityRangeKey>(() =>
    readPersistedDashboardActivityRange(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY),
  );
  const [selectedInvocation, setSelectedInvocation] =
    useState<DashboardWorkingConversationInvocationSelection | null>(null);
  const [selectedConversation, setSelectedConversation] = useState<{
    key: string;
    label: string | null;
  } | null>(null);
  const [includeUpstreamAccountActivity, setIncludeUpstreamAccountActivity] = useState(false);
  const { upstreamAccountId, upstreamAccountTab, openUpstreamAccount, closeUpstreamAccount } =
    useUpstreamAccountDetailRoute();
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
  const overviewSnapshotRuntime = useDashboardOverviewSnapshotRuntime(activeRange);
  const {
    data: dashboardActivity,
    isLoading: dashboardActivityLoading,
    isRefreshing: dashboardActivityRefreshing,
    recentLoading: dashboardActivityRecentLoading,
    recentError: dashboardActivityRecentError,
    error: dashboardActivityError,
    recentInvocationLimit: upstreamAccountRecentPreviewLimit,
    reload: reloadDashboardActivity,
    retryRecent: retryDashboardActivityRecent,
  } = useDashboardActivitySnapshot(
    activeRange,
    dashboardActivityEnabled,
    true,
    recentPreviewLimit,
    includeUpstreamAccountActivity,
  );

  useEffect(() => {
    if (
      selectedInvocation != null &&
      routeInvokeId != null &&
      selectedInvocation.invocation.record.invokeId !== routeInvokeId
    ) {
      setSelectedInvocation(null);
    }
  }, [routeInvokeId, selectedInvocation]);

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
    persistDashboardActivityRange(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, activeRange);
  }, [activeRange]);

  const handleOpenUpstreamAccount = (
    accountId: number,
    _accountLabel: string,
    options?: DashboardOpenUpstreamAccountOptions,
  ) => {
    setSelectedInvocation(null);
    setSelectedConversation(null);
    if (routeInvokeId != null) {
      const search = new URLSearchParams({
        upstreamAccountId: String(Math.trunc(accountId)),
      });
      if (options?.tab && options.tab !== "overview") {
        search.set("upstreamAccountTab", options.tab);
      }
      navigate({ pathname: "/dashboard", search: `?${search.toString()}` }, { replace: true });
      return;
    }
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
        snapshotStatus={overviewSnapshotRuntime.status}
        snapshotBundle={overviewSnapshotRuntime.bundle}
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
          if (routeInvokeId != null) {
            const search = new URLSearchParams({
              promptCacheConversationKey: selection.promptCacheKey,
            });
            navigate(
              { pathname: "/dashboard", search: `?${search.toString()}` },
              { replace: true },
            );
            return;
          }
          openPromptCacheConversation(selection.promptCacheKey, {
            tab: selection.tab,
            clearUpstreamAccount: true,
          });
        }}
        onOpenInvocation={(selection) => {
          closeUpstreamAccount({ replace: true });
          closePromptCacheConversation({ replace: true });
          setSelectedConversation(null);
          setSelectedInvocation(selection);
          navigate(
            `/dashboard/invocations/${encodeURIComponent(selection.invocation.record.invokeId)}`,
          );
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
        upstreamAccountActivityRefreshing={dashboardActivityRefreshing}
        upstreamAccountActivityError={dashboardActivityError}
        upstreamAccountRecentLoading={dashboardActivityRecentLoading}
        upstreamAccountRecentError={dashboardActivityRecentError}
        onRetryUpstreamAccountRecent={retryDashboardActivityRecent}
        upstreamAccountRecentPreviewLimit={upstreamAccountRecentPreviewLimit}
        onUpstreamAccountActivityEnabledChange={setIncludeUpstreamAccountActivity}
        onUpstreamAccountPolicyChanged={() => {
          reloadDashboardActivity();
        }}
      />
      <DashboardInvocationDetailDrawer
        open={routeInvokeId != null}
        invocationId={routeInvokeId ?? null}
        selection={selectedInvocation}
        onClose={() => {
          setSelectedInvocation(null);
          navigate("/dashboard");
        }}
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
