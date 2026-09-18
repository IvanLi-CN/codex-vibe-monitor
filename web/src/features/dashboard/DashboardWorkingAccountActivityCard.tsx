import { useCallback, useRef } from "react";
import { useTranslation } from "../../i18n";
import type { UpstreamAccountActivityAccount } from "../../lib/api";
import type { RoutingStateVersion } from "../../lib/api/core-foundation";
import type { DashboardWorkingConversationInvocationSelection } from "../../lib/dashboardWorkingConversations";
import { buildDashboardWorkingConversationInvocationModel } from "../../lib/dashboardWorkingConversations";
import {
  buildDashboardWorkingAccountCardLayout,
  useDashboardWorkingAccountCardWidth,
} from "./DashboardWorkingAccountActivityLayout";
import {
  AccountActivityHeader,
  AccountActivityMetrics,
  AccountActivityRecentSection,
} from "./DashboardWorkingAccountActivitySections";
import { buildDashboardWorkingAccountMetricModel } from "./DashboardWorkingAccountMetricModel";
import { useDashboardWorkingAccountPolicyController } from "./DashboardWorkingAccountPolicyController";
import type {
  DashboardOpenUpstreamAccountOptions,
  DashboardWorkingConversationSelection,
} from "./DashboardWorkingConversationsSection";
import { ACCOUNT_CARD_CLASS_NAME } from "./DashboardWorkingConversationsSection";

function buildAccountRecentInvocationModels(account: UpstreamAccountActivityAccount) {
  return account.recentInvocations.map((preview) =>
    buildDashboardWorkingConversationInvocationModel(preview),
  );
}

export function DashboardUpstreamAccountActivityCard({
  account,
  routingStateVersion,
  locale,
  localeTag,
  nowMs,
  recentPreviewLimit,
  onOpenUpstreamAccount,
  onOpenConversation,
  onOpenInvocation,
  onPolicyChanged,
  recentLoading = false,
  recentError,
  onRetryRecent,
}: {
  account: UpstreamAccountActivityAccount;
  routingStateVersion?: RoutingStateVersion | null;
  locale: "zh" | "en";
  localeTag: string;
  nowMs: number;
  recentPreviewLimit: number;
  onOpenUpstreamAccount?: (
    accountId: number,
    accountLabel: string,
    options?: DashboardOpenUpstreamAccountOptions,
  ) => void;
  onOpenConversation?: (selection: DashboardWorkingConversationSelection) => void;
  onOpenInvocation?: (selection: DashboardWorkingConversationInvocationSelection) => void;
  onPolicyChanged?: () => void;
  recentLoading?: boolean;
  recentError?: string | null;
  onRetryRecent?: () => void;
}) {
  const { t } = useTranslation();
  const cardRef = useRef<HTMLElement | null>(null);
  const cardWidth = useDashboardWorkingAccountCardWidth(cardRef);
  const policy = useDashboardWorkingAccountPolicyController({
    account,
    routingStateVersion,
    onPolicyChanged,
  });
  const recentInvocations = buildAccountRecentInvocationModels(account);
  const model = buildDashboardWorkingAccountMetricModel({ account, locale, localeTag, t });
  const handleOpenHealthEventsTab = useCallback(() => {
    if (account.upstreamAccountId == null) return;
    onOpenUpstreamAccount?.(account.upstreamAccountId, account.displayName, {
      tab: "healthEvents",
    });
  }, [account.displayName, account.upstreamAccountId, onOpenUpstreamAccount]);
  const layout = buildDashboardWorkingAccountCardLayout(cardWidth);
  return (
    <article
      ref={cardRef}
      data-testid="dashboard-upstream-account-card"
      data-account-key={account.accountKey ?? account.upstreamAccountId ?? "unassigned"}
      data-header-layout={layout.headerLayout}
      data-inline-metric-layout={layout.inlineMetricLayout}
      data-metric-columns={String(layout.heroMetricColumnCount)}
      data-recent-breakdown-layout={layout.recentBreakdownLayout}
      className={ACCOUNT_CARD_CLASS_NAME}
    >
      <AccountActivityHeader
        account={account}
        locale={locale}
        headerLayout={layout.headerLayout}
        policy={policy}
        model={model}
        t={t}
        policySaveError={policy.policySaveError}
        onOpenUpstreamAccount={onOpenUpstreamAccount}
        onOpenHealthEventsTab={handleOpenHealthEventsTab}
      />
      <AccountActivityMetrics
        account={account}
        locale={locale}
        heroMetricColumnCount={layout.heroMetricColumnCount}
        model={model}
        t={t}
      />
      <AccountActivityRecentSection
        account={account}
        locale={locale}
        nowMs={nowMs}
        recentPreviewLimit={recentPreviewLimit}
        recentLoading={recentLoading}
        recentError={recentError}
        recentInvocations={recentInvocations}
        recentBreakdownLayout={layout.recentBreakdownLayout}
        recentDetailsLayout={layout.recentDetailsLayout}
        recentBridgeSegments={model.recentBridgeSegments}
        t={t}
        onRetryRecent={onRetryRecent}
        onOpenUpstreamAccount={onOpenUpstreamAccount}
        onOpenConversation={onOpenConversation}
        onOpenInvocation={onOpenInvocation}
      />
    </article>
  );
}
