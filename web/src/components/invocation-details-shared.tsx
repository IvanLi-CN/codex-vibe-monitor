/* eslint-disable react-refresh/only-export-components */
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { AppIcon } from "./AppIcon";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Spinner } from "./ui/spinner";
import {
  fetchInvocationPoolAttempts,
  type ApiInvocation,
  type ApiInvocationAbnormalResponseBodyPreview,
  type ApiPoolUpstreamRequestAttempt,
  type ForwardProxyBindingNode,
} from "../lib/api";
import { useForwardProxyBindingNodes } from "../hooks/useForwardProxyBindingNodes";
import {
  formatProxyWeightDelta,
  formatResponseContentEncoding,
  formatServiceTier,
  getFastIndicatorState,
  isPoolRouteMode,
  resolveFirstResponseByteTotalMs,
  resolveInvocationAccountLabel,
  resolveInvocationEndpointDisplay,
  type FastIndicatorState,
  type InvocationEndpointDisplay,
} from "../lib/invocation";
import type { TranslationKey } from "../i18n";
import { cn } from "../lib/utils";
import {
  getReasoningEffortTone,
  REASONING_EFFORT_TONE_CLASSNAMES,
} from "./invocation-table-reasoning";
import { subscribeToSse } from "../lib/sse";

export const FALLBACK_CELL = "—";

type Translator = (
  key: TranslationKey,
  values?: Record<string, string | number>,
) => string;

export type DetailPanelSize = "compact" | "default";

export interface InvocationDetailViewModel {
  accountLabel: string;
  accountId: number | null;
  accountClickable: boolean;
  accountPlanType: string | null;
  proxyDisplayName: string;
  modelValue: string;
  requestedServiceTierValue: string;
  serviceTierValue: string;
  billingServiceTierValue: string;
  fastIndicatorState: FastIndicatorState;
  costValue: string;
  inputTokensValue: string;
  cacheInputTokensValue: string;
  outputTokensValue: string;
  outputReasoningBreakdownValue: string;
  reasoningTokensValue: string;
  reasoningEffortValue: string;
  totalTokensValue: string;
  endpointValue: string;
  endpointDisplay: InvocationEndpointDisplay;
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
  attemptsByInvokeId: Record<
    string,
    ApiPoolUpstreamRequestAttempt[] | undefined
  >;
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
  abnormalResponseBody?: ApiInvocationAbnormalResponseBodyPreview | null;
  abnormalResponseBodyLoading?: boolean;
  abnormalResponseBodyError?: string | null;
  onOpenFullDetails?: (() => void) | null;
  showFullDetailsAction?: boolean;
  t: Translator;
}

function isZhLocale(locale: string) {
  return locale.trim().toLowerCase().startsWith("zh");
}

export function formatMilliseconds(value: number | null | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value))
    return FALLBACK_CELL;
  return `${value.toFixed(1)} ms`;
}

export function resolveInvocationCollapsedErrorSummary(
  record: Pick<ApiInvocation, "errorMessage" | "downstreamErrorMessage">,
) {
  const canonicalUpstreamError = record.errorMessage?.trim();
  if (canonicalUpstreamError) return canonicalUpstreamError;
  return record.downstreamErrorMessage?.trim() ?? "";
}

export function formatSecondsFromMilliseconds(
  value: number | null | undefined,
  localeTag: string,
) {
  if (typeof value !== "number" || !Number.isFinite(value))
    return FALLBACK_CELL;

  const seconds = value / 1000;
  const precision =
    Math.abs(seconds) >= 100 ? 1 : Math.abs(seconds) >= 1 ? 2 : 3;
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
  return formatSecondsFromMilliseconds(
    Math.max(0, nowMs - occurredMs),
    localeTag,
  );
}

export function formatOptionalNumber(
  value: number | null | undefined,
  formatter: Intl.NumberFormat,
) {
  if (typeof value !== "number" || !Number.isFinite(value))
    return FALLBACK_CELL;
  return formatter.format(value);
}

export function formatOptionalText(value: string | null | undefined) {
  const normalized = value?.trim();
  return normalized ? normalized : FALLBACK_CELL;
}

function formatOptionalStatusCode(value: number | null | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value))
    return FALLBACK_CELL;
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
    return (
      <span className="font-mono text-sm text-base-content/70">
        {FALLBACK_CELL}
      </span>
    );
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
      <span className="block max-w-full truncate whitespace-nowrap">
        {value}
      </span>
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

function collectPoolAttemptProxyBindingKeys(
  attempts: ApiPoolUpstreamRequestAttempt[] | undefined,
) {
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
      <AppIcon
        name="lightning-bolt"
        className="h-3.5 w-3.5 -translate-y-px"
        aria-hidden
      />
    </span>
  );
}

function renderEndpointRawPath(endpointValue: string, className?: string) {
  return (
    <span
      className={cn(
        "block truncate whitespace-nowrap font-mono text-base-content/70",
        className,
      )}
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
      return (
        <Badge variant="error">
          {t("records.filters.failureClass.service")}
        </Badge>
      );
    case "client_failure":
      return (
        <Badge variant="warning">
          {t("records.filters.failureClass.client")}
        </Badge>
      );
    case "client_abort":
      return (
        <Badge variant="secondary">
          {t("records.filters.failureClass.abort")}
        </Badge>
      );
    default:
      return FALLBACK_CELL;
  }
}

function formatActionableValue(
  value: ApiInvocation["isActionable"],
  t: Translator,
) {
  if (typeof value !== "boolean") return FALLBACK_CELL;
  return (
    <Badge variant={value ? "warning" : "secondary"}>
      {value
        ? t("table.details.actionableYes")
        : t("table.details.actionableNo")}
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
      structuredHint:
        "该记录仅保留结构化字段；离线归档保留归档行，超窗 raw file 不保证继续可用。",
      prunedPrefix: "精简于",
    };
  }

  return {
    full: "Full",
    structuredOnly: "Structured only",
    level: "Detail level",
    prunedAt: "Detail pruned at",
    pruneReason: "Detail prune reason",
    fullHint:
      "Full troubleshooting detail is still available inside the online retention window.",
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
      <div className="w-fit max-w-full">
        {renderEndpointSummary(endpointDisplay, t)}
      </div>
      <span className="break-all font-mono text-xs text-base-content/70">
        {endpointValue}
      </span>
    </div>
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
  const requestedServiceTierValue = formatServiceTier(
    record.requestedServiceTier,
  );
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
  const reasoningTokensValue = formatOptionalNumber(
    record.reasoningTokens,
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
  const endpointDisplay = resolveInvocationEndpointDisplay(record.endpoint);
  const endpointValue = endpointDisplay.endpointValue;
  const errorMessage = record.errorMessage?.trim() ?? "";
  const collapsedErrorSummary = resolveInvocationCollapsedErrorSummary(record);

  const proxyWeightDeltaView = formatProxyWeightDelta(record.proxyWeightDelta);
  const proxyWeightDeltaValue =
    proxyWeightDeltaView.direction === "missing" ? (
      FALLBACK_CELL
    ) : (
      <span
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
    detailLevel === "structured_only"
      ? detailLabels.structuredOnly
      : detailLabels.full;
  const detailLevelBadgeVariant =
    detailLevel === "structured_only" ? "warning" : "secondary";
  const detailNotice =
    detailLevel === "structured_only" ? detailLabels.structuredHint : null;
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
        "font-mono text-sm",
      ),
    },
    { key: "proxy", label: t("table.details.proxy"), value: proxyDisplayName },
    {
      key: "endpoint",
      label: t("table.details.endpoint"),
      value: renderDetailEndpointValue(endpointDisplay, endpointValue, t),
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
        record.poolAttemptCount != null
          ? String(record.poolAttemptCount)
          : undefined,
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
      value: formatSecondsFromMilliseconds(
        record.tUpstreamConnectMs,
        localeTag,
      ),
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
      typeof record.upstreamAccountId === "number" &&
      Number.isFinite(record.upstreamAccountId)
        ? Math.trunc(record.upstreamAccountId)
        : null,
    accountClickable,
    accountPlanType: record.upstreamAccountPlanType?.trim() || null,
    proxyDisplayName,
    modelValue: record.model ?? FALLBACK_CELL,
    requestedServiceTierValue,
    serviceTierValue,
    billingServiceTierValue,
    fastIndicatorState,
    costValue:
      typeof record.cost === "number"
        ? currencyFormatter.format(record.cost)
        : FALLBACK_CELL,
    inputTokensValue: formatOptionalNumber(record.inputTokens, numberFormatter),
    cacheInputTokensValue: formatOptionalNumber(
      record.cacheInputTokens,
      numberFormatter,
    ),
    outputTokensValue: formatOptionalNumber(
      record.outputTokens,
      numberFormatter,
    ),
    outputReasoningBreakdownValue,
    reasoningTokensValue,
    reasoningEffortValue,
    totalTokensValue: formatOptionalNumber(record.totalTokens, numberFormatter),
    endpointValue,
    endpointDisplay,
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
  if (
    typeof attempt.upstreamAccountId === "number" &&
    Number.isFinite(attempt.upstreamAccountId)
  ) {
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

function poolAttemptPhaseMeta(phase: string | null | undefined): {
  variant: "default" | "secondary" | "warning" | "info";
  key: TranslationKey;
} {
  switch (phase?.trim().toLowerCase()) {
    case "connecting":
      return {
        variant: "secondary",
        key: "table.poolAttempts.phase.connecting",
      };
    case "sending_request":
      return {
        variant: "default",
        key: "table.poolAttempts.phase.sendingRequest",
      };
    case "waiting_first_byte":
      return {
        variant: "warning",
        key: "table.poolAttempts.phase.waitingFirstByte",
      };
    case "streaming_response":
      return {
        variant: "info",
        key: "table.poolAttempts.phase.streamingResponse",
      };
    case "completed":
      return {
        variant: "secondary",
        key: "table.poolAttempts.phase.completed",
      };
    case "failed":
      return { variant: "secondary", key: "table.poolAttempts.phase.failed" };
    default:
      return { variant: "secondary", key: "table.poolAttempts.phase.unknown" };
  }
}

function isPoolAttemptTerminal(attempt: ApiPoolUpstreamRequestAttempt) {
  if (attempt.finishedAt?.trim()) return true;
  return attempt.status.trim().toLowerCase() !== "pending";
}

function isSyntheticPoolTerminalAttempt(attempt: ApiPoolUpstreamRequestAttempt) {
  const normalizedStatus = attempt.status.trim().toLowerCase();
  return (
    normalizedStatus === "budget_exhausted_final" ||
    attempt.sameAccountRetryIndex <= 0
  );
}

function poolAttemptTerminalDescriptionKey(
  terminalReason: string | null | undefined,
): TranslationKey {
  return terminalReason?.trim().toLowerCase() ===
    "max_distinct_accounts_exhausted"
    ? "table.poolAttempts.terminal.budgetExhaustedDescription"
    : "table.poolAttempts.terminal.genericDescription";
}

function isInvocationDisplayTerminal(status: string | null | undefined) {
  const normalized = status?.trim().toLowerCase();
  return Boolean(
    normalized && normalized !== "running" && normalized !== "pending",
  );
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

  const currentFinishedAt = current.finishedAt
    ? Date.parse(current.finishedAt)
    : Number.NaN;
  const incomingFinishedAt = incoming.finishedAt
    ? Date.parse(incoming.finishedAt)
    : Number.NaN;
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
    const comparison = comparePoolAttemptRecency(
      current[index],
      incoming[index],
    );
    if (comparison > 0) sawNewer = true;
    if (comparison < 0) sawOlder = true;
  }

  if (sawOlder && !sawNewer) return false;
  return true;
}

const MAX_BUFFERED_POOL_ATTEMPT_SNAPSHOTS = 12;

export function useInvocationPoolAttempts(
  expandedRecord: ApiInvocation | null,
) {
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
  const bufferedSnapshotsRef = useRef<
    Record<string, ApiPoolUpstreamRequestAttempt[] | undefined>
  >({});
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
      expandedRecord && isPoolRouteMode(expandedRecord.routeMode)
        ? expandedRecord.invokeId
        : null;
  }, [expandedRecord]);

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== "pool_attempts") return;
      const activeInvokeId = activeExpandedInvokeIdRef.current;
      const currentBuffered = bufferedSnapshotsRef.current[payload.invokeId];
      const currentVisible =
        payload.invokeId === activeInvokeId
          ? attemptsRef.current[payload.invokeId]
          : currentBuffered;
      if (!shouldReplacePoolAttemptSnapshot(currentVisible, payload.attempts))
        return;

      const nextBuffered = {
        ...bufferedSnapshotsRef.current,
        [payload.invokeId]: payload.attempts,
      };
      const nextOrder = [
        payload.invokeId,
        ...bufferedSnapshotOrderRef.current.filter(
          (invokeId) => invokeId !== payload.invokeId,
        ),
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
        [payload.invokeId]: (versionRef.current[payload.invokeId] ?? 0) + 1,
      };
      if (payload.invokeId !== activeInvokeId) {
        return;
      }

      const nextAttempts = {
        ...attemptsRef.current,
        [payload.invokeId]: payload.attempts,
      };
      attemptsRef.current = nextAttempts;
      setPoolAttemptsByInvokeId(nextAttempts);
      setPoolAttemptLoadingByInvokeId((current) => ({
        ...current,
        [payload.invokeId]: false,
      }));
      setPoolAttemptErrorByInvokeId((current) => ({
        ...current,
        [payload.invokeId]: null,
      }));
    });

    return unsubscribe;
  }, []);

  const expandedPoolAttemptRouteMode = expandedRecord?.routeMode ?? null;
  const expandedPoolAttemptInvokeId = expandedRecord?.invokeId ?? null;
  const expandedPoolAttemptStatus = expandedRecord?.status ?? null;
  const expandedPoolAttemptCount = expandedRecord?.poolAttemptCount ?? null;
  const expandedPoolDistinctAccountCount =
    expandedRecord?.poolDistinctAccountCount ?? null;
  const expandedPoolAttemptTerminalReason =
    expandedRecord?.poolAttemptTerminalReason ?? null;
  const expandedPoolFailureKind = expandedRecord?.failureKind ?? null;
  const expandedPoolErrorMessage = expandedRecord?.errorMessage ?? null;
  const expandedPoolDownstreamStatusCode =
    expandedRecord?.downstreamStatusCode ?? null;
  const expandedPoolDownstreamErrorMessage =
    expandedRecord?.downstreamErrorMessage ?? null;
  const expandedPoolUpstreamErrorCode =
    expandedRecord?.upstreamErrorCode ?? null;
  const expandedPoolUpstreamErrorMessage =
    expandedRecord?.upstreamErrorMessage ?? null;
  const expandedPoolUpstreamRequestId =
    expandedRecord?.upstreamRequestId ?? null;
  const expandedPoolUpstreamAccountId =
    expandedRecord?.upstreamAccountId ?? null;
  const expandedPoolUpstreamAccountName =
    expandedRecord?.upstreamAccountName ?? null;
  const expandedPoolTUpstreamConnectMs =
    expandedRecord?.tUpstreamConnectMs ?? null;
  const expandedPoolTUpstreamTtfbMs =
    expandedRecord?.tUpstreamTtfbMs ?? null;
  const expandedPoolTUpstreamStreamMs =
    expandedRecord?.tUpstreamStreamMs ?? null;

  useEffect(() => {
    if (
      !expandedPoolAttemptInvokeId ||
      !isPoolRouteMode(expandedPoolAttemptRouteMode)
    ) {
      return;
    }
    const invokeId = expandedPoolAttemptInvokeId;
    const normalizedStatus =
      expandedPoolAttemptStatus?.trim().toLowerCase() ?? "";
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
    const isInFlight =
      normalizedStatus === "running" || normalizedStatus === "pending";
    const bufferedAttempts = bufferedSnapshotsRef.current[invokeId];
    const stateAttempts = attemptsRef.current[invokeId];
    const cachedAttempts =
      shouldReplacePoolAttemptSnapshot(stateAttempts, bufferedAttempts ?? []) &&
      bufferedAttempts
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
      typeof expandedPoolAttemptCount === "number" &&
      Number.isFinite(expandedPoolAttemptCount)
        ? Math.max(Math.trunc(expandedPoolAttemptCount), 0)
        : null;
    const cachedAttemptCount = cachedAttempts?.length ?? 0;
    const loadedKey = loadedKeyRef.current[invokeId];
    const loadingKey = loadingKeyRef.current[invokeId];
    const shouldRefreshPendingTerminalAttempt =
      isInvocationDisplayTerminal(expandedPoolAttemptStatus) &&
      (cachedAttempts?.some((attempt) => !isPoolAttemptTerminal(attempt)) ??
        false);
    const shouldRefreshInFlightKeyMismatch =
      isInFlight &&
      hasCachedAttempts &&
      loadedKey !== undefined &&
      loadedKey !== requestKey &&
      (cachedAttempts?.some((attempt) => !isPoolAttemptTerminal(attempt)) ??
        false);
    const shouldRefetch =
      cachedAttempts === undefined ||
      (expectedAttemptCount != null &&
        cachedAttemptCount < expectedAttemptCount) ||
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
        if (
          latestVersion !== fetchVersion ||
          (existingAttempts?.length ?? 0) > 0
        ) {
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
      record.poolAttemptCount != null
        ? String(record.poolAttemptCount)
        : undefined,
    )}`,
    `${t("table.details.poolDistinctAccountCount")}: ${formatOptionalText(
      record.poolDistinctAccountCount != null
        ? String(record.poolDistinctAccountCount)
        : undefined,
    )}`,
    `${t("table.details.poolAttemptTerminalReason")}: ${formatOptionalText(record.poolAttemptTerminalReason)}`,
  ];
  const realAttempts = attempts?.filter(
    (attempt) => !isSyntheticPoolTerminalAttempt(attempt),
  );
  const syntheticTerminalAttempts = attempts?.filter(
    isSyntheticPoolTerminalAttempt,
  );
  const loadedSummaryParts =
    attempts && attempts.length > 0
      ? [
          `${t("table.poolAttempts.realAttemptCount")}: ${realAttempts?.length ?? 0}`,
          `${t("table.poolAttempts.terminalRecordCount")}: ${syntheticTerminalAttempts?.length ?? 0}`,
        ]
      : [];

  return (
    <div className="flex flex-col gap-3" data-testid="pool-attempts-section">
      <div className="space-y-1">
        <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
          {t("table.poolAttempts.title")}
        </span>
        <div className="text-xs text-base-content/60">
          {summaryParts.join(" · ")}
        </div>
        {loadedSummaryParts.length > 0 ? (
          <div className="text-xs text-base-content/60">
            {loadedSummaryParts.join(" · ")}
          </div>
        ) : null}
      </div>

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
            const statusMeta = poolAttemptStatusMeta(attempt.status);
            const phase = resolvePoolAttemptPhase(attempt);
            const phaseMeta = poolAttemptPhaseMeta(phase);
            const accountLabel = formatPoolAttemptAccountLabel(attempt);
            const httpStatusValue = formatOptionalStatusCode(
              attempt.httpStatus,
            );
            const downstreamHttpStatusValue = formatOptionalStatusCode(
              attempt.downstreamHttpStatus,
            );
            const proxyBindingDisplay = formatPoolAttemptProxyBindingDisplay(
              attempt,
              proxyBindingNodesByKey,
            );

            return (
              <div
                key={`${attempt.id}-${attempt.attemptIndex}`}
                className="rounded-lg border border-base-300/70 bg-base-100/70 p-3"
                data-testid="pool-attempt-item"
              >
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant={statusMeta.variant}>
                    {t(statusMeta.key)}
                  </Badge>
                  {!isPoolAttemptTerminal(attempt) ? (
                    <Badge
                      variant={phaseMeta.variant}
                      data-testid="pool-attempt-phase-badge"
                    >
                      {t(phaseMeta.key)}
                    </Badge>
                  ) : null}
                  <span className="font-mono text-xs text-base-content/70">
                    #{attempt.attemptIndex}
                  </span>
                  <span className="text-sm font-medium">{accountLabel}</span>
                </div>
                <div className="mt-2 grid gap-2 text-sm md:grid-cols-2 xl:grid-cols-3">
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t("table.poolAttempts.retry")}
                    </span>
                    <span className="font-mono">
                      {attempt.sameAccountRetryIndex}/
                      {attempt.distinctAccountIndex}
                    </span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t("table.poolAttempts.proxy")}
                    </span>
                    <span
                      className={cn(
                        "min-w-0 truncate whitespace-nowrap",
                        proxyBindingDisplay.resolved
                          ? "font-medium"
                          : "font-mono",
                      )}
                      title={proxyBindingDisplay.title}
                      data-testid="pool-attempt-proxy-value"
                    >
                      {proxyBindingDisplay.value}
                    </span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t("table.poolAttempts.upstreamHttpStatus")}
                    </span>
                    <span className="font-mono">{httpStatusValue}</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t("table.poolAttempts.downstreamHttpStatus")}
                    </span>
                    <span className="font-mono">
                      {downstreamHttpStatusValue}
                    </span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t("table.poolAttempts.failureKind")}
                    </span>
                    <span className="break-all font-mono">
                      {formatOptionalText(attempt.failureKind)}
                    </span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t("table.poolAttempts.connectLatency")}
                    </span>
                    <span className="font-mono">
                      {formatMilliseconds(attempt.connectLatencyMs)}
                    </span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t("table.poolAttempts.firstByteLatency")}
                    </span>
                    <span className="font-mono">
                      {formatMilliseconds(attempt.firstByteLatencyMs)}
                    </span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t("table.poolAttempts.streamLatency")}
                    </span>
                    <span className="font-mono">
                      {formatMilliseconds(attempt.streamLatencyMs)}
                    </span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t("table.poolAttempts.startedAt")}
                    </span>
                    <span className="font-mono">
                      {formatDetailTimestamp(attempt.startedAt)}
                    </span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t("table.poolAttempts.finishedAt")}
                    </span>
                    <span className="font-mono">
                      {formatDetailTimestamp(attempt.finishedAt)}
                    </span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t("table.poolAttempts.upstreamRequestId")}
                    </span>
                    <span className="break-all font-mono">
                      {formatOptionalText(attempt.upstreamRequestId)}
                    </span>
                  </div>
                </div>
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
              </div>
            );
              })}
            </div>
          ) : null}
          {syntheticTerminalAttempts?.map((attempt) => {
            const statusMeta = poolAttemptStatusMeta(attempt.status);
            const accountLabel = formatPoolAttemptAccountLabel(attempt);
            const httpStatusValue = formatOptionalStatusCode(
              attempt.httpStatus,
            );
            const distinctAccountValue =
              attempt.distinctAccountIndex > 0
                ? String(attempt.distinctAccountIndex)
                : record.poolDistinctAccountCount != null
                  ? String(record.poolDistinctAccountCount)
                  : undefined;

            return (
              <div
                key={`terminal-${attempt.id}-${attempt.attemptIndex}`}
                className="overflow-hidden rounded-xl border border-warning/40 bg-warning/10 shadow-sm"
                data-testid="pool-attempt-terminal-record"
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
                        <Badge variant={statusMeta.variant}>
                          {t(statusMeta.key)}
                        </Badge>
                      </div>
                      <p className="mt-1 max-w-3xl text-sm text-base-content/75">
                        {t(
                          poolAttemptTerminalDescriptionKey(
                            attempt.failureKind ??
                              record.poolAttemptTerminalReason,
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
                    <span className="mt-1 block font-mono">
                      {httpStatusValue}
                    </span>
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
  );
}

function resolveUnavailableResponseBodyMessage(
  reason: string | null | undefined,
  t: Translator,
) {
  const normalized = reason?.trim().toLowerCase() ?? "";
  if (normalized === "not_abnormal")
    return t("table.responseBody.unavailable.notAbnormal");
  if (normalized === "detail_pruned")
    return t("table.responseBody.unavailable.detailPruned");
  if (normalized.startsWith("raw_file_missing"))
    return t("table.responseBody.unavailable.rawFileMissing");
  if (normalized.startsWith("raw_file_unreadable"))
    return t("table.responseBody.unavailable.rawFileUnreadable");
  if (normalized.startsWith("preview_only"))
    return t("table.responseBody.unavailable.previewOnly");
  if (normalized.startsWith("missing_body"))
    return t("table.responseBody.unavailable.missingBody");
  return t("table.responseBody.unavailable.generic");
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
  const showUpstreamErrorSection = Boolean(
    canonicalUpstreamError || upstreamRawError,
  );
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

  return (
    <div
      id={detailId}
      className={cn("flex flex-col gap-4", size === "compact" ? "p-3" : "p-4")}
    >
      {detailNotice ? (
        <div
          className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs leading-5 text-warning"
          data-testid="invocation-detail-notice"
        >
          {detailNotice}
        </div>
      ) : null}

      <div className="flex flex-col gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
          {t("table.detailsTitle")}
        </span>
        <div className="grid gap-2 md:grid-cols-2">
          {detailPairs.map((entry) => (
            <div key={entry.key} className="flex items-start gap-2">
              <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60 md:min-w-36">
                {entry.label}
              </span>
              <div className="min-w-0 break-all font-mono text-sm">
                {entry.value}
              </div>
            </div>
          ))}
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
          {t("table.details.timingsTitle")}
        </span>
        <div className="grid gap-2 md:grid-cols-2">
          {timingPairs.map((entry) => (
            <div key={entry.label} className="flex items-start gap-2">
              <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60 md:min-w-36">
                {entry.label}
              </span>
              <span className="font-mono text-sm">{entry.value}</span>
            </div>
          ))}
        </div>
      </div>

      {showResponseBodySection ? (
        <div
          className="flex flex-col gap-2"
          data-testid="invocation-response-body-section"
        >
          <div className="flex flex-wrap items-center justify-between gap-3">
            <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
              {t("table.responseBody.title")}
            </span>
            {showFullDetailsAction && onOpenFullDetails ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={onOpenFullDetails}
              >
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
          ) : abnormalResponseBody?.available &&
            abnormalResponseBody.previewText ? (
            <>
              <pre
                className="max-h-72 overflow-auto rounded-lg border border-base-300/70 bg-base-100/70 p-3 whitespace-pre-wrap break-words font-mono text-sm"
                data-testid="invocation-response-body-preview"
              >
                {abnormalResponseBody.previewText}
              </pre>
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
              {resolveUnavailableResponseBodyMessage(
                abnormalResponseBody.unavailableReason,
                t,
              )}
            </div>
          ) : null}
        </div>
      ) : null}

      {renderPoolAttemptsContent(
        record,
        poolAttemptsState,
        poolAttemptProxyBindingNodesByKey,
        t,
      )}

      {showUpstreamErrorSection ? (
        <div className="flex flex-col gap-2">
          <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
            {t("table.upstreamErrorDetailsTitle")}
          </span>
          {canonicalUpstreamError ? (
            <div
              className="flex flex-col gap-1"
              data-testid="invocation-upstream-error-section"
            >
              <span className="text-[11px] font-semibold uppercase tracking-wide text-base-content/60">
                {t("table.upstreamCanonicalErrorLabel")}
              </span>
              <pre className="whitespace-pre-wrap break-words font-mono text-sm">
                {canonicalUpstreamError}
              </pre>
            </div>
          ) : null}
          {upstreamRawError && upstreamRawError !== canonicalUpstreamError ? (
            <div className="flex flex-col gap-1">
              <span className="text-[11px] font-semibold uppercase tracking-wide text-base-content/60">
                {t("table.details.upstreamErrorMessage")}
              </span>
              <pre className="whitespace-pre-wrap break-words font-mono text-sm">
                {upstreamRawError}
              </pre>
            </div>
          ) : null}
        </div>
      ) : null}

      {showDownstreamErrorSection ? (
        <div
          className="flex flex-col gap-2"
          data-testid="invocation-downstream-error-section"
        >
          <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
            {t("table.downstreamErrorDetailsTitle")}
          </span>
          <div className="grid gap-2 md:grid-cols-2">
            <div className="flex items-start gap-2">
              <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60 md:min-w-36">
                {t("table.details.downstreamStatusCode")}
              </span>
              <span className="font-mono text-sm">
                {formatOptionalStatusCode(record.downstreamStatusCode)}
              </span>
            </div>
          </div>
          {downstreamErrorMessage ? (
            <div className="flex flex-col gap-1">
              <span className="text-[11px] font-semibold uppercase tracking-wide text-base-content/60">
                {t("table.details.downstreamErrorMessage")}
              </span>
              <pre className="whitespace-pre-wrap break-words font-mono text-sm">
                {downstreamErrorMessage}
              </pre>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
