import type { KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent } from "react";
import { Chip } from "../../components/ui/chip";
import { Spinner } from "../../components/ui/spinner";
import type { useTranslation } from "../../i18n";
import type { DashboardWorkingConversationCardModel } from "../../lib/dashboardWorkingConversations";
import { formatDashboardWorkingConversationSequenceId } from "../../lib/dashboardWorkingConversations";
import { sumInvocationPhaseCounts } from "../../lib/invocationPhase";
import { cn } from "../../lib/utils";
import { InvocationPhaseSegments } from "../invocations/InvocationPhaseChip";
import { AppIcon } from "../shared/AppIcon";
import type { DashboardWorkingConversationsSectionProps } from "./DashboardWorkingConversationsSection";
import {
  type DashboardManualBindingChipMeta,
  InlineInvocationStatus,
  InvocationSlot,
  PlaceholderSlot,
  resolveDashboardManualBindingChipMeta,
  resolveStatusMeta,
  type StatusMeta,
  SummaryMetric,
} from "./DashboardWorkingConversationsSection";

const CARD_CLASS_NAME =
  "relative min-w-0 overflow-hidden rounded-2xl border border-base-300/60 bg-base-100/42 p-3 shadow-[0_18px_44px_rgba(2,6,23,0.15)]";
const DASHBOARD_WORKING_CONVERSATION_ROW_GAP_PX = 16;

type DashboardWorkingConversationTranslation = ReturnType<typeof useTranslation>["t"];

type DashboardWorkingConversationVirtualRow = {
  key: string | number | bigint;
  index: number;
  start: number;
};

interface DashboardWorkingConversationAnchorCardElement extends HTMLElement {
  __dashboardWorkingConversationAnchorKey?: string;
}

interface DashboardWorkingConversationCardProps {
  card: DashboardWorkingConversationCardModel;
  selectionModeEnabled: boolean;
  selectedPromptCacheKeySet: ReadonlySet<string>;
  selectionSummaryLabel: string;
  nowMs: number;
  locale: "zh" | "en";
  numberFormatter: Intl.NumberFormat;
  currencyFormatter: Intl.NumberFormat;
  timestampFormatter: Intl.DateTimeFormat;
  t: DashboardWorkingConversationTranslation;
  onOpenUpstreamAccount?: DashboardWorkingConversationsSectionProps["onOpenUpstreamAccount"];
  onOpenConversation?: DashboardWorkingConversationsSectionProps["onOpenConversation"];
  onOpenInvocation?: DashboardWorkingConversationsSectionProps["onOpenInvocation"];
  handleConversationCardClickCapture: (
    event: ReactMouseEvent<HTMLElement>,
    promptCacheKey: string,
  ) => void;
  handleSelectionCardClick: (event: ReactMouseEvent<HTMLElement>, promptCacheKey: string) => void;
  handleSelectionCardKeyDown: (
    event: ReactKeyboardEvent<HTMLElement>,
    promptCacheKey: string,
  ) => void;
}

type DashboardWorkingConversationCardContentProps = Omit<
  DashboardWorkingConversationCardProps,
  "card"
> & {
  card: DashboardWorkingConversationCardModel;
  displaySequenceId: string;
  currentStatusMeta: StatusMeta;
  currentStatusLabel: string;
  sortAnchorLabel: string;
  manualBindingChipMeta: DashboardManualBindingChipMeta | null;
  manualBindingActionLabel: string | null;
  isCardSelected: boolean;
};

function DashboardWorkingConversationCardTitle({
  card,
  displaySequenceId,
  manualBindingChipMeta,
  manualBindingActionLabel,
  onOpenConversation,
  selectionModeEnabled,
  t,
}: Pick<
  DashboardWorkingConversationCardContentProps,
  | "card"
  | "displaySequenceId"
  | "manualBindingChipMeta"
  | "manualBindingActionLabel"
  | "onOpenConversation"
  | "selectionModeEnabled"
  | "t"
>) {
  const sequenceConversationActionLabel = `${t("dashboard.workingConversations.openConversation")} · ${displaySequenceId} · ${card.promptCacheKey}`;
  return (
    <div className="flex min-w-0 flex-1 items-center gap-2">
      {onOpenConversation && !selectionModeEnabled ? (
        <button
          type="button"
          data-testid="dashboard-working-conversation-sequence-button"
          className="inline-flex shrink-0 cursor-pointer appearance-none items-center whitespace-nowrap border-0 bg-transparent p-0 text-left font-mono text-[0.95rem] font-semibold tracking-[0.08em] text-base-content transition-opacity duration-200 hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
          aria-label={sequenceConversationActionLabel}
          title={sequenceConversationActionLabel}
          onClick={() => {
            onOpenConversation({
              conversationSequenceId: card.conversationSequenceId,
              promptCacheKey: card.promptCacheKey,
            });
          }}
        >
          <span className="block whitespace-nowrap">{displaySequenceId}</span>
        </button>
      ) : (
        <div className="shrink-0 whitespace-nowrap font-mono text-[0.95rem] font-semibold tracking-[0.08em] text-base-content">
          {displaySequenceId}
        </div>
      )}
      {manualBindingChipMeta ? (
        onOpenConversation && !selectionModeEnabled ? (
          <Chip
            asChild
            size="compact"
            tone={manualBindingChipMeta.tone}
            data-testid="dashboard-working-conversation-manual-binding-badge"
            className="min-w-0 max-w-[20rem] shrink cursor-pointer px-2 text-[10.5px] leading-4 transition-opacity duration-200 hover:opacity-80"
            aria-label={manualBindingActionLabel ?? undefined}
            title={manualBindingActionLabel ?? undefined}
          >
            <button
              type="button"
              className="min-w-0 max-w-[20rem] truncate whitespace-nowrap appearance-none text-left"
              onClick={(event) => {
                event.stopPropagation();
                onOpenConversation({
                  conversationSequenceId: card.conversationSequenceId,
                  promptCacheKey: card.promptCacheKey,
                  tab: "settings",
                });
              }}
            >
              <span className="block min-w-0 max-w-[20rem] truncate whitespace-nowrap">
                {manualBindingChipMeta.displayValue}
              </span>
            </button>
          </Chip>
        ) : (
          <Chip
            size="compact"
            tone={manualBindingChipMeta.tone}
            data-testid="dashboard-working-conversation-manual-binding-badge"
            className="min-w-0 max-w-full px-2 text-[10.5px] leading-4"
            title={manualBindingChipMeta.accessibleLabel}
          >
            <span className="min-w-0 max-w-[20rem] truncate whitespace-nowrap">
              {manualBindingChipMeta.displayValue}
            </span>
          </Chip>
        )
      ) : null}
    </div>
  );
}

function DashboardWorkingConversationCardStatus({
  card,
  currentStatusLabel,
  currentStatusMeta,
  sortAnchorLabel,
}: Pick<
  DashboardWorkingConversationCardContentProps,
  "card" | "currentStatusLabel" | "currentStatusMeta" | "sortAnchorLabel"
>) {
  return (
    <div className="flex shrink-0 items-center justify-end gap-2 whitespace-nowrap text-[10px] text-base-content/62">
      <span className="font-mono">{sortAnchorLabel}</span>
      {card.inFlightPhaseCounts && sumInvocationPhaseCounts(card.inFlightPhaseCounts) > 0 ? (
        <span data-testid="dashboard-working-conversation-phase-summary">
          <InvocationPhaseSegments
            counts={card.inFlightPhaseCounts}
            appearance="inline"
            motion="static"
            showLabel={false}
            showZero={false}
            countVisibility="multipleOnly"
          />
        </span>
      ) : card.inFlightPhaseCounts ? (
        <InlineInvocationStatus
          meta={currentStatusMeta}
          label={currentStatusLabel}
          showLabel={false}
        />
      ) : null}
    </div>
  );
}

function DashboardWorkingConversationCardHeading(
  props: Pick<
    DashboardWorkingConversationCardContentProps,
    | "card"
    | "currentStatusLabel"
    | "currentStatusMeta"
    | "displaySequenceId"
    | "manualBindingActionLabel"
    | "manualBindingChipMeta"
    | "onOpenConversation"
    | "selectionModeEnabled"
    | "sortAnchorLabel"
    | "t"
  >,
) {
  return (
    <div className="flex min-w-0 items-center justify-between gap-3">
      <DashboardWorkingConversationCardTitle {...props} />
      <DashboardWorkingConversationCardStatus {...props} />
    </div>
  );
}

function DashboardWorkingConversationCardMetrics({
  card,
  currencyFormatter,
  numberFormatter,
  t,
}: Pick<
  DashboardWorkingConversationCardProps,
  "card" | "currencyFormatter" | "numberFormatter" | "t"
>) {
  return (
    <div className="mt-1.5 sm:mt-2">
      <div className="grid grid-cols-3 gap-1.5">
        <SummaryMetric
          label={t("dashboard.workingConversations.requestCountLabel")}
          value={numberFormatter.format(card.requestCount)}
        />
        <SummaryMetric
          label={t("dashboard.workingConversations.totalTokensLabel")}
          value={numberFormatter.format(card.totalTokens)}
        />
        <SummaryMetric
          label={t("dashboard.workingConversations.totalCostLabel")}
          value={currencyFormatter.format(card.totalCost)}
        />
      </div>
    </div>
  );
}

function DashboardWorkingConversationCardInvocations({
  card,
  locale,
  nowMs,
  onOpenInvocation,
  onOpenUpstreamAccount,
  selectionModeEnabled,
  t,
}: Pick<
  DashboardWorkingConversationCardProps,
  | "card"
  | "locale"
  | "nowMs"
  | "onOpenInvocation"
  | "onOpenUpstreamAccount"
  | "selectionModeEnabled"
  | "t"
>) {
  return (
    <div className="mt-2 space-y-1.5 sm:mt-3 sm:space-y-2">
      <InvocationSlot
        invocation={card.currentInvocation}
        label={t("dashboard.workingConversations.currentInvocation")}
        slotKind="current"
        conversationSequenceId={card.conversationSequenceId}
        promptCacheKey={card.promptCacheKey}
        nowMs={nowMs}
        locale={locale}
        interactionsDisabled={selectionModeEnabled}
        onOpenUpstreamAccount={onOpenUpstreamAccount}
        onOpenInvocation={onOpenInvocation}
      />
      {card.previousInvocation ? (
        <InvocationSlot
          invocation={card.previousInvocation}
          label={t("dashboard.workingConversations.previousInvocation")}
          slotKind="previous"
          conversationSequenceId={card.conversationSequenceId}
          promptCacheKey={card.promptCacheKey}
          nowMs={nowMs}
          locale={locale}
          interactionsDisabled={selectionModeEnabled}
          onOpenUpstreamAccount={onOpenUpstreamAccount}
          onOpenInvocation={onOpenInvocation}
        />
      ) : (
        <PlaceholderSlot slotKind="previous" />
      )}
      {card.earlierInvocation ? (
        <InvocationSlot
          invocation={card.earlierInvocation}
          label={t("dashboard.workingConversations.earlierInvocation")}
          slotKind="earlier"
          conversationSequenceId={card.conversationSequenceId}
          promptCacheKey={card.promptCacheKey}
          nowMs={nowMs}
          locale={locale}
          interactionsDisabled={selectionModeEnabled}
          onOpenUpstreamAccount={onOpenUpstreamAccount}
          onOpenInvocation={onOpenInvocation}
        />
      ) : (
        <PlaceholderSlot slotKind="earlier" />
      )}
    </div>
  );
}

function DashboardWorkingConversationCardFrame(
  props: DashboardWorkingConversationCardContentProps,
) {
  const {
    card,
    selectionModeEnabled,
    isCardSelected,
    selectionSummaryLabel,
    displaySequenceId,
    currentStatusMeta,
    handleConversationCardClickCapture,
    handleSelectionCardClick,
    handleSelectionCardKeyDown,
    ...contentProps
  } = props;
  return (
    <article
      key={card.promptCacheKey}
      ref={(node) => {
        if (!node) return;
        (
          node as DashboardWorkingConversationAnchorCardElement
        ).__dashboardWorkingConversationAnchorKey = card.promptCacheKey;
      }}
      data-testid="dashboard-working-conversation-card"
      data-conversation-sequence-id={displaySequenceId}
      data-selection-mode={selectionModeEnabled ? "true" : "false"}
      data-selected={isCardSelected ? "true" : "false"}
      role={selectionModeEnabled ? "checkbox" : undefined}
      tabIndex={selectionModeEnabled ? 0 : undefined}
      aria-label={
        selectionModeEnabled ? `${selectionSummaryLabel} · ${displaySequenceId}` : undefined
      }
      className={cn(
        CARD_CLASS_NAME,
        currentStatusMeta.cardToneClassName,
        selectionModeEnabled &&
          "cursor-pointer ring-1 ring-white/8 hover:ring-info/30 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-info",
        isCardSelected &&
          "ring-2 ring-info/55 bg-info/10 shadow-[inset_0_1px_0_rgba(255,255,255,0.07),0_22px_34px_rgba(2,6,23,0.24)]",
      )}
      onClickCapture={(event) => handleConversationCardClickCapture(event, card.promptCacheKey)}
      onClick={
        selectionModeEnabled
          ? (event) => handleSelectionCardClick(event, card.promptCacheKey)
          : undefined
      }
      onKeyDown={
        selectionModeEnabled
          ? (event) => handleSelectionCardKeyDown(event, card.promptCacheKey)
          : undefined
      }
    >
      <div className="relative">
        {selectionModeEnabled || isCardSelected ? (
          <div className="absolute right-0 top-0 z-[1] rounded-full border border-base-100/8 bg-base-200/78 p-0.5 shadow-[0_10px_24px_rgba(2,6,23,0.26)] backdrop-blur-md">
            <span
              data-testid="dashboard-working-conversation-selection-indicator"
              className={cn(
                "inline-flex h-7 w-7 items-center justify-center rounded-full border text-[11px] shadow-[inset_0_1px_0_rgba(255,255,255,0.08)]",
                isCardSelected
                  ? "border-info/42 bg-base-100/94 text-info"
                  : "border-base-300/70 bg-base-100/90 text-base-content/45",
              )}
            >
              {isCardSelected ? (
                <AppIcon name="check-bold" className="h-3.5 w-3.5" aria-hidden />
              ) : (
                <span className="h-2.5 w-2.5 rounded-full border border-current/45" />
              )}
            </span>
          </div>
        ) : null}
        <DashboardWorkingConversationCardHeading
          {...contentProps}
          card={card}
          selectionModeEnabled={selectionModeEnabled}
          displaySequenceId={displaySequenceId}
          currentStatusMeta={currentStatusMeta}
        />
        <DashboardWorkingConversationCardMetrics {...contentProps} card={card} />
        <DashboardWorkingConversationCardInvocations
          {...contentProps}
          card={card}
          selectionModeEnabled={selectionModeEnabled}
        />
      </div>
    </article>
  );
}

function DashboardWorkingConversationCard(props: DashboardWorkingConversationCardProps) {
  const { card, selectedPromptCacheKeySet, t, timestampFormatter } = props;
  const isCardSelected = selectedPromptCacheKeySet.has(card.promptCacheKey);
  const currentStatusMeta = resolveStatusMeta(
    card.currentInvocation.tone,
    card.currentInvocation.displayStatus,
  );
  const displaySequenceId = formatDashboardWorkingConversationSequenceId(
    card.conversationSequenceId,
  );
  const currentStatusLabel = currentStatusMeta.labelKey
    ? t(currentStatusMeta.labelKey)
    : (currentStatusMeta.label ?? t("table.status.unknown"));
  const sortAnchorLabel =
    card.sortAnchorEpoch != null ? timestampFormatter.format(new Date(card.sortAnchorEpoch)) : "-";
  const manualBindingChipMeta = resolveDashboardManualBindingChipMeta(card.manualBinding, t);
  const manualBindingActionLabel = manualBindingChipMeta
    ? `${t("dashboard.workingConversations.openConversationSettings")} · ${manualBindingChipMeta.accessibleLabel}`
    : null;
  const contentProps = {
    ...props,
    displaySequenceId,
    currentStatusMeta,
    currentStatusLabel,
    sortAnchorLabel,
    manualBindingChipMeta,
    manualBindingActionLabel,
    isCardSelected,
  } satisfies DashboardWorkingConversationCardContentProps;
  return <DashboardWorkingConversationCardFrame {...contentProps} />;
}

type DashboardWorkingConversationGridCardProps = Omit<
  DashboardWorkingConversationCardProps,
  "card"
>;

interface DashboardWorkingConversationGridProps {
  renderedRows: DashboardWorkingConversationVirtualRow[];
  rows: DashboardWorkingConversationCardModel[][];
  totalSize: number;
  scrollMargin: number;
  columnCount: number;
  isLoadingMore: boolean;
  measureRow: (node: HTMLDivElement | null) => void;
  cardProps: DashboardWorkingConversationGridCardProps;
}

function DashboardWorkingConversationVirtualRow({
  virtualRow,
  rowCards,
  columnCount,
  scrollMargin,
  isLastRow,
  measureRow,
  cardProps,
}: {
  virtualRow: DashboardWorkingConversationVirtualRow;
  rowCards: DashboardWorkingConversationCardModel[];
  columnCount: number;
  scrollMargin: number;
  isLastRow: boolean;
  measureRow: (node: HTMLDivElement | null) => void;
  cardProps: DashboardWorkingConversationGridCardProps;
}) {
  return (
    <div
      key={virtualRow.key}
      ref={measureRow}
      data-testid="dashboard-working-conversations-row"
      data-row-index={virtualRow.index}
      data-index={virtualRow.index}
      style={{
        position: "absolute",
        top: 0,
        left: 0,
        width: "100%",
        transform: `translateY(${virtualRow.start - scrollMargin}px)`,
        paddingBottom: isLastRow ? 0 : `${DASHBOARD_WORKING_CONVERSATION_ROW_GAP_PX}px`,
      }}
    >
      <div
        className="grid grid-cols-1 gap-4 xl:grid-cols-2 2xl:grid-cols-3 desktop1660:grid-cols-4"
        style={{ gridTemplateColumns: `repeat(${columnCount}, minmax(0, 1fr))` }}
      >
        {rowCards.map((card) => (
          <DashboardWorkingConversationCard key={card.promptCacheKey} {...cardProps} card={card} />
        ))}
      </div>
    </div>
  );
}

export function DashboardWorkingConversationGrid({
  renderedRows,
  rows,
  totalSize,
  scrollMargin,
  columnCount,
  isLoadingMore,
  measureRow,
  cardProps,
}: DashboardWorkingConversationGridProps) {
  return (
    <div className="grid grid-cols-1 xl:grid-cols-2 2xl:grid-cols-3 desktop1660:grid-cols-4">
      <div className="col-span-full" style={{ height: `${totalSize}px`, position: "relative" }}>
        {renderedRows.map((virtualRow) => (
          <DashboardWorkingConversationVirtualRow
            key={virtualRow.key}
            virtualRow={virtualRow}
            rowCards={rows[virtualRow.index] ?? []}
            columnCount={columnCount}
            scrollMargin={scrollMargin}
            isLastRow={virtualRow.index === rows.length - 1}
            measureRow={measureRow}
            cardProps={cardProps}
          />
        ))}
      </div>
      {isLoadingMore ? (
        <div className="col-span-full flex items-center justify-center gap-2 py-4 text-sm text-base-content/65">
          <Spinner size="sm" aria-label={cardProps.t("chart.loadingDetailed")} />
          <span>{cardProps.t("chart.loadingDetailed")}</span>
        </div>
      ) : null}
    </div>
  );
}
