import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Spinner } from "../../components/ui/spinner";
import { useForwardProxyBindingNodes } from "../../hooks/useForwardProxyBindingNodes";
import type { TranslationKey } from "../../i18n";
import {
  type ApiInvocation,
  type ApiInvocationAbnormalResponseBodyPreview,
  type ApiPoolUpstreamRequestAttempt,
  type ForwardProxyBindingNode,
  fetchInvocationPoolAttempts,
} from "../../lib/api";
import {
  type FastIndicatorState,
  formatProxyWeightDelta,
  formatResponseContentEncoding,
  formatServiceTier,
  getFastIndicatorState,
  type InvocationCompactionKind,
  type InvocationEndpointDisplay,
  type InvocationImageIntentDisplay,
  isInvocationPoolAccountRoutingInProgress,
  isPoolRouteMode,
  resolveFirstResponseByteTotalMs,
  resolveInvocationAccountLabel,
  resolveInvocationEndpointDisplay,
  resolveInvocationImageIntentDisplay,
  resolveInvocationModelDisplay,
} from "../../lib/invocation";
import { buildTopicDescriptor, subscribeToTopic } from "../../lib/sse";
import { cn } from "../../lib/utils";
import { AppIcon } from "../shared/AppIcon";
import {
  getReasoningEffortTone,
  REASONING_EFFORT_TONE_CLASSNAMES,
} from "./invocation-table-reasoning";
import { PoolAttemptRecordCard } from "./PoolAttemptRecordCard";
import { StructuredPayloadViewer } from "./StructuredPayloadViewer";

export const FALLBACK_CELL = "—";
export const INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME =
  "invocation-account-routing-in-progress text-primary";

type Translator = (key: TranslationKey, values?: Record<string, string | number>) => string;

export type DetailPanelSize = "compact" | "default";

export interface InvocationDetailViewModel {
  accountLabel: string;
  accountId: number | null;
  accountClickable: boolean;
  accountRoutingInProgress: boolean;
  accountPlanType: string | null;
  proxyDisplayName: string;
  modelValue: string;
  modelHasMismatch: boolean;
  requestModelValue: string;
  responseModelValue: string;
  requestedServiceTierValue: string;
  serviceTierValue: string;
  billingServiceTierValue: string;
  fastIndicatorState: FastIndicatorState;
  costValue: string;
  inputTokensValue: string;
  cacheWriteTokensValue: string;
  cacheInputTokensValue: string;
  outputTokensValue: string;
  outputReasoningBreakdownValue: string;
  reasoningTokensValue: string;
  reasoningEffortValue: string;
  totalTokensValue: string;
  endpointValue: string;
  endpointDisplay: InvocationEndpointDisplay;
  imageIntentDisplay: InvocationImageIntentDisplay;
  errorMessage: string;
  collapsedErrorSummary: string;
  totalLatencyValue: string;
  firstResponseByteTotalValue: string;
  firstByteLatencyValue: string;
  responseContentEncodingValue: string;
  detailNotice: string | null;
  detailPairs: Array<{ key: string; label: string; value: ReactNode }>;
  timingPairs: Array<{ label: string; value: string }>;
}

export interface InvocationPoolAttemptsState {
  attemptsByInvokeId: Record<string, ApiPoolUpstreamRequestAttempt[] | undefined>;
  loadingByInvokeId: Record<string, boolean | undefined>;
  errorByInvokeId: Record<string, string | null | undefined>;
}

export type InvocationAccountValueRenderer = (
  accountLabel: string,
  accountId: number | null,
  accountClickable: boolean,
  className?: string,
) => ReactNode;

interface BuildInvocationDetailViewModelOptions {
  record: ApiInvocation;
  normalizedStatus: string;
  t: Translator;
  locale: string;
  localeTag: string;
  nowMs: number;
  numberFormatter: Intl.NumberFormat;
  currencyFormatter: Intl.NumberFormat;
  renderAccountValue: InvocationAccountValueRenderer;
}

interface InvocationExpandedDetailsProps {
  record: ApiInvocation;
  detailId: string;
  detailPairs: Array<{ key: string; label: string; value: ReactNode }>;
  timingPairs: Array<{ label: string; value: string }>;
  errorMessage: string;
  detailNotice: string | null;
  size: DetailPanelSize;
  poolAttemptsState: InvocationPoolAttemptsState;
  focusedAttemptId?: string | null;
  abnormalResponseBody?: ApiInvocationAbnormalResponseBodyPreview | null;
  abnormalResponseBodyLoading?: boolean;
  abnormalResponseBodyError?: string | null;
  onOpenFullDetails?: (() => void) | null;
  showFullDetailsAction?: boolean;
  t: Translator;
}

type DetailPair = { key: string; label: string; value: ReactNode };

function isZhLocale(locale: string) {
  return locale.trim().toLowerCase().startsWith("zh");
}

export function formatMilliseconds(value: number | null | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return `${value.toFixed(1)} ms`;
}

export function resolveInvocationCollapsedErrorSummary(
  record: Pick<ApiInvocation, "errorMessage" | "downstreamErrorMessage">,
) {
  const canonicalUpstreamError = record.errorMessage?.trim();
  if (canonicalUpstreamError) return canonicalUpstreamError;
  return record.downstreamErrorMessage?.trim() ?? "";
}

export function formatSecondsFromMilliseconds(value: number | null | undefined, localeTag: string) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;

  const seconds = value / 1000;
  const precision = Math.abs(seconds) >= 100 ? 1 : Math.abs(seconds) >= 1 ? 2 : 3;
  const rounded = Number(seconds.toFixed(precision));

  return `${rounded.toLocaleString(localeTag, {
    minimumFractionDigits: 0,
    maximumFractionDigits: precision,
  })} s`;
}

export function formatElapsedSecondsFromTimestamp(
  occurredAt: string | null | undefined,
  localeTag: string,
  nowMs: number,
) {
  const occurredMs = occurredAt ? Date.parse(occurredAt) : Number.NaN;
  if (!Number.isFinite(occurredMs)) return FALLBACK_CELL;
  return formatSecondsFromMilliseconds(Math.max(0, nowMs - occurredMs), localeTag);
}

export function formatOptionalNumber(
  value: number | null | undefined,
  formatter: Intl.NumberFormat,
) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return formatter.format(value);
}

export function formatOptionalText(value: string | null | undefined) {
  const normalized = value?.trim();
  return normalized ? normalized : FALLBACK_CELL;
}

function formatOptionalStatusCode(value: number | null | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return String(Math.trunc(value));
}

export function canOpenInvocationAccount(record: ApiInvocation) {
  return (
    isPoolRouteMode(record.routeMode) &&
    typeof record.upstreamAccountId === "number" &&
    Number.isFinite(record.upstreamAccountId)
  );
}

function normalizeDetailLevel(value: ApiInvocation["detailLevel"]) {
  return value === "structured_only" ? "structured_only" : "full";
}

export function formatDetailTimestamp(value: string | null | undefined) {
  const normalized = value?.trim();
  if (!normalized) return FALLBACK_CELL;

  const parsed = new Date(normalized);
  if (Number.isNaN(parsed.getTime())) return normalized;

  return parsed.toISOString().replace(".000Z", "Z").replace("T", " ");
}

export function renderReasoningEffortBadge(value: string) {
  if (value === FALLBACK_CELL) {
    return <span className="font-mono text-sm text-base-content/70">{FALLBACK_CELL}</span>;
  }

  const tone = getReasoningEffortTone(value);

  return (
    <Badge
      variant="secondary"
      className={cn(
        "max-w-full justify-center overflow-hidden px-2 py-0 text-[10px] font-semibold tracking-[0.01em]",
        REASONING_EFFORT_TONE_CLASSNAMES[tone],
      )}
      title={value}
      data-reasoning-effort-tone={tone}
    >
      <span className="block max-w-full truncate whitespace-nowrap">{value}</span>
    </Badge>
  );
}

export function resolveProxyDisplayName(record: ApiInvocation) {
  const payloadProxyName = record.proxyDisplayName?.trim();
  if (payloadProxyName) return payloadProxyName;
  return FALLBACK_CELL;
}

function compactProxyBindingKey(proxyBindingKey: string) {
  if (proxyBindingKey.length <= 18) return proxyBindingKey;
  return `${proxyBindingKey.slice(0, 8)}...${proxyBindingKey.slice(-6)}`;
}

function collectPoolAttemptProxyBindingKeys(attempts: ApiPoolUpstreamRequestAttempt[] | undefined) {
  if (!attempts?.length) return [];
  return Array.from(
    new Set(
      attempts
        .map((attempt) => attempt.proxyBindingKeySnapshot?.trim() ?? "")
        .filter((key) => key.length > 0 && key !== "__direct__"),
    ),
  ).sort((left, right) => left.localeCompare(right));
}

function buildForwardProxyBindingNodeMap(nodes: ForwardProxyBindingNode[]) {
  const entries = new Map<string, ForwardProxyBindingNode>();
  for (const node of nodes) {
    entries.set(node.key, node);
    for (const aliasKey of node.aliasKeys ?? []) {
      entries.set(aliasKey, node);
    }
  }
  return entries;
}

function formatPoolAttemptProxyBindingDisplay(
  attempt: ApiPoolUpstreamRequestAttempt,
  proxyBindingNodesByKey: Map<string, ForwardProxyBindingNode>,
) {
  const proxyBindingKey = attempt.proxyBindingKeySnapshot?.trim();
  if (!proxyBindingKey) {
    return { value: FALLBACK_CELL, title: FALLBACK_CELL, resolved: false };
  }
  if (proxyBindingKey === "__direct__") {
    return { value: "Direct", title: "Direct", resolved: true };
  }

  const node = proxyBindingNodesByKey.get(proxyBindingKey);
  const displayName = node?.displayName.trim();
  if (displayName && displayName !== proxyBindingKey) {
    return {
      value: displayName,
      title: `${displayName} (${proxyBindingKey})`,
      resolved: true,
    };
  }

  return {
    value: compactProxyBindingKey(proxyBindingKey),
    title: proxyBindingKey,
    resolved: false,
  };
}

export function renderFastIndicator(state: FastIndicatorState, t: Translator) {
  if (state === "none") return null;

  const isEffective = state === "effective";
  const titleKey: TranslationKey = isEffective
    ? "table.model.fastPriorityTitle"
    : "table.model.fastRequestedOnlyTitle";
  const ariaKey: TranslationKey = isEffective
    ? "table.model.fastPriorityAria"
    : "table.model.fastRequestedOnlyAria";

  return (
    <span
      className={cn(
        "inline-flex h-4 w-4 flex-none items-center justify-center",
        isEffective ? "text-amber-500" : "text-base-content/50",
      )}
      title={t(titleKey)}
      aria-label={t(ariaKey)}
      data-testid="invocation-fast-icon"
      data-fast-state={state}
      role="img"
    >
      <AppIcon name="lightning-bolt" className="h-3.5 w-3.5 -translate-y-px" aria-hidden />
    </span>
  );
}

export function renderInvocationModelBadge(
  value: string,
  options: {
    t: Translator;
    hasMismatch?: boolean;
    className?: string;
    textClassName?: string;
    iconClassName?: string;
    title?: string;
    testId?: string;
  },
) {
  const {
    t,
    hasMismatch = false,
    className,
    textClassName,
    iconClassName,
    title,
    testId,
  } = options;
  const mismatchTitle = hasMismatch ? t("table.model.routingMismatchTitle") : null;
  const resolvedTitle = title ?? value;

  return (
    <div
      className={cn("flex min-w-0 items-center gap-1", className)}
      title={resolvedTitle}
      data-testid={testId}
      data-model-routed={hasMismatch ? "true" : "false"}
    >
      {hasMismatch ? (
        <span
          className="inline-flex h-4 w-4 flex-none items-center justify-center text-base-content/55"
          title={mismatchTitle ?? undefined}
          aria-label={t("table.model.routingMismatchAria")}
          data-testid={
            testId ? `${testId}-routing-indicator` : "invocation-model-routing-indicator"
          }
          role="img"
        >
          <AppIcon
            name="compare-horizontal"
            className={cn("h-3.5 w-3.5", iconClassName)}
            aria-hidden
          />
        </span>
      ) : null}
      <span className={cn("min-w-0 max-w-full truncate leading-none", textClassName)}>{value}</span>
    </div>
  );
}

export function renderInvocationModelRoutingSummary({
  requestModelValue,
  responseModelValue,
  hasMismatch,
  t,
  adornments,
  className,
  indicatorTestId,
}: {
  requestModelValue: string;
  responseModelValue: string;
  hasMismatch: boolean;
  t: Translator;
  adornments?: ReactNode;
  className?: string;
  indicatorTestId?: string;
}) {
  if (!hasMismatch) return null;

  return (
    <div
      role="img"
      className={cn("min-w-0", className)}
      data-testid="invocation-model-route-summary"
      title={`${t("table.details.requestModel")}: ${requestModelValue} · ${t(
        "table.details.responseModel",
      )}: ${responseModelValue}`}
      aria-label={`${t("table.model.routingMismatchAria")}: ${requestModelValue} -> ${responseModelValue}`}
    >
      <div className="flex min-w-0 items-center justify-between gap-2">
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <span
            className="inline-flex h-4 w-4 flex-none items-center justify-center text-base-content/55"
            title={t("table.model.routingMismatchTitle")}
            data-testid={indicatorTestId}
            aria-hidden
          >
            <AppIcon name="compare-horizontal" className="h-3.5 w-3.5" />
          </span>
          <span
            className="min-w-0 truncate font-mono text-sm font-medium leading-6 text-base-content/58 line-through decoration-base-content/30 decoration-1"
            title={requestModelValue}
          >
            {requestModelValue}
          </span>
          <span
            className="inline-flex h-5 w-5 flex-none items-center justify-center rounded-full bg-warning/12 text-warning"
            aria-hidden
          >
            <AppIcon name="arrow-right-bold" className="h-3.5 w-3.5" />
          </span>
          <span
            className="min-w-0 truncate font-mono text-sm font-semibold leading-6 text-base-content/95"
            title={responseModelValue}
          >
            {responseModelValue}
          </span>
        </div>
        {adornments ? <div className="flex flex-none items-center gap-1">{adornments}</div> : null}
      </div>
    </div>
  );
}

function renderEndpointRawPath(endpointValue: string, className?: string) {
  return (
    <span
      className={cn("block max-w-full break-all font-mono text-base-content/70", className)}
      title={endpointValue}
      data-testid="invocation-endpoint-path"
      data-endpoint-kind="raw"
    >
      {endpointValue}
    </span>
  );
}

export function renderEndpointSummary(
  endpointDisplay: InvocationEndpointDisplay,
  t: Translator,
  className?: string,
) {
  if (
    endpointDisplay.kind === "raw" ||
    endpointDisplay.labelKey == null ||
    endpointDisplay.badgeVariant == null
  ) {
    return renderEndpointRawPath(endpointDisplay.endpointValue, className);
  }

  const title =
    endpointDisplay.kind === "compact"
      ? `${t("table.endpoint.compactHint")} · ${endpointDisplay.endpointValue}`
      : endpointDisplay.endpointValue;

  return (
    <Badge
      variant={endpointDisplay.badgeVariant}
      className={cn(
        "invocation-endpoint-badge max-w-full justify-center overflow-hidden px-2 py-0 text-[10px] font-semibold tracking-[0.01em]",
        className,
      )}
      title={title}
      data-testid="invocation-endpoint-badge"
      data-endpoint-kind={endpointDisplay.kind}
    >
      <span className="block max-w-full truncate whitespace-nowrap">
        {t(endpointDisplay.labelKey)}
      </span>
    </Badge>
  );
}

function formatFailureClassValue(
  failureClass: ApiInvocation["failureClass"],
  t: Translator,
): ReactNode {
  switch (failureClass) {
    case "service_failure":
      return <Badge variant="error">{t("records.filters.failureClass.service")}</Badge>;
    case "client_failure":
      return <Badge variant="warning">{t("records.filters.failureClass.client")}</Badge>;
    case "client_abort":
      return <Badge variant="secondary">{t("records.filters.failureClass.abort")}</Badge>;
    default:
      return FALLBACK_CELL;
  }
}

function formatActionableValue(value: ApiInvocation["isActionable"], t: Translator) {
  if (typeof value !== "boolean") return FALLBACK_CELL;
  return (
    <Badge variant={value ? "warning" : "secondary"}>
      {value ? t("table.details.actionableYes") : t("table.details.actionableNo")}
    </Badge>
  );
}

function resolveDetailLabels(locale: string) {
  if (isZhLocale(locale)) {
    return {
      full: "Full",
      structuredOnly: "Structured only",
      level: "细节层级",
      prunedAt: "精简时间",
      pruneReason: "精简原因",
      fullHint: "完整调试细节仍在当前在线保留窗口内。",
      structuredHint: "该记录仅保留结构化字段；离线归档保留归档行，超窗 raw file 不保证继续可用。",
      prunedPrefix: "精简于",
    };
  }

  return {
    full: "Full",
    structuredOnly: "Structured only",
    level: "Detail level",
    prunedAt: "Detail pruned at",
    pruneReason: "Detail prune reason",
    fullHint: "Full troubleshooting detail is still available inside the online retention window.",
    structuredHint:
      "Only structured fields remain online for this record. Offline archives keep the archived row, but aged raw files may no longer be available.",
    prunedPrefix: "Pruned at",
  };
}

function renderDetailEndpointValue(
  endpointDisplay: InvocationEndpointDisplay,
  endpointValue: string,
  t: Translator,
) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <div className="w-fit max-w-full">{renderEndpointSummary(endpointDisplay, t)}</div>
      <span className="break-all font-mono text-xs text-base-content/70">{endpointValue}</span>
    </div>
  );
}

function renderCompactionKindValue(
  value: InvocationCompactionKind | null | undefined,
  t: Translator,
) {
  if (value === "compact") {
    return <Badge variant="info">{t("table.endpoint.compactBadge")}</Badge>;
  }
  if (value === "remote_v2") {
    return <Badge variant="info">{t("table.endpoint.remoteV2Badge")}</Badge>;
  }
  return FALLBACK_CELL;
}

export function renderImageIntentBadge(
  imageIntentDisplay: InvocationImageIntentDisplay,
  t: Translator,
  className?: string,
) {
  if (
    !imageIntentDisplay.showsBadge ||
    imageIntentDisplay.badgeVariant == null ||
    imageIntentDisplay.badgeLabelKey == null
  ) {
    return null;
  }

  return (
    <Badge
      variant={imageIntentDisplay.badgeVariant}
      className={cn(
        "max-w-full justify-center overflow-hidden px-2 py-0 text-[10px] font-semibold tracking-[0.01em]",
        className,
      )}
      data-testid="invocation-image-tool-badge"
      data-image-intent-kind={imageIntentDisplay.kind}
    >
      <span className="block max-w-full truncate whitespace-nowrap">
        {t(imageIntentDisplay.badgeLabelKey)}
      </span>
    </Badge>
  );
}

function renderImageIntentValue(imageIntentDisplay: InvocationImageIntentDisplay, t: Translator) {
  if (imageIntentDisplay.detailLabelKey == null) {
    return FALLBACK_CELL;
  }

  return imageIntentDisplay.showsBadge ? (
    <div className="flex min-w-0 flex-wrap items-center gap-2">
      {renderImageIntentBadge(imageIntentDisplay, t)}
      <span className="font-mono text-xs text-base-content/70">
        {t(imageIntentDisplay.detailLabelKey)}
      </span>
    </div>
  ) : (
    <span className="font-mono text-xs text-base-content/70">
      {t(imageIntentDisplay.detailLabelKey)}
    </span>
  );
}

export function buildInvocationDetailViewModel({
  record,
  normalizedStatus,
  t,
  locale,
  localeTag,
  nowMs,
  numberFormatter,
  currencyFormatter,
  renderAccountValue,
}: BuildInvocationDetailViewModelOptions): InvocationDetailViewModel {
  const proxyDisplayName = resolveProxyDisplayName(record);
  const modelDisplay = resolveInvocationModelDisplay(record);
  const legacyModelValue = record.model?.trim() || modelDisplay.primaryValue || FALLBACK_CELL;
  const requestModelValue = modelDisplay.requestValue ?? FALLBACK_CELL;
  const responseModelValue = modelDisplay.responseValue ?? legacyModelValue ?? FALLBACK_CELL;
  const accountLabel = resolveInvocationAccountLabel(
    record.routeMode,
    normalizedStatus,
    record.failureKind,
    record.errorMessage,
    record.upstreamAccountName,
    record.upstreamAccountId,
    t("table.account.reverseProxy"),
    t("table.account.poolRoutingPending"),
    t("table.account.poolAccountUnknown"),
    t("table.account.poolAccountUnavailable"),
  );
  const accountClickable = canOpenInvocationAccount(record);
  const accountRoutingInProgress = isInvocationPoolAccountRoutingInProgress(
    record.routeMode,
    normalizedStatus,
    record.upstreamAccountName,
    record.upstreamAccountId,
  );
  const accountValueClassName = cn(
    "font-mono text-sm",
    accountRoutingInProgress && INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME,
  );
  const requestedServiceTierValue = formatServiceTier(record.requestedServiceTier);
  const serviceTierValue = formatServiceTier(record.serviceTier);
  const billingServiceTierValue = formatServiceTier(
    record.billingServiceTier,
    t("table.details.billingServiceTierUnresolved"),
  );
  const fastIndicatorState = getFastIndicatorState(
    record.requestedServiceTier,
    record.serviceTier,
    record.billingServiceTier,
  );
  const reasoningEffortValue = formatOptionalText(record.reasoningEffort);
  const reasoningTokensValue = formatOptionalNumber(record.reasoningTokens, numberFormatter);
  const cacheWriteTokensValue = formatOptionalNumber(
    record.cacheWriteTokens ??
      (record.inputTokens == null
        ? undefined
        : Math.max(0, record.inputTokens - (record.cacheInputTokens ?? 0))),
    numberFormatter,
  );
  const outputReasoningBreakdownValue = `${t("table.column.reasoningTokensShort")} ${reasoningTokensValue}`;
  const totalLatencyValue =
    normalizedStatus === "running" || normalizedStatus === "pending"
      ? formatElapsedSecondsFromTimestamp(record.occurredAt, localeTag, nowMs)
      : formatSecondsFromMilliseconds(record.tTotalMs, localeTag);
  const firstResponseByteTotalValue = formatSecondsFromMilliseconds(
    resolveFirstResponseByteTotalMs(record),
    localeTag,
  );
  const firstByteLatencyValue = formatMilliseconds(record.tUpstreamTtfbMs);
  const responseContentEncodingValue = formatResponseContentEncoding(
    record.responseContentEncoding,
  );
  const endpointDisplay = resolveInvocationEndpointDisplay(record);
  const imageIntentDisplay = resolveInvocationImageIntentDisplay(record);
  const endpointValue = endpointDisplay.endpointValue;
  const errorMessage = record.errorMessage?.trim() ?? "";
  const collapsedErrorSummary = resolveInvocationCollapsedErrorSummary(record);

  const proxyWeightDeltaView = formatProxyWeightDelta(record.proxyWeightDelta);
  const proxyWeightDeltaValue =
    proxyWeightDeltaView.direction === "missing" ? (
      FALLBACK_CELL
    ) : (
      <span
        role="img"
        className={`inline-flex items-center gap-1 font-mono ${
          proxyWeightDeltaView.direction === "up"
            ? "text-success"
            : proxyWeightDeltaView.direction === "down"
              ? "text-error"
              : "text-base-content/70"
        }`}
        aria-label={
          proxyWeightDeltaView.direction === "up"
            ? t("table.details.proxyWeightDeltaA11yIncrease", {
                value: proxyWeightDeltaView.value,
              })
            : proxyWeightDeltaView.direction === "down"
              ? t("table.details.proxyWeightDeltaA11yDecrease", {
                  value: proxyWeightDeltaView.value,
                })
              : t("table.details.proxyWeightDeltaA11yUnchanged", {
                  value: proxyWeightDeltaView.value,
                })
        }
      >
        <AppIcon
          name={
            proxyWeightDeltaView.direction === "up"
              ? "arrow-up-bold"
              : proxyWeightDeltaView.direction === "down"
                ? "arrow-down-bold"
                : "arrow-right-bold"
          }
          className="h-3.5 w-3.5"
          aria-hidden
        />
        <span aria-hidden>{proxyWeightDeltaView.value}</span>
      </span>
    );

  const detailLabels = resolveDetailLabels(locale);
  const detailLevel = normalizeDetailLevel(record.detailLevel);
  const detailPrunedAtValue = formatDetailTimestamp(record.detailPrunedAt);
  const detailPruneReasonValue = formatOptionalText(record.detailPruneReason);
  const detailLevelBadgeLabel =
    detailLevel === "structured_only" ? detailLabels.structuredOnly : detailLabels.full;
  const detailLevelBadgeVariant = detailLevel === "structured_only" ? "warning" : "secondary";
  const detailNotice = detailLevel === "structured_only" ? detailLabels.structuredHint : null;
  const detailPrunedSummary =
    detailLevel === "structured_only" && detailPrunedAtValue !== FALLBACK_CELL
      ? `${detailLabels.prunedPrefix} ${detailPrunedAtValue}`
      : null;
  const detailLevelTooltip =
    detailLevel === "structured_only"
      ? detailPrunedSummary
        ? `${detailLabels.structuredHint} ${detailPrunedSummary}.`
        : detailLabels.structuredHint
      : detailLabels.fullHint;

  const detailPairs: Array<{ key: string; label: string; value: ReactNode }> = [
    {
      key: "invokeId",
      label: t("table.details.invokeId"),
      value: record.invokeId || FALLBACK_CELL,
    },
    {
      key: "account",
      label: t("table.details.account"),
      value: renderAccountValue(
        accountLabel,
        record.upstreamAccountId ?? null,
        accountClickable,
        accountValueClassName,
      ),
    },
    { key: "proxy", label: t("table.details.proxy"), value: proxyDisplayName },
    {
      key: "endpoint",
      label: t("table.details.endpoint"),
      value: renderDetailEndpointValue(endpointDisplay, endpointValue, t),
    },
    {
      key: "requestModel",
      label: t("table.details.requestModel"),
      value: requestModelValue,
    },
    {
      key: "responseModel",
      label: t("table.details.responseModel"),
      value: renderInvocationModelBadge(responseModelValue, {
        t,
        hasMismatch: modelDisplay.hasMismatch,
        title: responseModelValue,
        textClassName: "font-mono text-xs text-base-content/70",
      }),
    },
    {
      key: "compactionRequest",
      label: t("table.details.compactionRequest"),
      value: renderCompactionKindValue(record.compactionRequestKind, t),
    },
    {
      key: "compactionResponse",
      label: t("table.details.compactionResponse"),
      value: renderCompactionKindValue(record.compactionResponseKind, t),
    },
    {
      key: "imageIntent",
      label: t("table.details.imageTool"),
      value: renderImageIntentValue(imageIntentDisplay, t),
    },
    {
      key: "requesterIp",
      label: t("table.details.requesterIp"),
      value: record.requesterIp || FALLBACK_CELL,
    },
    {
      key: "promptCacheKey",
      label: t("table.details.promptCacheKey"),
      value: record.promptCacheKey || FALLBACK_CELL,
    },
    {
      key: "poolAttemptCount",
      label: t("table.details.poolAttemptCount"),
      value: formatOptionalText(
        record.poolAttemptCount != null ? String(record.poolAttemptCount) : undefined,
      ),
    },
    {
      key: "poolDistinctAccountCount",
      label: t("table.details.poolDistinctAccountCount"),
      value: formatOptionalText(
        record.poolDistinctAccountCount != null
          ? String(record.poolDistinctAccountCount)
          : undefined,
      ),
    },
    {
      key: "poolAttemptTerminalReason",
      label: t("table.details.poolAttemptTerminalReason"),
      value: formatOptionalText(record.poolAttemptTerminalReason),
    },
    {
      key: "responseContentEncoding",
      label: t("table.details.httpCompression"),
      value: responseContentEncodingValue,
    },
    {
      key: "requestedServiceTier",
      label: t("table.details.requestedServiceTier"),
      value: requestedServiceTierValue,
    },
    {
      key: "serviceTier",
      label: t("table.details.serviceTier"),
      value: serviceTierValue,
    },
    {
      key: "billingServiceTier",
      label: t("table.details.billingServiceTier"),
      value: billingServiceTierValue,
    },
    {
      key: "reasoningEffort",
      label: t("table.details.reasoningEffort"),
      value: renderReasoningEffortBadge(reasoningEffortValue),
    },
    {
      key: "reasoningTokens",
      label: t("table.details.reasoningTokens"),
      value: reasoningTokensValue,
    },
    {
      key: "proxyWeightDelta",
      label: t("table.details.proxyWeightDelta"),
      value: proxyWeightDeltaValue,
    },
    {
      key: "failureClass",
      label: t("table.details.failureClass"),
      value: formatFailureClassValue(record.failureClass, t),
    },
    {
      key: "actionable",
      label: t("table.details.actionable"),
      value: formatActionableValue(record.isActionable, t),
    },
    {
      key: "failureKind",
      label: t("table.details.failureKind"),
      value: formatOptionalText(record.failureKind),
    },
    {
      key: "streamTerminalEvent",
      label: t("table.details.streamTerminalEvent"),
      value: formatOptionalText(record.streamTerminalEvent),
    },
    {
      key: "upstreamErrorCode",
      label: t("table.details.upstreamErrorCode"),
      value: formatOptionalText(record.upstreamErrorCode),
    },
    {
      key: "upstreamErrorMessage",
      label: t("table.details.upstreamErrorMessage"),
      value: formatOptionalText(record.upstreamErrorMessage),
    },
    {
      key: "downstreamStatusCode",
      label: t("table.details.downstreamStatusCode"),
      value: formatOptionalStatusCode(record.downstreamStatusCode),
    },
    {
      key: "upstreamRequestId",
      label: t("table.details.upstreamRequestId"),
      value: formatOptionalText(record.upstreamRequestId),
    },
    {
      key: "detailLevel",
      label: detailLabels.level,
      value: (
        <Badge
          variant={detailLevelBadgeVariant}
          className="max-w-full justify-center overflow-hidden px-2 py-0 text-[10px] font-semibold tracking-[0.01em]"
          title={detailLevelTooltip}
          data-testid="invocation-detail-level-badge"
        >
          <span className="block max-w-full truncate whitespace-nowrap">
            {detailLevelBadgeLabel}
          </span>
        </Badge>
      ),
    },
    {
      key: "detailPrunedAt",
      label: detailLabels.prunedAt,
      value: detailPrunedAtValue,
    },
    {
      key: "detailPruneReason",
      label: detailLabels.pruneReason,
      value: detailPruneReasonValue,
    },
  ];

  const timingPairs: Array<{ label: string; value: string }> = [
    {
      label: t("table.details.stage.requestRead"),
      value: formatSecondsFromMilliseconds(record.tReqReadMs, localeTag),
    },
    {
      label: t("table.details.stage.requestParse"),
      value: formatSecondsFromMilliseconds(record.tReqParseMs, localeTag),
    },
    {
      label: t("table.details.stage.upstreamConnect"),
      value: formatSecondsFromMilliseconds(record.tUpstreamConnectMs, localeTag),
    },
    {
      label: t("table.details.stage.upstreamFirstByte"),
      value: formatMilliseconds(record.tUpstreamTtfbMs),
    },
    {
      label: t("table.details.stage.upstreamStream"),
      value: formatSecondsFromMilliseconds(record.tUpstreamStreamMs, localeTag),
    },
    {
      label: t("table.details.stage.responseParse"),
      value: formatSecondsFromMilliseconds(record.tRespParseMs, localeTag),
    },
    {
      label: t("table.details.stage.persistence"),
      value: formatSecondsFromMilliseconds(record.tPersistMs, localeTag),
    },
    {
      label: t("table.details.stage.total"),
      value: formatSecondsFromMilliseconds(record.tTotalMs, localeTag),
    },
  ];

  return {
    accountLabel,
    accountId:
      typeof record.upstreamAccountId === "number" && Number.isFinite(record.upstreamAccountId)
        ? Math.trunc(record.upstreamAccountId)
        : null,
    accountClickable,
    accountRoutingInProgress,
    accountPlanType: record.upstreamAccountPlanType?.trim() || null,
    proxyDisplayName,
    modelValue: modelDisplay.primaryValue,
    modelHasMismatch: modelDisplay.hasMismatch,
    requestModelValue,
    responseModelValue,
    requestedServiceTierValue,
    serviceTierValue,
    billingServiceTierValue,
    fastIndicatorState,
    costValue:
      typeof record.cost === "number" ? currencyFormatter.format(record.cost) : FALLBACK_CELL,
    inputTokensValue: formatOptionalNumber(record.inputTokens, numberFormatter),
    cacheWriteTokensValue,
    cacheInputTokensValue: formatOptionalNumber(record.cacheInputTokens, numberFormatter),
    outputTokensValue: formatOptionalNumber(record.outputTokens, numberFormatter),
    outputReasoningBreakdownValue,
    reasoningTokensValue,
    reasoningEffortValue,
    totalTokensValue: formatOptionalNumber(record.totalTokens, numberFormatter),
    endpointValue,
    endpointDisplay,
    imageIntentDisplay,
    errorMessage,
    collapsedErrorSummary,
    totalLatencyValue,
    firstResponseByteTotalValue,
    firstByteLatencyValue,
    responseContentEncodingValue,
    detailNotice,
    detailPairs,
    timingPairs,
  };
}

function formatPoolAttemptAccountLabel(attempt: ApiPoolUpstreamRequestAttempt) {
  const accountName = attempt.upstreamAccountName?.trim();
  if (accountName) return accountName;
  if (typeof attempt.upstreamAccountId === "number" && Number.isFinite(attempt.upstreamAccountId)) {
    return `#${Math.trunc(attempt.upstreamAccountId)}`;
  }
  return FALLBACK_CELL;
}

function poolAttemptStatusMeta(status: string | null | undefined): {
  variant: "success" | "warning" | "error" | "secondary";
  key: TranslationKey;
} {
  switch (status?.trim().toLowerCase()) {
    case "pending":
      return { variant: "warning", key: "table.poolAttempts.status.pending" };
    case "success":
      return { variant: "success", key: "table.poolAttempts.status.success" };
    case "http_failure":
      return { variant: "error", key: "table.poolAttempts.status.httpFailure" };
    case "transport_failure":
      return {
        variant: "warning",
        key: "table.poolAttempts.status.transportFailure",
      };
    case "budget_exhausted_final":
      return {
        variant: "warning",
        key: "table.poolAttempts.status.budgetExhaustedFinal",
      };
    default:
      return { variant: "secondary", key: "table.poolAttempts.status.unknown" };
  }
}

function resolvePoolAttemptPhase(attempt: ApiPoolUpstreamRequestAttempt) {
  const explicitPhase = attempt.phase?.trim().toLowerCase();
  if (explicitPhase) return explicitPhase;

  const normalizedStatus = attempt.status?.trim().toLowerCase();
  if (normalizedStatus === "pending") return "sending_request";
  if (normalizedStatus === "success") return "completed";
  return "failed";
}

function isPoolAttemptTerminal(attempt: ApiPoolUpstreamRequestAttempt) {
  if (attempt.finishedAt?.trim()) return true;
  return attempt.status.trim().toLowerCase() !== "pending";
}

function isSyntheticPoolTerminalAttempt(attempt: ApiPoolUpstreamRequestAttempt) {
  const normalizedStatus = attempt.status.trim().toLowerCase();
  return normalizedStatus === "budget_exhausted_final" || attempt.sameAccountRetryIndex <= 0;
}

function poolAttemptTerminalDescriptionKey(
  terminalReason: string | null | undefined,
): TranslationKey {
  return terminalReason?.trim().toLowerCase() === "max_distinct_accounts_exhausted"
    ? "table.poolAttempts.terminal.budgetExhaustedDescription"
    : "table.poolAttempts.terminal.genericDescription";
}

function isInvocationDisplayTerminal(status: string | null | undefined) {
  const normalized = status?.trim().toLowerCase();
  return Boolean(normalized && normalized !== "running" && normalized !== "pending");
}

function poolAttemptCompletenessScore(attempt: ApiPoolUpstreamRequestAttempt) {
  let score = 0;
  if (attempt.finishedAt?.trim()) score += 8;
  if (attempt.httpStatus != null) score += 2;
  if (attempt.downstreamHttpStatus != null) score += 1;
  if (attempt.failureKind?.trim()) score += 1;
  if (attempt.errorMessage?.trim()) score += 1;
  if (attempt.downstreamErrorMessage?.trim()) score += 1;
  if (attempt.connectLatencyMs != null) score += 1;
  if (attempt.firstByteLatencyMs != null) score += 1;
  if (attempt.streamLatencyMs != null) score += 1;
  if (attempt.upstreamRequestId?.trim()) score += 1;
  return score;
}

function poolAttemptPhaseRank(attempt: ApiPoolUpstreamRequestAttempt) {
  switch (resolvePoolAttemptPhase(attempt)) {
    case "connecting":
      return 0;
    case "sending_request":
      return 1;
    case "waiting_first_byte":
      return 2;
    case "streaming_response":
      return 3;
    case "completed":
    case "failed":
      return 4;
    default:
      return -1;
  }
}

function comparePoolAttemptRecency(
  current: ApiPoolUpstreamRequestAttempt | undefined,
  incoming: ApiPoolUpstreamRequestAttempt | undefined,
) {
  if (!current && incoming) return 1;
  if (current && !incoming) return -1;
  if (!current || !incoming) return 0;

  const currentTerminal = isPoolAttemptTerminal(current);
  const incomingTerminal = isPoolAttemptTerminal(incoming);
  if (currentTerminal !== incomingTerminal) {
    return incomingTerminal ? 1 : -1;
  }

  const currentPhaseRank = poolAttemptPhaseRank(current);
  const incomingPhaseRank = poolAttemptPhaseRank(incoming);
  if (currentPhaseRank !== incomingPhaseRank) {
    return incomingPhaseRank > currentPhaseRank ? 1 : -1;
  }

  const currentFinishedAt = current.finishedAt ? Date.parse(current.finishedAt) : Number.NaN;
  const incomingFinishedAt = incoming.finishedAt ? Date.parse(incoming.finishedAt) : Number.NaN;
  if (
    Number.isFinite(currentFinishedAt) &&
    Number.isFinite(incomingFinishedAt) &&
    currentFinishedAt !== incomingFinishedAt
  ) {
    return incomingFinishedAt > currentFinishedAt ? 1 : -1;
  }

  const currentScore = poolAttemptCompletenessScore(current);
  const incomingScore = poolAttemptCompletenessScore(incoming);
  if (currentScore !== incomingScore) {
    return incomingScore > currentScore ? 1 : -1;
  }

  return 0;
}

function shouldReplacePoolAttemptSnapshot(
  current: ApiPoolUpstreamRequestAttempt[] | undefined,
  incoming: ApiPoolUpstreamRequestAttempt[],
) {
  if (!current) return true;
  if (incoming.length > current.length) return true;
  if (incoming.length < current.length) return false;

  let sawNewer = false;
  let sawOlder = false;
  for (let index = 0; index < incoming.length; index += 1) {
    const comparison = comparePoolAttemptRecency(current[index], incoming[index]);
    if (comparison > 0) sawNewer = true;
    if (comparison < 0) sawOlder = true;
  }

  if (sawOlder && !sawNewer) return false;
  return true;
}

const MAX_BUFFERED_POOL_ATTEMPT_SNAPSHOTS = 12;

export function useInvocationPoolAttempts(expandedRecord: ApiInvocation | null) {
  const [attemptsByInvokeId, setPoolAttemptsByInvokeId] = useState<
    Record<string, ApiPoolUpstreamRequestAttempt[] | undefined>
  >({});
  const [loadingByInvokeId, setPoolAttemptLoadingByInvokeId] = useState<
    Record<string, boolean | undefined>
  >({});
  const [errorByInvokeId, setPoolAttemptErrorByInvokeId] = useState<
    Record<string, string | null | undefined>
  >({});
  const attemptsRef = useRef(attemptsByInvokeId);
  const loadingRef = useRef(loadingByInvokeId);
  const activeExpandedInvokeIdRef = useRef<string | null>(null);
  const versionRef = useRef<Record<string, number | undefined>>({});
  const loadedKeyRef = useRef<Record<string, string | undefined>>({});
  const loadingKeyRef = useRef<Record<string, string | undefined>>({});
  const activeRequestIdRef = useRef<Record<string, number | undefined>>({});
  const bufferedSnapshotsRef = useRef<Record<string, ApiPoolUpstreamRequestAttempt[] | undefined>>(
    {},
  );
  const bufferedSnapshotOrderRef = useRef<string[]>([]);
  const nextRequestIdRef = useRef(0);

  useEffect(() => {
    attemptsRef.current = attemptsByInvokeId;
  }, [attemptsByInvokeId]);

  useEffect(() => {
    loadingRef.current = loadingByInvokeId;
  }, [loadingByInvokeId]);

  useEffect(() => {
    activeExpandedInvokeIdRef.current =
      expandedRecord && isPoolRouteMode(expandedRecord.routeMode) ? expandedRecord.invokeId : null;
  }, [expandedRecord]);

  useEffect(() => {
    const activeInvokeId = activeExpandedInvokeIdRef.current;
    if (!activeInvokeId) {
      return;
    }
    const unsubscribe = subscribeToTopic<ApiPoolUpstreamRequestAttempt[]>(
      buildTopicDescriptor("invocation.pool-attempts", {
        invokeId: activeInvokeId,
      }),
      (event) => {
        const payloadInvokeId = activeInvokeId;
        const payloadAttempts = event.payload;
        const activeVisibleInvokeId = activeExpandedInvokeIdRef.current;
        const currentBuffered = bufferedSnapshotsRef.current[payloadInvokeId];
        const currentVisible =
          payloadInvokeId === activeVisibleInvokeId
            ? attemptsRef.current[payloadInvokeId]
            : currentBuffered;
        if (!shouldReplacePoolAttemptSnapshot(currentVisible, payloadAttempts)) return;

        const nextBuffered = {
          ...bufferedSnapshotsRef.current,
          [payloadInvokeId]: payloadAttempts,
        };
        const nextOrder = [
          payloadInvokeId,
          ...bufferedSnapshotOrderRef.current.filter((invokeId) => invokeId !== payloadInvokeId),
        ];
        while (nextOrder.length > MAX_BUFFERED_POOL_ATTEMPT_SNAPSHOTS) {
          const evictedInvokeId = nextOrder.pop();
          if (!evictedInvokeId) break;
          delete nextBuffered[evictedInvokeId];
        }
        bufferedSnapshotsRef.current = nextBuffered;
        bufferedSnapshotOrderRef.current = nextOrder;
        versionRef.current = {
          ...versionRef.current,
          [payloadInvokeId]: (versionRef.current[payloadInvokeId] ?? 0) + 1,
        };
        if (payloadInvokeId !== activeVisibleInvokeId) {
          return;
        }

        const nextAttempts = {
          ...attemptsRef.current,
          [payloadInvokeId]: payloadAttempts,
        };
        attemptsRef.current = nextAttempts;
        setPoolAttemptsByInvokeId(nextAttempts);
        setPoolAttemptLoadingByInvokeId((current) => ({
          ...current,
          [payloadInvokeId]: false,
        }));
        setPoolAttemptErrorByInvokeId((current) => ({
          ...current,
          [payloadInvokeId]: null,
        }));
      },
    );

    return unsubscribe;
  }, [expandedRecord]);

  const expandedPoolAttemptRouteMode = expandedRecord?.routeMode ?? null;
  const expandedPoolAttemptInvokeId = expandedRecord?.invokeId ?? null;
  const expandedPoolAttemptStatus = expandedRecord?.status ?? null;
  const expandedPoolAttemptCount = expandedRecord?.poolAttemptCount ?? null;
  const expandedPoolDistinctAccountCount = expandedRecord?.poolDistinctAccountCount ?? null;
  const expandedPoolAttemptTerminalReason = expandedRecord?.poolAttemptTerminalReason ?? null;
  const expandedPoolFailureKind = expandedRecord?.failureKind ?? null;
  const expandedPoolErrorMessage = expandedRecord?.errorMessage ?? null;
  const expandedPoolDownstreamStatusCode = expandedRecord?.downstreamStatusCode ?? null;
  const expandedPoolDownstreamErrorMessage = expandedRecord?.downstreamErrorMessage ?? null;
  const expandedPoolUpstreamErrorCode = expandedRecord?.upstreamErrorCode ?? null;
  const expandedPoolUpstreamErrorMessage = expandedRecord?.upstreamErrorMessage ?? null;
  const expandedPoolUpstreamRequestId = expandedRecord?.upstreamRequestId ?? null;
  const expandedPoolUpstreamAccountId = expandedRecord?.upstreamAccountId ?? null;
  const expandedPoolUpstreamAccountName = expandedRecord?.upstreamAccountName ?? null;
  const expandedPoolTUpstreamConnectMs = expandedRecord?.tUpstreamConnectMs ?? null;
  const expandedPoolTUpstreamTtfbMs = expandedRecord?.tUpstreamTtfbMs ?? null;
  const expandedPoolTUpstreamStreamMs = expandedRecord?.tUpstreamStreamMs ?? null;

  useEffect(() => {
    if (!expandedPoolAttemptInvokeId || !isPoolRouteMode(expandedPoolAttemptRouteMode)) {
      return;
    }
    const invokeId = expandedPoolAttemptInvokeId;
    const normalizedStatus = expandedPoolAttemptStatus?.trim().toLowerCase() ?? "";
    const requestKey = [
      normalizedStatus,
      expandedPoolAttemptCount ?? "",
      expandedPoolDistinctAccountCount ?? "",
      expandedPoolAttemptTerminalReason ?? "",
      expandedPoolFailureKind ?? "",
      expandedPoolErrorMessage ?? "",
      expandedPoolDownstreamStatusCode ?? "",
      expandedPoolDownstreamErrorMessage ?? "",
      expandedPoolUpstreamErrorCode ?? "",
      expandedPoolUpstreamErrorMessage ?? "",
      expandedPoolUpstreamRequestId ?? "",
      expandedPoolUpstreamAccountId ?? "",
      expandedPoolUpstreamAccountName ?? "",
      expandedPoolTUpstreamConnectMs ?? "",
      expandedPoolTUpstreamTtfbMs ?? "",
      expandedPoolTUpstreamStreamMs ?? "",
    ].join("|");
    const isInFlight = normalizedStatus === "running" || normalizedStatus === "pending";
    const bufferedAttempts = bufferedSnapshotsRef.current[invokeId];
    const stateAttempts = attemptsRef.current[invokeId];
    const cachedAttempts =
      shouldReplacePoolAttemptSnapshot(stateAttempts, bufferedAttempts ?? []) && bufferedAttempts
        ? bufferedAttempts
        : stateAttempts;
    if (cachedAttempts !== stateAttempts) {
      const nextAttempts = {
        ...attemptsRef.current,
        [invokeId]: cachedAttempts,
      };
      attemptsRef.current = nextAttempts;
      setPoolAttemptsByInvokeId(nextAttempts);
      setPoolAttemptErrorByInvokeId((current) => ({
        ...current,
        [invokeId]: null,
      }));
    }
    const hasCachedAttempts = cachedAttempts !== undefined;
    const expectedAttemptCount =
      typeof expandedPoolAttemptCount === "number" && Number.isFinite(expandedPoolAttemptCount)
        ? Math.max(Math.trunc(expandedPoolAttemptCount), 0)
        : null;
    const cachedAttemptCount = cachedAttempts?.length ?? 0;
    const loadedKey = loadedKeyRef.current[invokeId];
    const loadingKey = loadingKeyRef.current[invokeId];
    const shouldRefreshPendingTerminalAttempt =
      isInvocationDisplayTerminal(expandedPoolAttemptStatus) &&
      (cachedAttempts?.some((attempt) => !isPoolAttemptTerminal(attempt)) ?? false);
    const shouldRefreshInFlightKeyMismatch =
      isInFlight &&
      hasCachedAttempts &&
      loadedKey !== undefined &&
      loadedKey !== requestKey &&
      (cachedAttempts?.some((attempt) => !isPoolAttemptTerminal(attempt)) ?? false);
    const shouldRefetch =
      cachedAttempts === undefined ||
      (expectedAttemptCount != null && cachedAttemptCount < expectedAttemptCount) ||
      shouldRefreshPendingTerminalAttempt ||
      shouldRefreshInFlightKeyMismatch ||
      (hasCachedAttempts && loadedKey !== requestKey && !isInFlight);

    if (loadingRef.current[invokeId] && loadingKey === requestKey) return;
    if (!shouldRefetch) return;

    let cancelled = false;
    const requestId = ++nextRequestIdRef.current;
    const fetchVersion = versionRef.current[invokeId] ?? 0;
    const activeRequestIds = activeRequestIdRef.current;
    const loadingKeys = loadingKeyRef.current;
    loadingKeyRef.current[invokeId] = requestKey;
    activeRequestIdRef.current[invokeId] = requestId;
    setPoolAttemptLoadingByInvokeId((current) => ({
      ...current,
      [invokeId]: true,
    }));
    setPoolAttemptErrorByInvokeId((current) => ({
      ...current,
      [invokeId]: null,
    }));

    fetchInvocationPoolAttempts(invokeId)
      .then((attempts) => {
        if (cancelled) return;
        loadedKeyRef.current[invokeId] = requestKey;
        setPoolAttemptsByInvokeId((current) => {
          const latestVersion = versionRef.current[invokeId] ?? 0;
          const existingAttempts = current[invokeId];
          if (
            latestVersion !== fetchVersion &&
            !shouldReplacePoolAttemptSnapshot(existingAttempts, attempts)
          ) {
            return current;
          }
          if (!shouldReplacePoolAttemptSnapshot(existingAttempts, attempts)) {
            return current;
          }
          const nextAttempts = { ...current, [invokeId]: attempts };
          attemptsRef.current = nextAttempts;
          return nextAttempts;
        });
      })
      .catch((error) => {
        if (cancelled) return;
        const latestVersion = versionRef.current[invokeId] ?? 0;
        const existingAttempts = attemptsRef.current[invokeId];
        if (latestVersion !== fetchVersion || (existingAttempts?.length ?? 0) > 0) {
          return;
        }
        const message = error instanceof Error ? error.message : String(error);
        setPoolAttemptErrorByInvokeId((current) => ({
          ...current,
          [invokeId]: message,
        }));
      })
      .finally(() => {
        if (activeRequestIdRef.current[invokeId] === requestId) {
          delete activeRequestIdRef.current[invokeId];
          delete loadingKeyRef.current[invokeId];
          setPoolAttemptLoadingByInvokeId((current) => ({
            ...current,
            [invokeId]: false,
          }));
        }
      });

    return () => {
      cancelled = true;
      if (activeRequestIds[invokeId] === requestId) {
        delete activeRequestIds[invokeId];
        delete loadingKeys[invokeId];
        setPoolAttemptLoadingByInvokeId((current) => ({
          ...current,
          [invokeId]: false,
        }));
      }
    };
  }, [
    expandedPoolAttemptCount,
    expandedPoolAttemptInvokeId,
    expandedPoolAttemptRouteMode,
    expandedPoolAttemptStatus,
    expandedPoolAttemptTerminalReason,
    expandedPoolDistinctAccountCount,
    expandedPoolDownstreamErrorMessage,
    expandedPoolDownstreamStatusCode,
    expandedPoolErrorMessage,
    expandedPoolFailureKind,
    expandedPoolTUpstreamConnectMs,
    expandedPoolTUpstreamStreamMs,
    expandedPoolTUpstreamTtfbMs,
    expandedPoolUpstreamAccountId,
    expandedPoolUpstreamAccountName,
    expandedPoolUpstreamErrorCode,
    expandedPoolUpstreamErrorMessage,
    expandedPoolUpstreamRequestId,
  ]);

  return {
    attemptsByInvokeId,
    loadingByInvokeId,
    errorByInvokeId,
  };
}

function renderPoolAttemptsContent(
  record: ApiInvocation,
  poolAttemptsState: InvocationPoolAttemptsState,
  proxyBindingNodesByKey: Map<string, ForwardProxyBindingNode>,
  focusedAttemptId: string | null,
  t: Translator,
) {
  const invokeId = record.invokeId;
  const attempts = poolAttemptsState.attemptsByInvokeId[invokeId];
  const isLoadingAttempts = !!poolAttemptsState.loadingByInvokeId[invokeId];
  const attemptsError = poolAttemptsState.errorByInvokeId[invokeId];

  if (!isPoolRouteMode(record.routeMode)) {
    return (
      <div
        className="rounded-lg border border-base-300/70 bg-base-200/45 px-3 py-2 text-sm text-base-content/70"
        data-testid="pool-attempts-empty"
      >
        {t("table.poolAttempts.notPool")}
      </div>
    );
  }

  const summaryParts = [
    `${t("table.details.poolAttemptCount")}: ${formatOptionalText(
      record.poolAttemptCount != null ? String(record.poolAttemptCount) : undefined,
    )}`,
    `${t("table.details.poolDistinctAccountCount")}: ${formatOptionalText(
      record.poolDistinctAccountCount != null ? String(record.poolDistinctAccountCount) : undefined,
    )}`,
    `${t("table.details.poolAttemptTerminalReason")}: ${formatOptionalText(record.poolAttemptTerminalReason)}`,
  ];
  const realAttempts = attempts?.filter((attempt) => !isSyntheticPoolTerminalAttempt(attempt));
  const syntheticTerminalAttempts = attempts?.filter(isSyntheticPoolTerminalAttempt);
  const loadedSummaryParts =
    attempts && attempts.length > 0
      ? [
          `${t("table.poolAttempts.realAttemptCount")}: ${realAttempts?.length ?? 0}`,
          `${t("table.poolAttempts.terminalRecordCount")}: ${syntheticTerminalAttempts?.length ?? 0}`,
        ]
      : [];

  return (
    <div
      className="rounded-xl border border-base-300/70 bg-base-100/52 p-3"
      data-testid="pool-attempts-section"
    >
      <div className="flex flex-col gap-2 md:flex-row md:items-start md:justify-between">
        <div className="space-y-1">
          <span className="text-xs font-semibold text-base-content/82">
            {t("table.poolAttempts.title")}
          </span>
          <div className="text-xs leading-5 text-base-content/62">{summaryParts.join(" · ")}</div>
        </div>
        {loadedSummaryParts.length > 0 ? (
          <div className="flex flex-wrap gap-1.5 md:justify-end">
            {loadedSummaryParts.map((part) => (
              <span
                key={part}
                className="rounded-full border border-base-300/70 bg-base-200/58 px-2 py-1 text-[11px] font-medium text-base-content/70"
              >
                {part}
              </span>
            ))}
          </div>
        ) : null}
      </div>

      <div className="mt-3">
        {attemptsError ? (
          <div
            className="rounded-lg border border-error/25 bg-error/8 px-3 py-2 text-sm text-error"
            data-testid="pool-attempts-error"
          >
            {t("table.poolAttempts.loadError", { error: attemptsError })}
          </div>
        ) : attempts && attempts.length > 0 ? (
          <div className="space-y-3">
            {realAttempts && realAttempts.length > 0 ? (
              <div className="space-y-2" data-testid="pool-attempts-list">
                {realAttempts.map((attempt) => {
                  const proxyBindingDisplay = formatPoolAttemptProxyBindingDisplay(
                    attempt,
                    proxyBindingNodesByKey,
                  );
                  const isFocused =
                    focusedAttemptId != null && attempt.attemptId === focusedAttemptId;

                  return (
                    <PoolAttemptRecordCard
                      key={`${attempt.attemptId}-${attempt.attemptIndex}`}
                      attempt={attempt}
                      proxyDisplay={proxyBindingDisplay}
                      isFocused={isFocused}
                      t={t}
                      testId="pool-attempt-item"
                    >
                      {attempt.errorMessage?.trim() ? (
                        <div
                          className="mt-2 flex flex-col gap-1"
                          data-testid="pool-attempt-upstream-error"
                        >
                          <span className="text-[11px] font-semibold uppercase tracking-wide text-base-content/60">
                            {t("table.poolAttempts.upstreamErrorMessage")}
                          </span>
                          <pre className="whitespace-pre-wrap break-words font-mono text-sm text-base-content/80">
                            {attempt.errorMessage}
                          </pre>
                        </div>
                      ) : null}
                      {attempt.downstreamErrorMessage?.trim() ? (
                        <div
                          className="mt-2 flex flex-col gap-1"
                          data-testid="pool-attempt-downstream-error"
                        >
                          <span className="text-[11px] font-semibold uppercase tracking-wide text-base-content/60">
                            {t("table.poolAttempts.downstreamErrorMessage")}
                          </span>
                          <pre className="whitespace-pre-wrap break-words font-mono text-sm text-base-content/80">
                            {attempt.downstreamErrorMessage}
                          </pre>
                        </div>
                      ) : null}
                    </PoolAttemptRecordCard>
                  );
                })}
              </div>
            ) : null}
            {syntheticTerminalAttempts?.map((attempt) => {
              const statusMeta = poolAttemptStatusMeta(attempt.status);
              const accountLabel = formatPoolAttemptAccountLabel(attempt);
              const httpStatusValue = formatOptionalStatusCode(attempt.httpStatus);
              const distinctAccountValue =
                attempt.distinctAccountIndex > 0
                  ? String(attempt.distinctAccountIndex)
                  : record.poolDistinctAccountCount != null
                    ? String(record.poolDistinctAccountCount)
                    : undefined;

              return (
                <div
                  key={`terminal-${attempt.attemptId}-${attempt.attemptIndex}`}
                  className="overflow-hidden rounded-xl border border-warning/40 bg-warning/10 shadow-sm"
                  data-testid="pool-attempt-terminal-record"
                  data-attempt-id={attempt.attemptId}
                >
                  <div className="flex flex-col gap-3 border-b border-warning/25 bg-warning/12 px-3 py-3 md:flex-row md:items-start md:justify-between">
                    <div className="flex min-w-0 items-start gap-3">
                      <span
                        className="mt-1 h-2.5 w-2.5 flex-none rounded-full bg-warning ring-4 ring-warning/20"
                        aria-hidden="true"
                      />
                      <div className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-sm font-semibold">
                            {t("table.poolAttempts.terminal.title")}
                          </span>
                          <Badge variant={statusMeta.variant}>{t(statusMeta.key)}</Badge>
                        </div>
                        <p className="mt-1 max-w-3xl text-sm text-base-content/75">
                          {t(
                            poolAttemptTerminalDescriptionKey(
                              attempt.failureKind ?? record.poolAttemptTerminalReason,
                            ),
                          )}
                        </p>
                      </div>
                    </div>
                    <Badge variant="secondary" className="w-fit">
                      {t("table.poolAttempts.terminal.notDispatched")}
                    </Badge>
                  </div>
                  <div className="grid gap-2 p-3 text-sm md:grid-cols-2 xl:grid-cols-3">
                    <div className="rounded-lg border border-warning/20 bg-base-100/55 px-3 py-2">
                      <span className="block text-xs uppercase tracking-wide text-base-content/60">
                        {t("table.poolAttempts.terminal.realAttempts")}
                      </span>
                      <span className="mt-1 block font-mono text-base font-semibold">
                        {String(realAttempts?.length ?? 0)}
                      </span>
                    </div>
                    <div className="rounded-lg border border-warning/20 bg-base-100/55 px-3 py-2">
                      <span className="block text-xs uppercase tracking-wide text-base-content/60">
                        {t("table.poolAttempts.terminal.distinctAccounts")}
                      </span>
                      <span className="mt-1 block font-mono text-base font-semibold">
                        {formatOptionalText(distinctAccountValue)}
                      </span>
                    </div>
                    <div className="rounded-lg border border-base-300/70 bg-base-100/45 px-3 py-2">
                      <span className="block text-xs uppercase tracking-wide text-base-content/60">
                        {t("table.poolAttempts.terminal.previousAccount")}
                      </span>
                      <span className="mt-1 block break-all">{accountLabel}</span>
                    </div>
                    <div className="rounded-lg border border-base-300/70 bg-base-100/45 px-3 py-2">
                      <span className="block text-xs uppercase tracking-wide text-base-content/60">
                        {t("table.poolAttempts.terminal.previousHttpStatus")}
                      </span>
                      <span className="mt-1 block font-mono">{httpStatusValue}</span>
                    </div>
                    <div className="rounded-lg border border-base-300/70 bg-base-100/45 px-3 py-2 md:col-span-2 xl:col-span-1">
                      <span className="block text-xs uppercase tracking-wide text-base-content/60">
                        {t("table.poolAttempts.terminal.reason")}
                      </span>
                      <span className="mt-1 block break-all font-mono">
                        {formatOptionalText(attempt.failureKind)}
                      </span>
                    </div>
                  </div>
                  {attempt.errorMessage?.trim() ? (
                    <div
                      className="border-t border-warning/20 bg-base-100/35 px-3 py-2"
                      data-testid="pool-attempt-terminal-error"
                    >
                      <span className="text-[11px] font-semibold uppercase tracking-wide text-base-content/60">
                        {t("table.poolAttempts.terminal.previousError")}
                      </span>
                      <pre className="mt-1 whitespace-pre-wrap break-words font-mono text-sm text-base-content/80">
                        {attempt.errorMessage}
                      </pre>
                    </div>
                  ) : null}
                </div>
              );
            })}
          </div>
        ) : isLoadingAttempts ? (
          <div
            className="inline-flex items-center gap-2 rounded-lg border border-base-300/70 bg-base-200/45 px-3 py-2 text-sm text-base-content/70"
            data-testid="pool-attempts-loading"
          >
            <Spinner size="sm" aria-label={t("table.poolAttempts.loading")} />
            <span>{t("table.poolAttempts.loading")}</span>
          </div>
        ) : (
          <div
            className="rounded-lg border border-base-300/70 bg-base-200/45 px-3 py-2 text-sm text-base-content/70"
            data-testid="pool-attempts-empty"
          >
            {t("table.poolAttempts.empty")}
          </div>
        )}
      </div>
    </div>
  );
}

function resolveUnavailableResponseBodyMessage(reason: string | null | undefined, t: Translator) {
  const normalized = reason?.trim().toLowerCase() ?? "";
  if (normalized === "not_abnormal") return t("table.responseBody.unavailable.notAbnormal");
  if (normalized === "detail_pruned") return t("table.responseBody.unavailable.detailPruned");
  if (normalized.startsWith("raw_file_missing"))
    return t("table.responseBody.unavailable.rawFileMissing");
  if (normalized.startsWith("raw_file_unreadable"))
    return t("table.responseBody.unavailable.rawFileUnreadable");
  if (normalized.startsWith("preview_only")) return t("table.responseBody.unavailable.previewOnly");
  if (normalized.startsWith("missing_body")) return t("table.responseBody.unavailable.missingBody");
  return t("table.responseBody.unavailable.generic");
}

function getDetailPair(pairByKey: Map<string, DetailPair>, key: string) {
  return pairByKey.get(key);
}

function collectDetailPairs(pairByKey: Map<string, DetailPair>, keys: string[]) {
  return keys
    .map((key) => getDetailPair(pairByKey, key))
    .filter((pair): pair is DetailPair => pair != null);
}

function DetailSection({
  title,
  children,
  className,
  testId,
}: {
  title: string;
  children: ReactNode;
  className?: string;
  testId?: string;
}) {
  return (
    <section
      className={cn(
        "min-w-0 max-w-full rounded-xl border border-base-300/70 bg-base-100/52 p-3",
        className,
      )}
      data-testid={testId}
    >
      <h4 className="text-xs font-semibold text-base-content/82">{title}</h4>
      <div className="mt-3">{children}</div>
    </section>
  );
}

function DetailFields({
  pairs,
  columns = "md:grid-cols-2",
}: {
  pairs: DetailPair[];
  columns?: string;
}) {
  if (pairs.length === 0) return null;
  return (
    <dl className={cn("grid gap-x-5 gap-y-3", columns)}>
      {pairs.map((entry) => (
        <DetailField key={entry.key} entry={entry} />
      ))}
    </dl>
  );
}

function DetailField({ entry }: { entry: DetailPair }) {
  return (
    <div className="grid min-w-0 gap-1 sm:grid-cols-[8.75rem_minmax(0,1fr)] sm:gap-3">
      <dt className="text-xs leading-5 text-base-content/58">{entry.label}</dt>
      <dd className="min-w-0 break-words font-mono text-sm leading-5 text-base-content/88">
        {entry.value}
      </dd>
    </div>
  );
}

function DetailHeroFields({ pairs }: { pairs: DetailPair[] }) {
  if (pairs.length === 0) return null;
  return (
    <dl className="grid gap-2 md:grid-cols-2 xl:grid-cols-4">
      {pairs.map((entry) => (
        <div
          key={entry.key}
          className="min-w-0 rounded-lg border border-base-300/60 bg-base-200/42 px-3 py-2"
        >
          <dt className="text-[11px] leading-4 text-base-content/58">{entry.label}</dt>
          <dd className="mt-1 min-w-0 break-words font-mono text-sm font-semibold leading-5 text-base-content/90">
            {entry.value}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function TimingRail({ timingPairs }: { timingPairs: Array<{ label: string; value: string }> }) {
  return (
    <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-4">
      {timingPairs.map((entry, index) => {
        const isTotal = index === timingPairs.length - 1;
        return (
          <div
            key={entry.label}
            className={cn(
              "rounded-lg border px-3 py-2",
              isTotal ? "border-primary/24 bg-primary/8" : "border-base-300/60 bg-base-200/42",
            )}
          >
            <div className="flex items-center gap-2">
              <span
                className={cn(
                  "h-1.5 w-1.5 rounded-full",
                  isTotal ? "bg-primary" : "bg-base-content/36",
                )}
                aria-hidden
              />
              <span className="min-w-0 truncate text-xs text-base-content/60">{entry.label}</span>
            </div>
            <div
              className={cn(
                "mt-1 font-mono text-sm leading-5",
                isTotal ? "font-semibold text-primary" : "text-base-content/88",
              )}
            >
              {entry.value}
            </div>
          </div>
        );
      })}
    </div>
  );
}

function ErrorDetailBlock({
  title,
  children,
  testId,
  tone = "neutral",
}: {
  title: string;
  children: ReactNode;
  testId?: string;
  tone?: "neutral" | "error";
}) {
  return (
    <section
      className={cn(
        "rounded-xl border p-3",
        tone === "error" ? "border-error/25 bg-error/8" : "border-base-300/70 bg-base-100/52",
      )}
      data-testid={testId}
    >
      <h4 className="text-xs font-semibold text-base-content/82">{title}</h4>
      <div className="mt-3 space-y-3">{children}</div>
    </section>
  );
}

function DetailErrorText({ label, children }: { label: string; children: string }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-[11px] font-semibold uppercase tracking-wide text-base-content/60">
        {label}
      </span>
      <pre className="min-w-0 max-w-full whitespace-pre-wrap break-words font-mono text-sm leading-5 text-base-content/84 [overflow-wrap:anywhere]">
        {children}
      </pre>
    </div>
  );
}

export function InvocationExpandedDetails({
  record,
  detailId,
  detailPairs,
  timingPairs,
  errorMessage,
  detailNotice,
  size,
  poolAttemptsState,
  focusedAttemptId = null,
  abnormalResponseBody,
  abnormalResponseBodyLoading = false,
  abnormalResponseBodyError,
  onOpenFullDetails,
  showFullDetailsAction = false,
  t,
}: InvocationExpandedDetailsProps) {
  const poolAttempts = poolAttemptsState.attemptsByInvokeId[record.invokeId];
  const poolAttemptProxyBindingKeys = useMemo(
    () => collectPoolAttemptProxyBindingKeys(poolAttempts),
    [poolAttempts],
  );
  const { nodes: poolAttemptProxyBindingNodes } = useForwardProxyBindingNodes(
    poolAttemptProxyBindingKeys,
    {
      enabled: poolAttemptProxyBindingKeys.length > 0,
    },
  );
  const poolAttemptProxyBindingNodesByKey = useMemo(
    () => buildForwardProxyBindingNodeMap(poolAttemptProxyBindingNodes),
    [poolAttemptProxyBindingNodes],
  );
  const canonicalUpstreamError = errorMessage.trim();
  const upstreamRawError = record.upstreamErrorMessage?.trim() ?? "";
  const downstreamErrorMessage = record.downstreamErrorMessage?.trim() ?? "";
  const showUpstreamErrorSection = Boolean(canonicalUpstreamError || upstreamRawError);
  const showDownstreamErrorSection = Boolean(
    downstreamErrorMessage ||
      (typeof record.downstreamStatusCode === "number" &&
        Number.isFinite(record.downstreamStatusCode)),
  );
  const showResponseBodySection =
    abnormalResponseBody != null ||
    abnormalResponseBodyLoading ||
    Boolean(abnormalResponseBodyError) ||
    showFullDetailsAction;
  const detailPairByKey = useMemo(
    () => new Map(detailPairs.map((entry) => [entry.key, entry])),
    [detailPairs],
  );
  const identityPairs = collectDetailPairs(detailPairByKey, [
    "invokeId",
    "account",
    "requesterIp",
    "promptCacheKey",
  ]);
  const routingPairs = collectDetailPairs(detailPairByKey, [
    "proxy",
    "endpoint",
    "requestModel",
    "responseModel",
    "compactionRequest",
    "compactionResponse",
    "imageIntent",
    "responseContentEncoding",
    "requestedServiceTier",
    "serviceTier",
    "billingServiceTier",
    "reasoningEffort",
    "reasoningTokens",
    "proxyWeightDelta",
  ]);
  const failurePairs = collectDetailPairs(detailPairByKey, [
    "poolAttemptCount",
    "poolDistinctAccountCount",
    "poolAttemptTerminalReason",
    "failureClass",
    "failureKind",
    "actionable",
    "streamTerminalEvent",
    "upstreamErrorCode",
    "downstreamStatusCode",
    "upstreamRequestId",
  ]);
  const retentionPairs = collectDetailPairs(detailPairByKey, [
    "detailLevel",
    "detailPrunedAt",
    "detailPruneReason",
  ]);

  return (
    <div id={detailId} className={cn("flex flex-col gap-3", size === "compact" ? "p-3" : "p-4")}>
      {detailNotice ? (
        <div
          className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs leading-5 text-warning"
          data-testid="invocation-detail-notice"
        >
          {detailNotice}
        </div>
      ) : null}

      <DetailSection title={t("table.detailsTitle")}>
        <DetailHeroFields pairs={identityPairs} />
      </DetailSection>

      <DetailSection title={t("table.details.routingTitle")}>
        <DetailFields pairs={routingPairs} />
      </DetailSection>

      <DetailSection title={t("table.details.failureTitle")}>
        <DetailFields pairs={failurePairs} />
      </DetailSection>

      <DetailSection title={t("table.details.retentionTitle")}>
        <DetailFields pairs={retentionPairs} />
      </DetailSection>

      <DetailSection title={t("table.details.timingsTitle")}>
        <TimingRail timingPairs={timingPairs} />
      </DetailSection>

      {showUpstreamErrorSection ? (
        <ErrorDetailBlock title={t("table.upstreamErrorDetailsTitle")} tone="error">
          {canonicalUpstreamError ? (
            <div data-testid="invocation-upstream-error-section">
              <DetailErrorText label={t("table.upstreamCanonicalErrorLabel")}>
                {canonicalUpstreamError}
              </DetailErrorText>
            </div>
          ) : null}
          {upstreamRawError && upstreamRawError !== canonicalUpstreamError ? (
            <DetailErrorText label={t("table.details.upstreamErrorMessage")}>
              {upstreamRawError}
            </DetailErrorText>
          ) : null}
        </ErrorDetailBlock>
      ) : null}

      {showDownstreamErrorSection ? (
        <ErrorDetailBlock
          title={t("table.downstreamErrorDetailsTitle")}
          testId="invocation-downstream-error-section"
        >
          <DetailFields
            pairs={[
              {
                key: "downstreamStatusCode-inline",
                label: t("table.details.downstreamStatusCode"),
                value: formatOptionalStatusCode(record.downstreamStatusCode),
              },
            ]}
            columns="md:grid-cols-1"
          />
          {downstreamErrorMessage ? (
            <DetailErrorText label={t("table.details.downstreamErrorMessage")}>
              {downstreamErrorMessage}
            </DetailErrorText>
          ) : null}
        </ErrorDetailBlock>
      ) : null}

      {showResponseBodySection ? (
        <DetailSection
          title={t("table.responseBody.title")}
          testId="invocation-response-body-section"
        >
          <div className="flex flex-wrap items-center justify-between gap-3">
            <span className="text-xs text-base-content/62">{t("table.responseBody.title")}</span>
            {showFullDetailsAction && onOpenFullDetails ? (
              <Button type="button" variant="outline" size="sm" onClick={onOpenFullDetails}>
                {t("table.responseBody.openFullDetails")}
              </Button>
            ) : null}
          </div>

          {abnormalResponseBodyLoading ? (
            <div
              className="inline-flex items-center gap-2 rounded-lg border border-base-300/70 bg-base-200/45 px-3 py-2 text-sm text-base-content/70"
              data-testid="invocation-response-body-loading"
            >
              <Spinner size="sm" aria-label={t("table.responseBody.loading")} />
              <span>{t("table.responseBody.loading")}</span>
            </div>
          ) : abnormalResponseBodyError ? (
            <div
              className="rounded-lg border border-error/25 bg-error/8 px-3 py-2 text-sm text-error"
              data-testid="invocation-response-body-error"
            >
              {t("table.responseBody.loadError", {
                error: abnormalResponseBodyError,
              })}
            </div>
          ) : abnormalResponseBody?.available && abnormalResponseBody.previewText ? (
            <>
              <div data-testid="invocation-response-body-preview" className="min-w-0 max-w-full">
                <StructuredPayloadViewer
                  value={abnormalResponseBody.previewText}
                  labels={{
                    json: t("table.responseBody.format.json"),
                    ndjson: t("table.responseBody.format.ndjson"),
                    sse: t("table.responseBody.format.sse"),
                    text: t("table.responseBody.format.text"),
                    largePayload: t("table.responseBody.largePayload"),
                    parseLargePayload: t("table.responseBody.parseLargePayload"),
                    event: t("table.responseBody.event"),
                    data: t("table.responseBody.data"),
                    expand: t("table.responseBody.expand"),
                    collapse: t("table.responseBody.collapse"),
                  }}
                />
              </div>
              {abnormalResponseBody.hasMore ? (
                <p className="text-xs text-base-content/60">
                  {t("table.responseBody.previewTruncated")}
                </p>
              ) : null}
            </>
          ) : abnormalResponseBody ? (
            <div
              className="rounded-lg border border-base-300/70 bg-base-200/45 px-3 py-2 text-sm text-base-content/70"
              data-testid="invocation-response-body-unavailable"
            >
              {resolveUnavailableResponseBodyMessage(abnormalResponseBody.unavailableReason, t)}
            </div>
          ) : null}
        </DetailSection>
      ) : null}

      {renderPoolAttemptsContent(
        record,
        poolAttemptsState,
        poolAttemptProxyBindingNodesByKey,
        focusedAttemptId,
        t,
      )}
    </div>
  );
}
