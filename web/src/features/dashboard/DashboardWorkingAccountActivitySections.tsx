import { Chip } from "../../components/ui/chip";
import type { useTranslation } from "../../i18n";
import type { UpstreamAccountActivityAccount } from "../../lib/api";
import type {
  DashboardWorkingConversationInvocationModel,
  DashboardWorkingConversationInvocationSelection,
} from "../../lib/dashboardWorkingConversations";
import {
  compactUpstreamPlanLabel,
  shouldShowUpstreamPlanChip,
  upstreamPlanChipRecipe,
} from "../../lib/upstreamAccountChips";
import { cn } from "../../lib/utils";
import { InvocationPhaseSegments } from "../invocations/InvocationPhaseChip";
import { FALLBACK_CELL } from "../invocations/invocation-details-shared";
import type { DashboardWorkingAccountMetricModel } from "./DashboardWorkingAccountMetricModel";
import {
  AccountHeroMetric,
  AccountInlineMetric,
  AccountSegmentList,
} from "./DashboardWorkingAccountMetricPrimitives";
import {
  ACCOUNT_INLINE_TPM_SPLIT_VALUE_WIDTH_BUDGET_CH,
  AccountAttentionChips,
} from "./DashboardWorkingAccountMetrics";
import { AccountQuickPolicyChips } from "./DashboardWorkingAccountPolicy";
import type { DashboardWorkingAccountPolicyController } from "./DashboardWorkingAccountPolicyController";
import { AccountRecentInvocationRow } from "./DashboardWorkingAccountRecentInvocationRow";
import type { DashboardWorkingConversationSelection } from "./DashboardWorkingConversationsSection";
import {
  ACCOUNT_CARD_INNER_BORDER_CLASS_NAME,
  ACCOUNT_HEADER_BADGE_CLASS_NAME,
  DASHBOARD_RECENT_SKELETON_IDS,
} from "./DashboardWorkingConversationsSection";
import { UsageBreakdownTooltip } from "./UsageBreakdownTooltip";

type Translate = ReturnType<typeof useTranslation>["t"];
type AccountPolicyController = DashboardWorkingAccountPolicyController;
type AccountMetricModel = DashboardWorkingAccountMetricModel;

type AccountActivityIdentityProps = {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  policy: Pick<
    AccountPolicyController,
    | "policyDraft"
    | "isSavingPolicy"
    | "isPolicySaveScheduled"
    | "handleCyclePriorityPolicy"
    | "handleCycleFastModePolicy"
    | "handleToggleCutOut"
    | "handleToggleCutIn"
  >;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  onOpenHealthEventsTab: () => void;
};

function AccountActivityIdentity({
  account,
  locale,
  policy,
  onOpenUpstreamAccount,
  onOpenHealthEventsTab,
}: AccountActivityIdentityProps) {
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-2">
      <button
        type="button"
        data-motion-surface
        disabled={account.upstreamAccountId == null}
        className={cn(
          "inline-flex min-h-11 min-w-0 max-w-full appearance-none items-center border-0 bg-transparent py-1 text-left text-[1rem] font-semibold text-base-content transition-opacity duration-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary sm:min-h-8 sm:py-0",
          account.upstreamAccountId == null ? "cursor-default" : "cursor-pointer hover:opacity-80",
        )}
        onClick={() => {
          if (account.upstreamAccountId == null) return;
          onOpenUpstreamAccount?.(account.upstreamAccountId, account.displayName);
        }}
      >
        <span className="truncate">{account.displayName}</span>
      </button>
      <AccountAttentionChips
        account={account}
        locale={locale}
        clickable={account.upstreamAccountId != null}
        onClick={onOpenHealthEventsTab}
      />
      {shouldShowUpstreamPlanChip(account.planType) ? (
        <Chip
          tone={upstreamPlanChipRecipe(account.planType)?.tone ?? "secondary"}
          data-plan={upstreamPlanChipRecipe(account.planType)?.dataPlan}
          className={ACCOUNT_HEADER_BADGE_CLASS_NAME}
        >
          {compactUpstreamPlanLabel(account.planType)}
        </Chip>
      ) : null}
      <AccountQuickPolicyChipsProxy policy={policy} locale={locale} account={account} />
    </div>
  );
}

function AccountQuickPolicyChipsProxy({
  account,
  locale,
  policy,
}: {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  policy: AccountActivityIdentityProps["policy"];
}) {
  return (
    <AccountQuickPolicyChips
      draft={policy.policyDraft}
      locale={locale}
      disabled={account.upstreamAccountId == null}
      isSaving={policy.isSavingPolicy || policy.isPolicySaveScheduled}
      onCyclePriority={policy.handleCyclePriorityPolicy}
      onCycleFastMode={policy.handleCycleFastModePolicy}
      onToggleCutOut={policy.handleToggleCutOut}
      onToggleCutIn={policy.handleToggleCutIn}
    />
  );
}

function AccountActivityInlineMetrics({
  account,
  layout,
  model,
  t,
}: {
  account: UpstreamAccountActivityAccount;
  layout: "stacked" | "split";
  model: AccountMetricModel;
  t: Translate;
}) {
  if (layout === "stacked") {
    return (
      <div className="flex w-full min-w-0 flex-col gap-y-1.5 text-right self-start">
        <div className="grid w-full min-w-0 grid-cols-3 items-center gap-x-3">
          <div className="min-w-0">
            <AccountInlineMetric
              label={t("dashboard.today.inProgressConversations")}
              value={model.inProgressDisplayValue}
              tone="secondary"
              iconName="send"
              metricKey="in-progress"
              alignment="start"
              fillAvailableWidth
            />
          </div>
          <div className="min-w-0">
            <AccountInlineMetric
              label="TPM"
              value={model.tokensPerMinuteDisplayValue}
              tone="primary"
              iconName="speedometer"
              metricKey="tpm"
              alignment="center"
              fillAvailableWidth
              modelPerformance={account.modelPerformance}
              modelPerformanceTitle={model.modelPerformanceTitle}
            />
          </div>
          <div className="min-w-0">
            <AccountInlineMetric
              label={t("dashboard.today.spendRate")}
              value={model.spendRateDisplayValue}
              tone="warning"
              iconName="cash-clock"
              metricKey="spend-rate"
              alignment="end"
              fillAvailableWidth
              modelPerformance={account.modelPerformance}
              modelPerformanceTitle={model.modelPerformanceTitle}
            />
          </div>
        </div>
      </div>
    );
  }
  return (
    <div className="flex min-w-0 flex-wrap items-center justify-end gap-x-5 gap-y-1.5 text-right self-start">
      <AccountInlineMetric
        label={t("dashboard.today.inProgressConversations")}
        value={model.inProgressDisplayValue}
        tone="secondary"
        iconName="send"
        metricKey="in-progress"
      />
      <AccountInlineMetric
        label="TPM"
        value={model.tokensPerMinuteDisplayValue}
        tone="primary"
        iconName="speedometer"
        metricKey="tpm"
        valueWidthBudgetCh={ACCOUNT_INLINE_TPM_SPLIT_VALUE_WIDTH_BUDGET_CH}
        modelPerformance={account.modelPerformance}
        modelPerformanceTitle={model.modelPerformanceTitle}
      />
      <AccountInlineMetric
        label={t("dashboard.today.spendRate")}
        value={model.spendRateDisplayValue}
        tone="warning"
        iconName="cash-clock"
        metricKey="spend-rate"
        modelPerformance={account.modelPerformance}
        modelPerformanceTitle={model.modelPerformanceTitle}
      />
    </div>
  );
}

export function AccountActivityHeader({
  account,
  locale,
  headerLayout,
  policy,
  model,
  t,
  policySaveError,
  onOpenUpstreamAccount,
  onOpenHealthEventsTab,
}: {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  headerLayout: "stacked" | "split";
  policy: AccountActivityIdentityProps["policy"];
  model: AccountMetricModel;
  t: Translate;
  policySaveError: string | null;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  onOpenHealthEventsTab: () => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <div
        data-testid="dashboard-upstream-account-header-row"
        className={cn(
          "grid items-start gap-x-4 gap-y-2",
          headerLayout === "stacked" ? "grid-cols-1" : "grid-cols-[minmax(0,1fr)_auto]",
        )}
      >
        <AccountActivityIdentity
          account={account}
          locale={locale}
          policy={policy}
          onOpenUpstreamAccount={onOpenUpstreamAccount}
          onOpenHealthEventsTab={onOpenHealthEventsTab}
        />
        <AccountActivityInlineMetrics account={account} layout={headerLayout} model={model} t={t} />
      </div>
      {policySaveError ? (
        <div
          role="alert"
          data-testid="dashboard-upstream-account-policy-error"
          className="inline-flex max-w-full rounded-lg border border-error/30 bg-error/10 px-2.5 py-1 text-xs font-medium text-error"
        >
          {locale === "zh" ? "策略保存失败：" : "Policy save failed: "}
          <span className="truncate">{policySaveError}</span>
        </div>
      ) : null}
    </div>
  );
}

function AccountActivityUsageTooltip({
  account,
  model,
}: {
  account: UpstreamAccountActivityAccount;
  model: AccountMetricModel;
}) {
  return (
    <UsageBreakdownTooltip
      title={model.usageDetailsLabel}
      breakdown={account.usageBreakdown}
      formatNumber={model.formatBreakdownNumber}
      formatRatio={model.formatBreakdownRatio}
      formatCurrency={model.formatBreakdownCurrency}
      labels={model.usageBreakdownLabels}
    />
  );
}

function AccountActivityHeroMetricGrid({
  account,
  locale,
  heroMetricColumnCount,
  model,
  t,
}: {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  heroMetricColumnCount: 1 | 2 | 4;
  model: AccountMetricModel;
  t: Translate;
}) {
  return (
    <div
      className={cn(
        "grid gap-2.5",
        heroMetricColumnCount === 1
          ? "grid-cols-1"
          : heroMetricColumnCount === 2
            ? "grid-cols-2"
            : "grid-cols-4",
      )}
    >
      <AccountHeroMetric
        label={t("dashboard.today.firstResponseTime")}
        value={model.currentFirstByteDisplayValue}
        tone={model.currentFirstByteValue === FALLBACK_CELL ? "neutral" : "secondary"}
        iconName="timer-outline"
        metricKey="latency"
        detailSections={model.latencyDetailSections}
      >
        <AccountSegmentList
          segments={[
            {
              label: t("dashboard.today.responseTime"),
              value: model.currentResponseDurationDisplayValue,
              tone: model.currentResponseDurationValue === FALLBACK_CELL ? "neutral" : "primary",
            },
          ]}
          testId="dashboard-upstream-account-latency-breakdown"
          enableTooltips={false}
        />
      </AccountHeroMetric>
      <AccountHeroMetric
        label={locale === "zh" ? "请求数" : "Requests"}
        value={model.totalRequestDisplayValue}
        tone="neutral"
        iconName="counter"
        metricKey="requests"
        detailSections={model.requestDetailSections}
      >
        <AccountSegmentList
          segments={model.requestSummarySegments}
          testId="dashboard-upstream-account-request-breakdown"
          enableTooltips={false}
        />
      </AccountHeroMetric>
      <AccountHeroMetric
        label={locale === "zh" ? "成本" : "Cost"}
        value={model.totalCostDisplayValue}
        tone="warning"
        iconName="currency-usd"
        metricKey="cost"
        detailSections={model.costDetailSections}
        tooltipContent={<AccountActivityUsageTooltip account={account} model={model} />}
      >
        <AccountSegmentList
          segments={model.costSummarySegments}
          testId="dashboard-upstream-account-cost-breakdown"
          enableTooltips={false}
        />
      </AccountHeroMetric>
      <AccountHeroMetric
        label="Token"
        value={model.totalTokenDisplayValue}
        tone="success"
        iconName="database-outline"
        metricKey="token"
        detailSections={model.tokenDetailSections}
        tooltipContent={<AccountActivityUsageTooltip account={account} model={model} />}
      >
        <AccountSegmentList
          segments={model.tokenSummarySegments}
          testId="dashboard-upstream-account-token-breakdown"
          enableTooltips={false}
        />
      </AccountHeroMetric>
    </div>
  );
}

export function AccountActivityMetrics({
  account,
  locale,
  heroMetricColumnCount,
  model,
  t,
}: {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  heroMetricColumnCount: 1 | 2 | 4;
  model: AccountMetricModel;
  t: Translate;
}) {
  return (
    <div className="mt-4 flex flex-col gap-2.5">
      <AccountActivityHeroMetricGrid
        account={account}
        locale={locale}
        heroMetricColumnCount={heroMetricColumnCount}
        model={model}
        t={t}
      />
    </div>
  );
}

function AccountActivityRecentList({
  locale,
  nowMs,
  recentPreviewLimit,
  recentLoading,
  recentError,
  recentInvocations,
  recentDetailsLayout,
  t,
  onRetryRecent,
  onOpenUpstreamAccount,
  onOpenConversation,
  onOpenInvocation,
}: {
  locale: "zh" | "en";
  nowMs: number;
  recentPreviewLimit: number;
  recentLoading: boolean;
  recentError?: string | null;
  recentInvocations: DashboardWorkingConversationInvocationModel[];
  recentDetailsLayout: "stacked" | "split";
  t: Translate;
  onRetryRecent?: () => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  onOpenConversation?: (selection: DashboardWorkingConversationSelection) => void;
  onOpenInvocation?: (selection: DashboardWorkingConversationInvocationSelection) => void;
}) {
  return (
    <div
      data-testid="dashboard-upstream-account-recent-list"
      className="grid content-start gap-1.5"
      aria-live="polite"
    >
      {recentLoading && recentInvocations.length === 0
        ? DASHBOARD_RECENT_SKELETON_IDS.slice(0, recentPreviewLimit).map((skeletonId) => (
            <div
              key={skeletonId}
              data-testid="dashboard-upstream-account-recent-skeleton"
              className="min-h-12 rounded-xl bg-base-200/65 p-2.5"
            >
              <div className="h-2.5 w-28 animate-pulse rounded bg-base-300/75" />
              <div className="mt-2 h-2 w-3/4 animate-pulse rounded bg-base-300/55" />
            </div>
          ))
        : null}
      {!recentLoading && recentError ? (
        <div className="flex min-h-24 flex-col items-start justify-center gap-2 rounded-xl bg-error/8 px-3 py-2 text-xs text-base-content/72">
          <span>{t("dashboard.upstreamAccounts.recentError")}</span>
          <button
            type="button"
            className="font-semibold text-error underline decoration-error/45 underline-offset-4 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
            onClick={onRetryRecent}
          >
            {t("dashboard.upstreamAccounts.retryRecent")}
          </button>
        </div>
      ) : null}
      {!recentError && (!recentLoading || recentInvocations.length > 0)
        ? recentInvocations.map((invocation) => (
            <AccountRecentInvocationRow
              key={`${invocation.record.invokeId}:${invocation.record.occurredAt}:${invocation.record.id}`}
              invocation={invocation}
              locale={locale}
              nowMs={nowMs}
              detailsLayout={recentDetailsLayout}
              onOpenUpstreamAccount={onOpenUpstreamAccount}
              onOpenConversation={onOpenConversation}
              onOpenInvocation={onOpenInvocation}
            />
          ))
        : null}
    </div>
  );
}

export function AccountActivityRecentSection({
  account,
  locale,
  nowMs,
  recentPreviewLimit,
  recentLoading,
  recentError,
  recentInvocations,
  recentBreakdownLayout,
  recentDetailsLayout,
  recentBridgeSegments,
  t,
  onRetryRecent,
  onOpenUpstreamAccount,
  onOpenConversation,
  onOpenInvocation,
}: {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  nowMs: number;
  recentPreviewLimit: number;
  recentLoading: boolean;
  recentError?: string | null;
  recentInvocations: DashboardWorkingConversationInvocationModel[];
  recentBreakdownLayout: "stacked" | "inline";
  recentDetailsLayout: "stacked" | "split";
  recentBridgeSegments: AccountMetricModel["recentBridgeSegments"];
  t: Translate;
  onRetryRecent?: () => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  onOpenConversation?: (selection: DashboardWorkingConversationSelection) => void;
  onOpenInvocation?: (selection: DashboardWorkingConversationInvocationSelection) => void;
}) {
  return (
    <div
      data-testid="dashboard-upstream-account-recent-section"
      className={cn(
        "mt-3.5 flex flex-1 flex-col border-t pt-2.5",
        ACCOUNT_CARD_INNER_BORDER_CLASS_NAME,
      )}
    >
      <div
        className={cn(
          "mb-2 flex flex-wrap gap-x-3 gap-y-1.5",
          recentBreakdownLayout === "stacked"
            ? "items-start justify-start"
            : "items-center justify-between",
        )}
      >
        <div className="text-xs font-semibold leading-5 tracking-[0.06em] text-base-content/62">
          {t("dashboard.upstreamAccounts.recentInvocations", { count: recentPreviewLimit })}
        </div>
        <div
          className={cn(
            "flex flex-wrap gap-x-3 gap-y-1.5",
            recentBreakdownLayout === "stacked" ? "justify-start" : "items-center justify-end",
          )}
          data-testid="dashboard-upstream-account-recent-breakdown"
        >
          <InvocationPhaseSegments
            counts={account.inProgressPhaseCounts}
            appearance="inline"
            motion="static"
            showLabel={recentBreakdownLayout !== "stacked"}
            className="justify-end"
          />
          {recentBridgeSegments.length > 0 ? (
            <AccountSegmentList
              segments={recentBridgeSegments}
              showLabel={recentBreakdownLayout !== "stacked"}
              showIconWhenLabelHidden
              className="justify-end"
            />
          ) : null}
        </div>
      </div>
      <AccountActivityRecentList
        locale={locale}
        nowMs={nowMs}
        recentPreviewLimit={recentPreviewLimit}
        recentLoading={recentLoading}
        recentError={recentError}
        recentInvocations={recentInvocations}
        recentDetailsLayout={recentDetailsLayout}
        t={t}
        onRetryRecent={onRetryRecent}
        onOpenUpstreamAccount={onOpenUpstreamAccount}
        onOpenConversation={onOpenConversation}
        onOpenInvocation={onOpenInvocation}
      />
    </div>
  );
}
