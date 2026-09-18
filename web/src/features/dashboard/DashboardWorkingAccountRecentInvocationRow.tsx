import type * as React from "react";
import { type KeyboardEvent as ReactKeyboardEvent, useCallback, useMemo } from "react";
import { InvocationErrorSummary } from "../../components/InvocationErrorSummary";
import { Chip } from "../../components/ui/chip";
import { useTranslation } from "../../i18n";
import type {
  DashboardWorkingConversationInvocationModel,
  DashboardWorkingConversationInvocationSelection,
} from "../../lib/dashboardWorkingConversations";
import {
  formatDashboardWorkingConversationSequenceId,
  hashDashboardWorkingConversationKey,
} from "../../lib/dashboardWorkingConversations";
import { cn } from "../../lib/utils";
import {
  InvocationModelContextCluster,
  InvocationReasoningEffortChip,
} from "../invocations/InvocationModelContextCluster";
import { InvocationPhaseChip } from "../invocations/InvocationPhaseChip";
import {
  buildInvocationDetailViewModel,
  FALLBACK_CELL,
  renderEndpointSummary,
  renderFastIndicator,
} from "../invocations/invocation-details-shared";
import { renderInvocationTransportChip } from "../invocations/invocation-transport-chip";
import { AppIcon } from "../shared/AppIcon";
import { resolveModelIdentityIcon } from "../shared/ModelIdentity";
import {
  buildInvocationSummaryFields,
  invocationCacheHitRate,
  renderInvocationSummaryFields,
} from "./DashboardWorkingAccountMetrics";
import { CompactLatencyPills } from "./DashboardWorkingConversationLatency";
import type { DashboardWorkingConversationSelection } from "./DashboardWorkingConversationsSection";
import {
  ACCOUNT_CARD_INNER_BORDER_CLASS_NAME,
  DashboardImageToolIconChip,
  InlineInvocationStatus,
  renderUpstreamAccountRecentModelDisplay,
  resolveConversationIdentityTone,
  resolveStatusMeta,
  UPSTREAM_ACCOUNT_RECENT_COMPACT_BADGE_CLASS_NAME,
} from "./DashboardWorkingConversationsSection";

type RecentInvocationViewModel = ReturnType<typeof buildInvocationDetailViewModel>;

function useAccountRecentInvocationModel({
  invocation,
  locale,
  nowMs,
  onOpenUpstreamAccount,
}: {
  invocation: DashboardWorkingConversationInvocationModel;
  locale: "zh" | "en";
  nowMs: number;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}) {
  const { t } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag]);
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: "currency",
        currency: "USD",
        minimumFractionDigits: 4,
        maximumFractionDigits: 4,
      }),
    [localeTag],
  );
  const renderAccountValue = useCallback(
    (
      accountLabel: string,
      accountId: number | null,
      accountClickable: boolean,
      className?: string,
    ) => {
      if (!accountClickable || accountId == null) {
        return (
          <span className={cn("truncate", className)} title={accountLabel}>
            {accountLabel}
          </span>
        );
      }
      return (
        <button
          type="button"
          className={cn(
            "pointer-events-auto inline-flex min-w-0 cursor-pointer appearance-none items-center truncate border-0 bg-transparent p-0 text-left font-inherit text-current no-underline transition-opacity duration-200 hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
            className,
          )}
          data-motion-surface
          onClick={(event) => {
            event.stopPropagation();
            onOpenUpstreamAccount?.(accountId, accountLabel);
          }}
          onKeyDown={(event) => event.stopPropagation()}
          title={accountLabel}
        >
          {accountLabel}
        </button>
      );
    },
    [onOpenUpstreamAccount],
  );
  const viewModel = useMemo(
    () =>
      buildInvocationDetailViewModel({
        record: invocation.record,
        normalizedStatus: invocation.displayStatus.trim().toLowerCase(),
        t,
        locale,
        localeTag,
        nowMs,
        numberFormatter,
        currencyFormatter,
        renderAccountValue,
      }),
    [
      currencyFormatter,
      invocation.displayStatus,
      invocation.record,
      locale,
      localeTag,
      nowMs,
      numberFormatter,
      renderAccountValue,
      t,
    ],
  );
  return { t, localeTag, viewModel };
}

function useAccountRecentInvocationPresentation({
  invocation,
  localeTag,
  viewModel,
  t,
}: {
  invocation: DashboardWorkingConversationInvocationModel;
  localeTag: string;
  viewModel: RecentInvocationViewModel;
  t: ReturnType<typeof useTranslation>["t"];
}) {
  const timestampFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(localeTag, {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hour12: false,
      }),
    [localeTag],
  );
  const statusMeta = resolveStatusMeta(invocation.tone, invocation.displayStatus);
  const statusLabel = statusMeta.labelKey
    ? t(statusMeta.labelKey)
    : (statusMeta.label ?? t("table.status.unknown"));
  const occurredAtLabel =
    invocation.occurredAtEpoch != null
      ? timestampFormatter.format(new Date(invocation.occurredAtEpoch))
      : invocation.preview.occurredAt || FALLBACK_CELL;
  const displayPromptCacheKey = invocation.preview.promptCacheKey?.trim() ?? "";
  const displayConversationSequenceId = displayPromptCacheKey
    ? formatDashboardWorkingConversationSequenceId(
        `WC-${hashDashboardWorkingConversationKey(displayPromptCacheKey).slice(0, 6)}`,
      )
    : "";
  const recentSummaryFields = useMemo(
    () =>
      buildInvocationSummaryFields({
        cacheHitRate: invocationCacheHitRate(invocation.record),
        totalTokensValue: viewModel.totalTokensValue,
        cost: invocation.record.cost,
        costValue: viewModel.costValue,
        localeTag,
        testIdPrefix: "dashboard-upstream-account-recent-summary",
      }),
    [invocation.record, localeTag, viewModel.costValue, viewModel.totalTokensValue],
  );
  return {
    statusMeta,
    statusLabel,
    occurredAtShortLabel:
      invocation.occurredAtEpoch != null
        ? timestampFormatter.format(new Date(invocation.occurredAtEpoch))
        : occurredAtLabel,
    displayPromptCacheKey,
    displayConversationSequenceId,
    conversationIdentityTone: displayPromptCacheKey
      ? resolveConversationIdentityTone(displayPromptCacheKey)
      : null,
    recentSummaryFields,
    recentSummaryTitle: `${t("table.column.inputTokens")}: ${viewModel.inputTokensValue} · Cache write: ${viewModel.cacheWriteTokensValue} · ${t("table.column.cacheInputTokens")}: ${viewModel.cacheInputTokensValue} · ${t("table.column.outputTokens")}: ${viewModel.outputTokensValue} · ${t("table.column.totalTokens")}: ${viewModel.totalTokensValue} · ${t("table.column.costUsd")}: ${viewModel.costValue} · ${t("table.details.reasoningTokens")}: ${viewModel.reasoningTokensValue}`,
    invocationActionLabel: `${t("dashboard.workingConversations.openInvocation")} · ${invocation.record.invokeId}`,
    conversationActionLabel: displayPromptCacheKey
      ? `${t("dashboard.workingConversations.openConversation")} · ${displayConversationSequenceId} · ${displayPromptCacheKey}`
      : null,
    fastIndicator: renderFastIndicator(viewModel.fastIndicatorState, t),
    shouldGroupModelContext:
      !viewModel.modelHasMismatch && resolveModelIdentityIcon(viewModel.modelValue) != null,
  };
}

function useAccountRecentInvocationActions({
  invocation,
  displayPromptCacheKey,
  onOpenConversation,
  onOpenInvocation,
}: {
  invocation: DashboardWorkingConversationInvocationModel;
  displayPromptCacheKey: string;
  onOpenConversation?: (selection: DashboardWorkingConversationSelection) => void;
  onOpenInvocation?: (selection: DashboardWorkingConversationInvocationSelection) => void;
}) {
  const handleOpenInvocation = useCallback(() => {
    onOpenInvocation?.({
      slotKind: "current",
      conversationSequenceId: invocation.record.invokeId,
      promptCacheKey:
        invocation.preview.promptCacheKey?.trim() || invocation.record.promptCacheKey?.trim() || "",
      invocation,
    });
  }, [invocation, onOpenInvocation]);
  const handleOpenConversation = useCallback(() => {
    if (!displayPromptCacheKey) return;
    onOpenConversation?.({
      conversationSequenceId: `WC-${hashDashboardWorkingConversationKey(displayPromptCacheKey).slice(0, 6)}`,
      promptCacheKey: displayPromptCacheKey,
    });
  }, [displayPromptCacheKey, onOpenConversation]);
  const handleIdentityChipClick = useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      handleOpenConversation();
    },
    [handleOpenConversation],
  );
  const handleIdentityChipKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      handleOpenConversation();
    },
    [handleOpenConversation],
  );
  return { handleOpenInvocation, handleIdentityChipClick, handleIdentityChipKeyDown };
}

function AccountRecentInvocationHeader({
  invocation,
  viewModel,
  presentation,
  actions,
  t,
  localeTag,
  nowMs,
}: {
  invocation: DashboardWorkingConversationInvocationModel;
  viewModel: RecentInvocationViewModel;
  presentation: ReturnType<typeof useAccountRecentInvocationPresentation>;
  actions: ReturnType<typeof useAccountRecentInvocationActions>;
  t: ReturnType<typeof useTranslation>["t"];
  localeTag: string;
  nowMs: number;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <div
        className="flex min-w-0 items-center gap-1.5"
        data-testid="dashboard-upstream-account-recent-identity"
      >
        {presentation.displayConversationSequenceId ? (
          <>
            <Chip
              asChild
              size="compact"
              tone={presentation.conversationIdentityTone ?? "secondary"}
              data-testid="dashboard-upstream-account-recent-identity-chip"
              className="max-w-[4.8rem] cursor-pointer px-1.5 font-mono text-[10px] font-semibold tracking-[0.04em] transition-opacity duration-200 hover:opacity-80"
              aria-label={presentation.conversationActionLabel ?? undefined}
              title={
                presentation.conversationActionLabel ?? presentation.displayConversationSequenceId
              }
            >
              <button
                type="button"
                className="pointer-events-auto min-w-0 max-w-full truncate whitespace-nowrap appearance-none"
                onClick={actions.handleIdentityChipClick}
                onKeyDown={actions.handleIdentityChipKeyDown}
              >
                {presentation.displayConversationSequenceId}
              </button>
            </Chip>
            <AppIcon
              name="chevron-right"
              className="h-3 w-3 shrink-0 text-base-content/45"
              aria-hidden
            />
          </>
        ) : null}
        <span
          className="truncate font-mono text-[12px] font-semibold text-base-content/88"
          title={invocation.record.invokeId}
        >
          {invocation.record.invokeId}
        </span>
      </div>
      {invocation.livePhase ? (
        <InvocationPhaseChip
          phase={invocation.livePhase}
          appearance="inline"
          motion="dynamic"
          showLabel={false}
        />
      ) : (
        <InlineInvocationStatus
          meta={presentation.statusMeta}
          label={presentation.statusLabel}
          className="pointer-events-auto"
          showLabel={false}
          detail={viewModel.collapsedErrorSummary}
        />
      )}
      {renderInvocationTransportChip(invocation.record, "min-h-5 px-2 py-0.5 text-[9.5px]")}
      {renderEndpointSummary(
        viewModel.endpointDisplay,
        t,
        UPSTREAM_ACCOUNT_RECENT_COMPACT_BADGE_CLASS_NAME,
      )}
      <DashboardImageToolIconChip
        endpointDisplay={viewModel.endpointDisplay}
        imageIntentDisplay={viewModel.imageIntentDisplay}
        t={t}
      />
      {!presentation.shouldGroupModelContext ? presentation.fastIndicator : null}
      <CompactLatencyPills invocation={invocation} nowMs={nowMs} localeTag={localeTag} t={t} />
    </div>
  );
}

function AccountRecentInvocationDetails({
  viewModel,
  presentation,
  detailsLayout,
  t,
}: {
  viewModel: RecentInvocationViewModel;
  presentation: ReturnType<typeof useAccountRecentInvocationPresentation>;
  detailsLayout: "stacked" | "split";
  t: ReturnType<typeof useTranslation>["t"];
}) {
  return (
    <div
      data-testid="dashboard-upstream-account-recent-details-row"
      data-layout={detailsLayout}
      className={cn(
        "grid min-w-0 grid-cols-1 gap-x-3 gap-y-0.5",
        detailsLayout === "split" && "sm:grid-cols-2 sm:items-center",
      )}
    >
      <div
        data-testid="dashboard-upstream-account-recent-meta-line"
        className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[11px] leading-[1.45] text-base-content/72"
      >
        <span>{presentation.occurredAtShortLabel}</span>
        <span className="text-base-content/28">·</span>
        {presentation.shouldGroupModelContext ? (
          <InvocationModelContextCluster
            modelValue={viewModel.modelValue}
            reasoningEffortValue={viewModel.reasoningEffortValue}
            fastIndicatorState={viewModel.fastIndicatorState}
            grouped
            t={t}
            testId="dashboard-upstream-account-recent-model-context"
            modelTestId="dashboard-upstream-account-recent-model"
          />
        ) : (
          <>
            <span className="min-w-0">
              {renderUpstreamAccountRecentModelDisplay(
                viewModel.modelHasMismatch,
                viewModel.modelValue,
                viewModel.requestModelValue,
                viewModel.responseModelValue,
                t,
              )}
            </span>
            {viewModel.reasoningEffortValue !== FALLBACK_CELL ? (
              <>
                <span className="text-base-content/28">·</span>
                <InvocationReasoningEffortChip
                  value={viewModel.reasoningEffortValue}
                  testId="dashboard-working-conversation-reasoning-effort"
                />
              </>
            ) : null}
          </>
        )}
      </div>
      <div
        data-testid="dashboard-upstream-account-recent-summary-line"
        className={cn(
          "flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 font-mono text-[10.5px] leading-[1.45] text-base-content/74",
          detailsLayout === "split" && "sm:justify-end sm:text-right",
        )}
        title={presentation.recentSummaryTitle}
      >
        {renderInvocationSummaryFields(presentation.recentSummaryFields)}
      </div>
    </div>
  );
}

function AccountRecentInvocationSurface({
  invocation,
  viewModel,
  presentation,
  actions,
  detailsLayout,
  t,
  localeTag,
  nowMs,
}: {
  invocation: DashboardWorkingConversationInvocationModel;
  viewModel: RecentInvocationViewModel;
  presentation: ReturnType<typeof useAccountRecentInvocationPresentation>;
  actions: ReturnType<typeof useAccountRecentInvocationActions>;
  detailsLayout: "stacked" | "split";
  t: ReturnType<typeof useTranslation>["t"];
  localeTag: string;
  nowMs: number;
}) {
  return (
    <div
      data-testid="dashboard-upstream-account-recent-row"
      data-motion-surface
      className={cn(
        "relative min-w-0 w-full max-w-full rounded-[0.85rem] border bg-base-100/58 px-3.5 py-2.5 text-left transition-colors duration-200 hover:bg-base-100/72",
        ACCOUNT_CARD_INNER_BORDER_CLASS_NAME,
      )}
    >
      <button
        type="button"
        data-testid="dashboard-upstream-account-recent-row-action"
        aria-label={presentation.invocationActionLabel}
        className="absolute inset-0 z-0 h-full w-full rounded-[0.85rem] appearance-none border-0 bg-transparent p-0 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
        onClick={actions.handleOpenInvocation}
      />
      <div className="relative z-[1] pointer-events-none flex min-w-0 flex-col gap-1.5">
        <AccountRecentInvocationHeader
          invocation={invocation}
          viewModel={viewModel}
          presentation={presentation}
          actions={actions}
          t={t}
          localeTag={localeTag}
          nowMs={nowMs}
        />
        <AccountRecentInvocationDetails
          viewModel={viewModel}
          presentation={presentation}
          detailsLayout={detailsLayout}
          t={t}
        />
      </div>
      {viewModel.collapsedErrorSummary ? (
        <InvocationErrorSummary
          className="pointer-events-auto mt-1 max-w-full"
          textClassName="text-[10px] text-error"
          message={viewModel.collapsedErrorSummary}
        />
      ) : null}
    </div>
  );
}

export function AccountRecentInvocationRow({
  invocation,
  locale,
  nowMs,
  detailsLayout,
  onOpenUpstreamAccount,
  onOpenConversation,
  onOpenInvocation,
}: {
  invocation: DashboardWorkingConversationInvocationModel;
  locale: "zh" | "en";
  nowMs: number;
  detailsLayout: "stacked" | "split";
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  onOpenConversation?: (selection: DashboardWorkingConversationSelection) => void;
  onOpenInvocation?: (selection: DashboardWorkingConversationInvocationSelection) => void;
}) {
  const { t, localeTag, viewModel } = useAccountRecentInvocationModel({
    invocation,
    locale,
    nowMs,
    onOpenUpstreamAccount,
  });
  const presentation = useAccountRecentInvocationPresentation({
    invocation,
    localeTag,
    viewModel,
    t,
  });
  const actions = useAccountRecentInvocationActions({
    invocation,
    displayPromptCacheKey: presentation.displayPromptCacheKey,
    onOpenConversation,
    onOpenInvocation,
  });
  return (
    <AccountRecentInvocationSurface
      invocation={invocation}
      viewModel={viewModel}
      presentation={presentation}
      actions={actions}
      detailsLayout={detailsLayout}
      t={t}
      localeTag={localeTag}
      nowMs={nowMs}
    />
  );
}
