import {
  Fragment,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { Link } from "react-router-dom";
import {
  Bar,
  CartesianGrid,
  ComposedChart,
  Line,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../../components/ui/dialog";
import {
  type FloatingSurfaceTheme,
  floatingSurfaceStyle,
} from "../../components/ui/floating-surface";
import { Input } from "../../components/ui/input";
import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import { SelectField } from "../../components/ui/select-field";
import { Spinner } from "../../components/ui/spinner";
import {
  type InvocationHistoryOverviewTopicPayload,
  resolveConversationDetailScope,
  useConversationDetailTopics,
} from "../../hooks/useConversationDetailTopics";
import { useTranslation } from "../../i18n";
import type {
  ApiInvocation,
  EffectiveRoutingRule,
  EffectiveRoutingRuleSource,
  ForwardProxyBindingNode,
  InvocationRecordsQuery,
  InvocationRecordsResponse,
  InvocationRecordsSummaryResponse,
  PoolRoutingSelectionAudit,
  PoolRoutingSelectionScoreSnapshot,
  PromptCacheConversation,
  PromptCacheConversationBindingKind,
  PromptCacheConversationBindingResponse,
  PromptCacheConversationOperationEvent,
  PromptCacheConversationOperationInfoType,
  PromptCacheConversationRewriteMode,
  PromptCacheConversationsResponse,
  PromptCacheConversationUpstreamAccount,
  UpdateGroupAccountRoutingRulePayload,
  UpstreamAccountSummary,
} from "../../lib/api";
import {
  fetchInvocationRecords,
  fetchInvocationRecordsSummary,
  fetchPromptCacheConversationBinding,
  fetchPromptCacheConversationOperationEvents,
  fetchUpstreamAccounts,
  updatePromptCacheConversationBinding,
} from "../../lib/api";
import { chartBaseTokens, chartStatusTokens, metricAccent } from "../../lib/chartTheme";
import { resolvePromptCacheInvocationOutcome } from "../../lib/conversationRequestPoint";
import { invocationStableKey } from "../../lib/invocation";
import { mergeInvocationRecordCollections } from "../../lib/invocationLiveMerge";
import { buildInvocationFromPromptCachePreview } from "../../lib/promptCacheLive";
import type { ThemeMode } from "../../theme";
import { AccountDetailDrawerShell } from "../account-pool/AccountDetailDrawerShell";
import {
  EffectiveRoutingRuleCard,
  type EffectiveRoutingRuleCardRowKey,
  type EffectiveRoutingRuleCardRowValueOverride,
} from "../account-pool/EffectiveRoutingRuleCard";
import { InvocationTable } from "../invocations/InvocationTable";
import { AppIcon } from "../shared/AppIcon";
import { ConversationSparkline } from "./KeyedConversationTable";
import { FALLBACK_CELL, findVisibleConversationChartMax } from "./keyedConversationChart";

interface PromptCacheConversationTableProps {
  stats: PromptCacheConversationsResponse | null;
  isLoading: boolean;
  error?: string | null;
  expandedPromptCacheKeys?: string[];
  onToggleExpandedPromptCacheKey?: (promptCacheKey: string) => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  keyColumnLabel?: string;
  emptyLabel?: string;
  historyQueryForConversationKey?: (conversationKey: string) => Partial<InvocationRecordsQuery>;
}

type ConversationHistoryQueryBuilder = NonNullable<
  PromptCacheConversationTableProps["historyQueryForConversationKey"]
>;

const PROMPT_CACHE_NOW_TICK_MS = 30_000;
const PROMPT_CACHE_CHART_MAX_WINDOW_MS = 24 * 3_600_000;
const PROMPT_CACHE_HISTORY_PAGE_SIZE = 50;
const PROMPT_CACHE_OPERATION_EVENT_PAGE_SIZE = 20;
const PROMPT_CACHE_ACTIVITY_MAX_CHART_RECORDS = 1_000;
const PROMPT_CACHE_HISTORY_TOP_INSERT_THRESHOLD_PX = 96;
const CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS = 30;
const CONVERSATION_ACTIVITY_WHEEL_THRESHOLD = 2;
const CONVERSATION_ACTIVITY_WHEEL_ZOOM_INTENSITY = 0.0018;
const CONVERSATION_ACTIVITY_WHEEL_PAN_INTENSITY = 0.012;
const CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_THRESHOLD_PX = 8;
const CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_RATIO = 1.45;
const CONVERSATION_ACTIVITY_POINTER_FREE_DIAGONAL_RATIO = 0.72;

type ConversationActivityRange = "today" | "yesterday" | "1d" | "7d" | "history";
type ConversationActivityMetric = "totalCount" | "totalCost" | "totalTokens";
type ConversationActivityDragAxis = "pending" | "horizontal" | "vertical" | "free";
type ConversationBindingDraftKind = PromptCacheConversationBindingKind;
export type PromptCacheConversationDrawerTab = "overview" | "calls" | "settings" | "operations";
type PromptCacheConversationOperationFilter = "all" | PromptCacheConversationOperationInfoType;
type OptionalBooleanDraft = "inherit" | "true" | "false";
type RewriteModeDraft = PromptCacheConversationRewriteMode;
type ConversationInlinePolicyField =
  | "allowCutOut"
  | "fastModeRewriteMode"
  | "imageToolRewriteMode"
  | "codexImagegenRewriteMode"
  | "availableModels"
  | "proxyBindings"
  | "timeoutResponsesFirstByte"
  | "timeoutCompactFirstByte"
  | "timeoutImageFirstByte"
  | "timeoutResponsesStream"
  | "timeoutCompactStream";

const CONVERSATION_ACTIVITY_METRICS: Array<{
  key: ConversationActivityMetric;
  labelKey: string;
}> = [
  { key: "totalCount", labelKey: "metric.totalCount" },
  { key: "totalCost", labelKey: "metric.totalCost" },
  { key: "totalTokens", labelKey: "metric.totalTokens" },
];

const PROMPT_CACHE_OPERATION_FILTER_OPTIONS: Array<{
  value: PromptCacheConversationOperationFilter;
  labelKey: string;
}> = [
  { value: "all", labelKey: "live.conversations.drawer.operations.filters.all" },
  { value: "routing", labelKey: "live.conversations.drawer.operations.filters.routing" },
  {
    value: "forwardProxy",
    labelKey: "live.conversations.drawer.operations.filters.forwardProxy",
  },
  {
    value: "requestRewrite",
    labelKey: "live.conversations.drawer.operations.filters.requestRewrite",
  },
];

function parseEpoch(raw?: string | null) {
  if (!raw) return null;
  const epoch = Date.parse(raw);
  return Number.isNaN(epoch) ? null : epoch;
}

function formatNumber(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return formatter.format(value);
}

function formatCurrency(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return formatter.format(value);
}

function formatDateLabel(raw: string, formatter: Intl.DateTimeFormat) {
  const value = new Date(raw);
  if (Number.isNaN(value.getTime())) return raw || FALLBACK_CELL;
  return formatter.format(value);
}

function conversationBindingAccountLabel(account: UpstreamAccountSummary) {
  const identity = account.email?.trim() || account.displayName.trim();
  const group = account.groupName?.trim();
  return group ? `${identity} · ${group}` : identity;
}

function conversationForwardProxyLabel(node: ForwardProxyBindingNode) {
  return node.protocolLabel ? `${node.displayName} · ${node.protocolLabel}` : node.displayName;
}

function normalizeConversationProxyKeys(values?: string[] | null): string[] {
  if (!Array.isArray(values)) return [];
  return Array.from(
    new Set(values.map((value) => value.trim()).filter((value) => value.length > 0)),
  );
}

function toggleConversationProxyKey(keys: string[], target: string): string[] {
  return keys.includes(target) ? keys.filter((key) => key !== target) : [...keys, target];
}

function splitConversationModelsDraft(value: string) {
  return value
    .split(/[\n,]/)
    .map((item) => item.trim())
    .filter((item, index, all) => item.length > 0 && all.indexOf(item) === index);
}

function applyBindingPolicyDraft(
  nextBinding: PromptCacheConversationBindingResponse,
  setters: {
    setAllowSwitchUpstreamDraft: (value: OptionalBooleanDraft) => void;
    setFastModeDraft: (value: RewriteModeDraft) => void;
    setImageToolDraft: (value: RewriteModeDraft) => void;
    setCodexImagegenDraft: (value: RewriteModeDraft) => void;
    setAvailableModelsMode: (value: "inherit" | "override") => void;
    setAvailableModelsDraft: (value: string) => void;
    setForwardProxyKeysDraft: (value: string[]) => void;
  },
) {
  setters.setAllowSwitchUpstreamDraft(
    nextBinding.allowSwitchUpstream == null
      ? "inherit"
      : nextBinding.allowSwitchUpstream
        ? "true"
        : "false",
  );
  setters.setFastModeDraft(nextBinding.fastModeRewriteMode ?? "keep_original");
  setters.setImageToolDraft(nextBinding.imageToolRewriteMode ?? "keep_original");
  setters.setCodexImagegenDraft(nextBinding.codexImagegenRewriteMode ?? "keep_original");
  setters.setAvailableModelsMode(nextBinding.availableModels == null ? "inherit" : "override");
  setters.setAvailableModelsDraft((nextBinding.availableModels ?? []).join(", "));
  setters.setForwardProxyKeysDraft(normalizeConversationProxyKeys(nextBinding.forwardProxyKeys));
}

function buildConversationEffectiveRoutingRule(
  binding: PromptCacheConversationBindingResponse | null,
): EffectiveRoutingRule | null {
  if (!binding) return null;
  return {
    allowCutOut: binding.allowSwitchUpstream ?? true,
    allowCutIn: true,
    priorityTier: "normal",
    fastModeRewriteMode: binding.fastModeRewriteMode ?? "keep_original",
    imageToolRewriteMode: binding.imageToolRewriteMode ?? "keep_original",
    codexImagegenRewriteMode: binding.codexImagegenRewriteMode ?? "keep_original",
    concurrencyLimit: 0,
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
    availableModels: binding.availableModels ?? [],
    systemDeniedModels: [],
    sourceTagIds: [],
    sourceTagNames: [],
    fieldSources: {
      allowCutOut: binding.policyFieldSources?.allowSwitchUpstream ?? "account",
      allowCutIn: "root",
      priorityTier: "root",
      fastModeRewriteMode: binding.policyFieldSources?.fastModeRewriteMode ?? "account",
      imageToolRewriteMode: binding.policyFieldSources?.imageToolRewriteMode ?? "account",
      codexImagegenRewriteMode: binding.policyFieldSources?.codexImagegenRewriteMode ?? "account",
      concurrencyLimit: "root",
      upstream429Retry: "root",
      availableModels: binding.policyFieldSources?.availableModels ?? "account",
      systemDeniedModels: "root",
    },
    timeouts: binding.timeouts,
    timeoutFieldSources: binding.timeoutFieldSources,
  };
}

function buildConversationRowValueOverrides(
  binding: PromptCacheConversationBindingResponse | null,
  t: (key: string) => string,
): Partial<Record<EffectiveRoutingRuleCardRowKey, EffectiveRoutingRuleCardRowValueOverride>> {
  if (!binding) return {};

  const overrides: Partial<
    Record<EffectiveRoutingRuleCardRowKey, EffectiveRoutingRuleCardRowValueOverride>
  > = {};

  if (binding.allowSwitchUpstream == null) {
    overrides.allowCutOut = {
      value: t("live.conversations.drawer.policy.cutOutInherited"),
      valueVariant: "secondary",
    };
  }

  if (binding.fastModeRewriteMode == null) {
    overrides.fastModeRewriteMode = {
      value: t("live.conversations.drawer.policy.rewriteInherited"),
    };
  }

  if (binding.imageToolRewriteMode == null) {
    overrides.imageToolRewriteMode = {
      value: t("live.conversations.drawer.policy.rewriteInherited"),
    };
  }
  if (binding.codexImagegenRewriteMode == null) {
    overrides.codexImagegenRewriteMode = {
      value: t("live.conversations.drawer.policy.rewriteInherited"),
    };
  }

  if (binding.availableModels == null) {
    overrides.availableModels = {
      value: t("accountPool.upstreamAccounts.effectiveRule.availableModelsInherited"),
    };
  }

  return overrides;
}

function conversationProxySource(
  binding: PromptCacheConversationBindingResponse | null,
): EffectiveRoutingRuleSource {
  return binding?.policyFieldSources?.forwardProxyKey ?? "account";
}

function mapTimeoutFieldToInlineField(
  key: keyof NonNullable<UpdateGroupAccountRoutingRulePayload["timeouts"]>,
): ConversationInlinePolicyField {
  switch (key) {
    case "responsesFirstByteTimeoutSecs":
      return "timeoutResponsesFirstByte";
    case "compactFirstByteTimeoutSecs":
      return "timeoutCompactFirstByte";
    case "imageFirstByteTimeoutSecs":
      return "timeoutImageFirstByte";
    case "responsesStreamTimeoutSecs":
      return "timeoutResponsesStream";
    case "compactStreamTimeoutSecs":
      return "timeoutCompactStream";
  }
  return "timeoutResponsesFirstByte";
}

function accountCanBePromptCacheBindingTarget(account: UpstreamAccountSummary) {
  if (account.provider !== "codex" || !account.enabled || account.status !== "active") {
    return false;
  }
  if (account.kind === "api_key_codex") {
    return Boolean(account.maskedApiKey?.trim());
  }
  if (account.kind === "oauth_codex") {
    return account.hasRefreshToken !== false;
  }
  return true;
}

function currentBindingLabel(
  binding: PromptCacheConversationBindingResponse | null,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (!binding || binding.bindingKind === "none") {
    return t("live.conversations.drawer.binding.currentNone");
  }
  if (binding.bindingKind === "group" && binding.groupName) {
    return t("live.conversations.drawer.binding.currentGroup", {
      group: binding.groupName,
    });
  }
  if (binding.bindingKind === "upstreamAccount" && binding.upstreamAccountId != null) {
    return t("live.conversations.drawer.binding.currentAccount", {
      account: binding.upstreamAccountName || `#${binding.upstreamAccountId}`,
    });
  }
  return t("live.conversations.drawer.binding.currentNone");
}

function encryptedOwnerLabel(binding: PromptCacheConversationBindingResponse | null) {
  if (!binding?.hasEncryptedSessionOwner) return null;
  const accountLabel =
    binding.encryptedOwnerAccountName?.trim() ||
    (binding.encryptedOwnerAccountId != null ? `#${binding.encryptedOwnerAccountId}` : null);
  if (!accountLabel) return null;
  const groupLabel = binding.encryptedOwnerGroupName?.trim();
  return groupLabel ? `${accountLabel} · ${groupLabel}` : accountLabel;
}

function formatConversationOperationOccurredAt(raw: string) {
  const value = new Date(raw);
  if (Number.isNaN(value.getTime())) {
    return raw || FALLBACK_CELL;
  }
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(value);
}

function conversationOperationInfoTypeBadgeVariant(
  infoType: PromptCacheConversationOperationInfoType,
): "default" | "info" | "accent" {
  switch (infoType) {
    case "routing":
      return "default";
    case "forwardProxy":
      return "info";
    case "requestRewrite":
      return "accent";
  }
}

function conversationOperationOriginBadgeVariant(origin: string): "secondary" | "warning" | "info" {
  switch (origin) {
    case "dashboardBulk":
      return "warning";
    case "systemAuto":
      return "info";
    default:
      return "secondary";
  }
}

function conversationOperationActionLabel(
  action: PromptCacheConversationOperationEvent["action"],
  headline: string,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const key = `live.conversations.drawer.operations.actions.${action}`;
  const translated = t(key);
  return translated === key ? headline : translated;
}

function conversationOperationInfoTypeLabel(
  infoType: PromptCacheConversationOperationInfoType,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  return t(`live.conversations.drawer.operations.filters.${infoType}`);
}

function conversationOperationOriginLabel(
  origin: string,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const key = `live.conversations.drawer.operations.origins.${origin}`;
  const translated = t(key);
  return translated === key ? origin : translated;
}

function conversationOperationRoutingReasonLabel(
  event: PromptCacheConversationOperationEvent,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const context = event.routingContext;
  if (!context) return t("live.conversations.drawer.operations.routingContext.legacy");
  const key = `live.conversations.drawer.operations.routingContext.reasons.${context.reasonCode}`;
  const translated = t(key, {
    status: context.causingHttpStatus ?? context.httpStatus ?? "-",
  });
  return translated === key
    ? t("live.conversations.drawer.operations.routingContext.reasons.unknown")
    : translated;
}

function routingSelectionWinnerLabel(
  audit: PoolRoutingSelectionAudit,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (!audit.selectedScore) {
    return t("table.poolAttempts.routingDecision.auditUnavailable", {
      account: audit.selectedAccountName,
      comparedAccount: audit.comparedAccountName ?? "-",
    });
  }
  const key = `table.poolAttempts.routingDecision.winnerReasons.${audit.winnerReasonCode}`;
  const translated = t(key, {
    account: audit.selectedAccountName,
    comparedAccount: audit.comparedAccountName ?? "-",
  });
  return translated === key
    ? t("table.poolAttempts.routingDecision.winnerReasons.unknown", {
        account: audit.selectedAccountName,
        comparedAccount: audit.comparedAccountName ?? "-",
      })
    : translated;
}

function routingSelectionScoreLabel(
  account: string,
  score: PoolRoutingSelectionScoreSnapshot,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  return t("table.poolAttempts.routingDecision.score", {
    account,
    modelPenalty: score.modelRoutePenalty,
    modelPenaltyCode: score.modelRoutePenaltyCode,
    routeFailurePenalty: score.routeBindingFailurePenalty,
    priority: score.routingPriorityRank,
    capacityLane: score.capacityLane,
    dispatchState: score.dispatchState,
    effectiveLoad: score.effectiveLoad,
    scarcityScore: score.scarcityScore,
  });
}

function routingSelectionExclusionLabel(
  account: string,
  reasonCode: string,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const key = `table.poolAttempts.routingDecision.exclusionReasons.${reasonCode}`;
  const translated = t(key, { account });
  return translated === key
    ? t("table.poolAttempts.routingDecision.exclusionReasons.unknown", { account })
    : translated;
}

function routingAttemptHref(event: PromptCacheConversationOperationEvent, attemptId: string) {
  const search = new URLSearchParams({ attemptId });
  if (event.invokeId && event.routingContext?.triggerAttemptId === attemptId) {
    search.set("invokeId", event.invokeId);
  }
  return `/records?${search.toString()}`;
}

function routingInvocationRecordHref(event: PromptCacheConversationOperationEvent) {
  const search = new URLSearchParams();
  const triggerAttemptId = event.routingContext?.triggerAttemptId;
  if (triggerAttemptId) search.set("attemptId", triggerAttemptId);
  if (event.invokeId) search.set("invokeId", event.invokeId);
  return search.toString() ? `/records?${search.toString()}` : null;
}

function conversationOperationShowsRoutingReason(event: PromptCacheConversationOperationEvent) {
  return (
    event.routingContext != null ||
    (event.origin === "systemAuto" &&
      (event.action === "stickyTargetChanged" ||
        event.action === "stickyTargetCleared" ||
        event.action === "stickyMutationSuppressed"))
  );
}

function conversationOperationBindingSnapshotLabel(
  snapshot:
    | PromptCacheConversationOperationEvent["bindingBefore"]
    | PromptCacheConversationOperationEvent["bindingAfter"],
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (!snapshot || snapshot.bindingKind === "none") {
    return t("live.conversations.drawer.operations.binding.none");
  }
  if (snapshot.bindingKind === "group" && snapshot.groupName) {
    return t("live.conversations.drawer.operations.binding.group", {
      group: snapshot.groupName,
    });
  }
  const accountLabel =
    snapshot.upstreamAccountName?.trim() ||
    (snapshot.upstreamAccountId != null ? `#${snapshot.upstreamAccountId}` : null);
  if (!accountLabel) {
    return t("live.conversations.drawer.operations.binding.none");
  }
  return t("live.conversations.drawer.operations.binding.account", {
    account: accountLabel,
  });
}

function conversationOperationStickySnapshotLabel(
  snapshot:
    | PromptCacheConversationOperationEvent["stickyBefore"]
    | PromptCacheConversationOperationEvent["stickyAfter"],
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (!snapshot) {
    return t("live.conversations.drawer.operations.sticky.none");
  }
  return snapshot.upstreamAccountName?.trim() || `#${snapshot.upstreamAccountId}`;
}

function conversationOperationChangedFieldLabel(
  field: string,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const key = `live.conversations.drawer.operations.fields.${field}`;
  const translated = t(key);
  return translated === key ? field : translated;
}

function nextBindingWouldOverrideEncryptedOwner(
  binding: PromptCacheConversationBindingResponse | null,
  nextBindingKind: ConversationBindingDraftKind,
  nextBindingGroupName: string,
  nextBindingAccountId: string,
) {
  if (!binding?.hasEncryptedSessionOwner) return false;
  if (nextBindingKind === "none") return false;
  if (nextBindingKind === "upstreamAccount") {
    const nextId = Number(nextBindingAccountId);
    return Number.isFinite(nextId) && nextId !== binding.encryptedOwnerAccountId;
  }
  if (nextBindingKind === "group") {
    return nextBindingGroupName.trim().length > 0;
  }
  return false;
}

function resolveUpstreamAccountLabel(
  account: PromptCacheConversationUpstreamAccount,
  fallbackAccountLabel: (id: number) => string,
) {
  const trimmedName = account.upstreamAccountName?.trim();
  if (trimmedName) return trimmedName;
  if (typeof account.upstreamAccountId === "number" && Number.isFinite(account.upstreamAccountId)) {
    return fallbackAccountLabel(Math.trunc(account.upstreamAccountId));
  }
  return FALLBACK_CELL;
}

function canOpenPromptCacheUpstreamAccount(account: PromptCacheConversationUpstreamAccount) {
  return (
    typeof account.upstreamAccountId === "number" && Number.isFinite(account.upstreamAccountId)
  );
}

function SummaryBlock({
  conversation,
  labels,
  numberFormatter,
  currencyFormatter,
}: {
  conversation: PromptCacheConversation;
  labels: {
    requestCount: string;
    totalTokens: string;
    totalCost: string;
  };
  numberFormatter: Intl.NumberFormat;
  currencyFormatter: Intl.NumberFormat;
}) {
  const items = [
    {
      label: labels.requestCount,
      value: formatNumber(conversation.requestCount, numberFormatter),
    },
    {
      label: labels.totalTokens,
      value: formatNumber(conversation.totalTokens, numberFormatter),
    },
    {
      label: labels.totalCost,
      value: formatCurrency(conversation.totalCost, currencyFormatter),
    },
  ];

  return (
    <div className="space-y-1.5">
      {items.map((item) => (
        <div key={item.label} className="flex items-center justify-between gap-3 text-[11px]">
          <span className="text-base-content/60">{item.label}</span>
          <span className="text-right font-medium">{item.value}</span>
        </div>
      ))}
    </div>
  );
}

function UpstreamAccountsBlock({
  upstreamAccounts,
  labels,
  numberFormatter,
  currencyFormatter,
  fallbackAccountLabel,
  onOpenAccountDetail,
}: {
  upstreamAccounts: PromptCacheConversationUpstreamAccount[];
  labels: {
    requestCountCompact: string;
    totalTokensCompact: string;
  };
  numberFormatter: Intl.NumberFormat;
  currencyFormatter: Intl.NumberFormat;
  fallbackAccountLabel: (id: number) => string;
  onOpenAccountDetail?: (account: PromptCacheConversationUpstreamAccount) => void;
}) {
  if (upstreamAccounts.length === 0) {
    return <div className="text-[11px] text-base-content/55">{FALLBACK_CELL}</div>;
  }

  return (
    <div className="space-y-1.5">
      {upstreamAccounts.slice(0, 3).map((account) => {
        const accountLabel = resolveUpstreamAccountLabel(account, fallbackAccountLabel);
        const clickable = canOpenPromptCacheUpstreamAccount(account);
        const accountKey = [
          account.upstreamAccountId ?? "unknown",
          account.upstreamAccountName ?? "none",
          account.requestCount,
          account.totalTokens,
          account.totalCost,
        ].join(":");

        return (
          <div
            key={accountKey}
            className="grid grid-cols-[7.5rem_minmax(0,1fr)] items-center gap-x-2 text-[11px]"
          >
            {clickable ? (
              <button
                type="button"
                className="truncate text-left font-medium transition hover:text-primary hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                onClick={() => onOpenAccountDetail?.(account)}
                title={accountLabel}
              >
                {accountLabel}
              </button>
            ) : (
              <span className="truncate font-medium">{accountLabel}</span>
            )}
            <span className="min-w-0 truncate text-base-content/62">
              {formatNumber(account.requestCount, numberFormatter)} {labels.requestCountCompact}
              {" · "}
              {labels.totalTokensCompact} {formatNumber(account.totalTokens, numberFormatter)}
              {" · "}
              {formatCurrency(account.totalCost, currencyFormatter)}
            </span>
          </div>
        );
      })}
    </div>
  );
}

function PromptCacheConversationInvocationTable({
  records,
  isLoading,
  error,
  emptyLabel,
  onOpenUpstreamAccount,
  scrollElement,
}: {
  records: ApiInvocation[];
  isLoading: boolean;
  error?: string | null;
  emptyLabel: string;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  scrollElement?: HTMLElement | null;
}) {
  const hasLoadedRecords = records.length > 0;

  if (hasLoadedRecords) {
    return (
      <div className="space-y-3">
        {error ? (
          <Alert variant="error">
            <span>{error}</span>
          </Alert>
        ) : null}
        <InvocationTable
          records={records}
          isLoading={false}
          error={null}
          emptyLabel={emptyLabel}
          onOpenUpstreamAccount={onOpenUpstreamAccount}
          scrollElement={scrollElement}
        />
      </div>
    );
  }

  return (
    <InvocationTable
      records={records}
      isLoading={isLoading}
      error={error}
      emptyLabel={emptyLabel}
      onOpenUpstreamAccount={onOpenUpstreamAccount}
      scrollElement={scrollElement}
    />
  );
}

function resolveConversationActivityRange(range: ConversationActivityRange) {
  if (range === "history") return {};

  const now = new Date();
  if (range === "today") {
    return {
      from: startOfLocalDay(now).toISOString(),
      to: now.toISOString(),
    };
  }
  if (range === "yesterday") {
    const end = startOfLocalDay(now);
    const start = new Date(end);
    start.setDate(start.getDate() - 1);
    return {
      from: start.toISOString(),
      to: end.toISOString(),
    };
  }
  const durationMs = range === "7d" ? 7 * 86_400_000 : 86_400_000;
  return {
    from: new Date(now.getTime() - durationMs).toISOString(),
    to: now.toISOString(),
  };
}

function startOfLocalDay(value: Date) {
  const next = new Date(value);
  next.setHours(0, 0, 0, 0);
  return next;
}

function formatCompactNumber(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return formatter.format(value);
}

function formatDurationMs(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  const seconds = value / 1000;
  const maximumFractionDigits = Math.abs(seconds) >= 10 ? 1 : 2;
  return `${formatter.format(Number(seconds.toFixed(maximumFractionDigits)))} s`;
}

function getConversationActivityValue(record: ApiInvocation, metric: ConversationActivityMetric) {
  if (metric === "totalCost") return record.cost ?? 0;
  if (metric === "totalTokens") return record.totalTokens ?? 0;
  return 1;
}

interface ConversationActivityBucket {
  index: number;
  label: string;
  tooltipLabel: string;
  success: number;
  failure: number;
  failureNegative: number;
  inFlight: number;
  neutral: number;
  totalCount: number;
  totalCost: number;
  totalTokens: number;
  totalMs: number;
  totalMsSamples: number;
  avgTotalMs: number | null;
}

interface ConversationActivityBucketSet {
  buckets: ConversationActivityBucket[];
  rangeStartMs: number;
  rangeEndMs: number;
}

function resolveDocumentThemeMode(): ThemeMode {
  if (typeof document === "undefined") return "light";
  const theme =
    document.body.getAttribute("data-theme") ??
    document.documentElement.getAttribute("data-theme") ??
    "";
  const normalizedTheme = theme.toLowerCase();
  if (normalizedTheme.includes("dark")) return "dark";
  if (normalizedTheme.includes("light")) return "light";
  const colorMode =
    document.body.getAttribute("data-color-mode") ??
    document.documentElement.getAttribute("data-color-mode");
  if (colorMode === "dark" || colorMode === "light") return colorMode;
  return "light";
}

function resolveDocumentFloatingSurfaceTheme(): FloatingSurfaceTheme {
  return resolveDocumentThemeMode() === "dark" ? "vibe-dark" : "vibe-light";
}

interface ConversationActivityViewport {
  startIndex: number;
  endIndex: number;
}

function clampConversationActivityValue(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function normalizeConversationActivityViewport(
  viewport: ConversationActivityViewport,
  pointCount: number,
): ConversationActivityViewport {
  if (pointCount <= 0) {
    return { startIndex: 0, endIndex: 0 };
  }

  const maxIndex = pointCount - 1;
  const minSpan = Math.min(CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS, pointCount);
  const currentSpan = Math.max(
    minSpan,
    Math.min(pointCount, viewport.endIndex - viewport.startIndex + 1),
  );
  const startIndex = clampConversationActivityValue(
    Math.round(viewport.startIndex),
    0,
    Math.max(0, pointCount - currentSpan),
  );

  return {
    startIndex,
    endIndex: Math.min(maxIndex, startIndex + currentSpan - 1),
  };
}

function shiftConversationActivityViewport(
  viewport: ConversationActivityViewport,
  pointCount: number,
  deltaIndexes: number,
): ConversationActivityViewport {
  const span = viewport.endIndex - viewport.startIndex + 1;
  return normalizeConversationActivityViewport(
    {
      startIndex: viewport.startIndex + deltaIndexes,
      endIndex: viewport.startIndex + deltaIndexes + span - 1,
    },
    pointCount,
  );
}

function isSameConversationActivityViewport(
  left: ConversationActivityViewport,
  right: ConversationActivityViewport,
) {
  return left.startIndex === right.startIndex && left.endIndex === right.endIndex;
}

function zoomConversationActivityViewport(
  viewport: ConversationActivityViewport,
  pointCount: number,
  zoomDelta: number,
  anchorRatio: number,
): ConversationActivityViewport {
  if (pointCount <= 0) return viewport;

  const currentSpan = viewport.endIndex - viewport.startIndex + 1;
  const nextSpan = clampConversationActivityValue(
    Math.round(currentSpan * Math.exp(zoomDelta)),
    Math.min(CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS, pointCount),
    pointCount,
  );
  const safeAnchorRatio = clampConversationActivityValue(anchorRatio, 0, 1);
  const anchorIndex = viewport.startIndex + (currentSpan - 1) * safeAnchorRatio;
  const nextStart = Math.round(anchorIndex - (nextSpan - 1) * safeAnchorRatio);

  return normalizeConversationActivityViewport(
    {
      startIndex: nextStart,
      endIndex: nextStart + nextSpan - 1,
    },
    pointCount,
  );
}

function buildConversationActivityBuckets({
  records,
  range,
  metric,
  localeTag,
  rangeStartMs,
  rangeEndMs,
}: {
  records: ApiInvocation[];
  range: ConversationActivityRange;
  metric: ConversationActivityMetric;
  localeTag: string;
  rangeStartMs?: number | null;
  rangeEndMs?: number | null;
}): ConversationActivityBucketSet {
  const now = new Date();
  const rangeBounds = resolveConversationActivityRange(range);
  let startMs =
    typeof rangeStartMs === "number" && Number.isFinite(rangeStartMs)
      ? rangeStartMs
      : rangeBounds.from
        ? Date.parse(rangeBounds.from)
        : Number.POSITIVE_INFINITY;
  let endMs =
    typeof rangeEndMs === "number" && Number.isFinite(rangeEndMs)
      ? rangeEndMs
      : rangeBounds.to
        ? Date.parse(rangeBounds.to)
        : Number.NEGATIVE_INFINITY;

  if (range === "history") {
    for (const record of records) {
      const occurredAt = Date.parse(record.occurredAt);
      if (!Number.isFinite(occurredAt)) continue;
      startMs = Math.min(startMs, occurredAt);
      endMs = Math.max(endMs, occurredAt);
    }
    if (!Number.isFinite(startMs) || !Number.isFinite(endMs)) {
      endMs = now.getTime();
      startMs = endMs - 86_400_000;
    }
  }

  if (!Number.isFinite(startMs) || !Number.isFinite(endMs) || endMs <= startMs) {
    if (
      range === "history" &&
      Number.isFinite(startMs) &&
      Number.isFinite(endMs) &&
      startMs === endMs
    ) {
      endMs = startMs + 60_000;
    } else {
      endMs = now.getTime();
      startMs = endMs - 86_400_000;
    }
  }

  const targetBuckets =
    endMs - startMs <= 86_400_000
      ? Math.ceil((endMs - startMs) / 60_000) + 1
      : range === "today" || range === "yesterday"
        ? 24
        : range === "1d"
          ? 24
          : 720;
  const bucketMs = Math.max(60_000, Math.ceil((endMs - startMs) / targetBuckets));
  const bucketCount = Math.max(1, Math.ceil((endMs - startMs) / bucketMs));
  const labelFormatter = new Intl.DateTimeFormat(localeTag, {
    month: range === "history" || range === "7d" ? "2-digit" : undefined,
    day: range === "history" || range === "7d" ? "2-digit" : undefined,
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    hourCycle: "h23",
  });
  const buckets: ConversationActivityBucket[] = Array.from({ length: bucketCount }, (_, index) => {
    const bucketStart = startMs + index * bucketMs;
    return {
      index,
      label: labelFormatter.format(new Date(bucketStart)),
      tooltipLabel: labelFormatter.format(new Date(bucketStart)),
      success: 0,
      failure: 0,
      failureNegative: 0,
      inFlight: 0,
      neutral: 0,
      totalCount: 0,
      totalCost: 0,
      totalTokens: 0,
      totalMs: 0,
      totalMsSamples: 0,
      avgTotalMs: null,
    };
  });

  for (const record of records) {
    const occurredAt = Date.parse(record.occurredAt);
    if (!Number.isFinite(occurredAt) || occurredAt < startMs || occurredAt > endMs) {
      continue;
    }
    const index = Math.min(
      buckets.length - 1,
      Math.max(0, Math.floor((occurredAt - startMs) / bucketMs)),
    );
    const bucket = buckets[index];
    if (!bucket) continue;
    const outcome = resolvePromptCacheInvocationOutcome(record);
    const metricValue = getConversationActivityValue(record, metric);
    if (outcome === "success") bucket.success += metricValue;
    else if (outcome === "failure") bucket.failure += metricValue;
    else if (outcome === "in_flight") bucket.inFlight += metricValue;
    else bucket.neutral += metricValue;
    bucket.totalCount += 1;
    bucket.totalCost += record.cost ?? 0;
    bucket.totalTokens += record.totalTokens ?? 0;
    if (typeof record.tTotalMs === "number" && Number.isFinite(record.tTotalMs)) {
      bucket.totalMs += record.tTotalMs;
      bucket.totalMsSamples += 1;
    }
  }

  for (const bucket of buckets) {
    bucket.failureNegative = bucket.failure > 0 ? -bucket.failure : 0;
    bucket.avgTotalMs = bucket.totalMsSamples > 0 ? bucket.totalMs / bucket.totalMsSamples : null;
  }

  return { buckets, rangeStartMs: startMs, rangeEndMs: endMs };
}

interface ConversationActivityTooltipPayloadEntry {
  payload?: ConversationActivityBucket;
}

interface ConversationActivityBarShapeProps {
  x?: number | string;
  y?: number | string;
  width?: number | string;
  height?: number | string;
  fill?: string;
}

function renderAlignedFailureBarShape({
  x,
  y,
  width,
  height,
  fill,
}: ConversationActivityBarShapeProps) {
  const numericX = Number(x);
  const numericY = Number(y);
  const numericWidth = Number(width);
  const numericHeight = Number(height);
  if (
    !Number.isFinite(numericX) ||
    !Number.isFinite(numericY) ||
    !Number.isFinite(numericWidth) ||
    !Number.isFinite(numericHeight) ||
    numericWidth <= 0 ||
    numericHeight === 0
  ) {
    return null;
  }
  const left = Math.min(numericX, numericX + numericWidth);
  const right = Math.max(numericX, numericX + numericWidth);
  const top = Math.min(numericY, numericY + numericHeight);
  const bottom = Math.max(numericY, numericY + numericHeight);
  const normalizedWidth = right - left;
  const normalizedHeight = bottom - top;
  const radius = Math.min(3, normalizedWidth / 2, normalizedHeight / 2);

  return (
    <path
      data-conversation-failure-bar-shape="negative"
      d={[
        `M${left},${top}`,
        `H${right}`,
        `V${bottom - radius}`,
        `Q${right},${bottom} ${right - radius},${bottom}`,
        `H${left + radius}`,
        `Q${left},${bottom} ${left},${bottom - radius}`,
        "Z",
      ].join(" ")}
      fill={fill}
      stroke="none"
    />
  );
}

function ConversationActivityTooltipContent({
  active,
  label,
  payload,
  renderValue,
}: {
  active?: boolean;
  label?: string | number;
  payload?: ConversationActivityTooltipPayloadEntry[];
  renderValue: (
    bucket: ConversationActivityBucket,
  ) => Array<{ label: string; value: string; color: string }>;
}) {
  const bucket = payload?.find((entry) => entry.payload)?.payload;
  if (!active || !bucket) return null;

  const rows = renderValue(bucket);
  if (rows.length === 0) return null;
  const surfaceTheme = resolveDocumentFloatingSurfaceTheme();

  return (
    <div
      role="tooltip"
      data-theme={surfaceTheme}
      data-inline-chart-tooltip="true"
      className="min-w-[11rem] max-w-[14rem] rounded-xl border px-3 py-2 text-[11px] leading-tight text-base-content"
      style={{
        ...floatingSurfaceStyle("neutral", surfaceTheme),
      }}
    >
      <div className="text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
        {typeof label === "string" ? label : bucket.tooltipLabel}
      </div>
      <div className="mt-2 space-y-1.5">
        {rows.map((row) => (
          <div key={row.label} className="flex items-start gap-2">
            <span
              className="mt-[5px] h-1.5 w-1.5 shrink-0 rounded-full"
              style={{ backgroundColor: row.color }}
              aria-hidden="true"
            />
            <div className="min-w-0 flex-1">
              <div className="text-base-content/62">{row.label}</div>
              <div className="mt-0.5 font-mono text-[12px] font-semibold tracking-tight text-base-content">
                {row.value}
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function ConversationActivityChart({
  buckets,
  rangeStartMs,
  rangeEndMs,
  metric,
  loading,
  numberFormatter,
  currencyFormatter,
  t,
}: {
  buckets: ConversationActivityBucket[];
  rangeStartMs: number | null;
  rangeEndMs: number | null;
  metric: ConversationActivityMetric;
  loading: boolean;
  numberFormatter: Intl.NumberFormat;
  currencyFormatter: Intl.NumberFormat;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  const themeMode = resolveDocumentThemeMode();
  const [viewport, setViewport] = useState<ConversationActivityViewport>({
    startIndex: 0,
    endIndex: Math.max(0, buckets.length - 1),
  });
  const viewportRef = useRef<ConversationActivityViewport>(viewport);
  const viewportIdentity = `${buckets.length}:${buckets[0]?.tooltipLabel ?? "empty"}:${buckets.at(-1)?.tooltipLabel ?? "empty"}`;
  const viewportIdentityRef = useRef(viewportIdentity);
  const interactionRef = useRef<HTMLDivElement | null>(null);
  const wheelListenerElementRef = useRef<HTMLDivElement | null>(null);
  const dragPreviewLayerRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<{
    pointerId: number;
    startClientX: number;
    startClientY: number;
    currentClientX: number;
    currentClientY: number;
    axis: ConversationActivityDragAxis;
    viewport: ConversationActivityViewport;
  } | null>(null);
  const dragPreviewOffsetRef = useRef(0);
  const dragPreviewFrameRef = useRef<number | null>(null);
  const wheelPanDeltaRef = useRef(0);
  const wheelPanFrameRef = useRef<number | null>(null);
  const wheelZoomDeltaRef = useRef(0);
  const wheelZoomAnchorRatioRef = useRef(0.5);
  const wheelZoomFrameRef = useRef<number | null>(null);

  useEffect(() => {
    viewportRef.current = viewport;
  }, [viewport]);

  useEffect(() => {
    setViewport((current) => {
      if (viewportIdentityRef.current !== viewportIdentity) {
        viewportIdentityRef.current = viewportIdentity;
        return normalizeConversationActivityViewport(
          { startIndex: 0, endIndex: Math.max(0, buckets.length - 1) },
          buckets.length,
        );
      }
      return normalizeConversationActivityViewport(current, buckets.length);
    });
  }, [buckets.length, viewportIdentity]);

  const visibleWindow = normalizeConversationActivityViewport(viewport, buckets.length);
  const visibleBuckets = buckets.slice(visibleWindow.startIndex, visibleWindow.endIndex + 1);
  const visibleTotalCount = visibleBuckets.reduce((sum, bucket) => sum + bucket.totalCount, 0);
  const viewportSpan = visibleWindow.endIndex - visibleWindow.startIndex + 1;
  const isZoomed = buckets.length > 0 && viewportSpan < buckets.length;
  const xDomain: [number, number] = [visibleWindow.startIndex, visibleWindow.endIndex];
  const barSize = useMemo(() => {
    if (buckets.length <= 0) return 1;
    const zoomFactor = buckets.length / Math.max(1, viewportSpan);
    const minimumReadableBarSize = buckets.length <= 60 ? 5 : 1;
    return clampConversationActivityValue(
      Math.round(zoomFactor * 0.75),
      minimumReadableBarSize,
      10,
    );
  }, [buckets.length, viewportSpan]);

  const getAnchorRatio = useCallback((clientX: number) => {
    const rect = interactionRef.current?.getBoundingClientRect();
    if (!rect || rect.width <= 0) return 0.5;
    return clampConversationActivityValue((clientX - rect.left) / rect.width, 0, 1);
  }, []);

  const scheduleWheelPan = useCallback(
    (deltaIndexes: number) => {
      wheelPanDeltaRef.current += deltaIndexes;
      if (wheelPanFrameRef.current != null) return;

      wheelPanFrameRef.current = window.requestAnimationFrame(() => {
        wheelPanFrameRef.current = null;
        const pendingDelta = wheelPanDeltaRef.current;
        wheelPanDeltaRef.current = 0;
        if (pendingDelta === 0) return;

        const roundedDelta =
          Math.round(pendingDelta) ||
          Math.sign(pendingDelta) *
            Math.max(
              1,
              Math.round(
                CONVERSATION_ACTIVITY_WHEEL_PAN_INTENSITY *
                  CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS,
              ),
            );
        setViewport((current) => {
          const normalized = normalizeConversationActivityViewport(current, buckets.length);
          const next = shiftConversationActivityViewport(normalized, buckets.length, roundedDelta);
          return isSameConversationActivityViewport(normalized, next) ? current : next;
        });
      });
    },
    [buckets.length],
  );

  const scheduleWheelZoom = useCallback(
    (deltaY: number, anchorRatio: number) => {
      wheelZoomDeltaRef.current += deltaY;
      wheelZoomAnchorRatioRef.current = anchorRatio;
      if (wheelZoomFrameRef.current != null) return;

      wheelZoomFrameRef.current = window.requestAnimationFrame(() => {
        wheelZoomFrameRef.current = null;
        const pendingDelta = wheelZoomDeltaRef.current;
        const pendingAnchorRatio = wheelZoomAnchorRatioRef.current;
        wheelZoomDeltaRef.current = 0;
        if (pendingDelta === 0) return;

        setViewport((current) => {
          const normalized = normalizeConversationActivityViewport(current, buckets.length);
          const next = zoomConversationActivityViewport(
            normalized,
            buckets.length,
            pendingDelta * CONVERSATION_ACTIVITY_WHEEL_ZOOM_INTENSITY,
            pendingAnchorRatio,
          );
          return isSameConversationActivityViewport(normalized, next) ? current : next;
        });
      });
    },
    [buckets.length],
  );

  useEffect(
    () => () => {
      if (wheelPanFrameRef.current != null) {
        window.cancelAnimationFrame(wheelPanFrameRef.current);
      }
      if (wheelZoomFrameRef.current != null) {
        window.cancelAnimationFrame(wheelZoomFrameRef.current);
      }
    },
    [],
  );

  const handleWheel = useCallback(
    (event: WheelEvent) => {
      if (buckets.length <= CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS) return;

      const horizontalIntent =
        Math.abs(event.deltaX) >= CONVERSATION_ACTIVITY_WHEEL_THRESHOLD &&
        Math.abs(event.deltaX) >= Math.abs(event.deltaY) &&
        !event.ctrlKey;
      const hasZoomIntent = event.ctrlKey || event.metaKey || event.altKey;
      if (!horizontalIntent && !hasZoomIntent) return;

      event.preventDefault();
      if (horizontalIntent) {
        const normalized = normalizeConversationActivityViewport(
          viewportRef.current,
          buckets.length,
        );
        const width = interactionRef.current?.getBoundingClientRect().width ?? 1;
        const span = normalized.endIndex - normalized.startIndex + 1;
        scheduleWheelPan((event.deltaX / Math.max(1, width)) * span);
        return;
      }

      scheduleWheelZoom(event.deltaY, getAnchorRatio(event.clientX));
    },
    [buckets.length, getAnchorRatio, scheduleWheelPan, scheduleWheelZoom],
  );

  const setInteractionLayerRef = useCallback(
    (node: HTMLDivElement | null) => {
      if (wheelListenerElementRef.current) {
        wheelListenerElementRef.current.removeEventListener("wheel", handleWheel);
        wheelListenerElementRef.current = null;
      }

      interactionRef.current = node;
      if (!node) return;

      node.addEventListener("wheel", handleWheel, { passive: false });
      wheelListenerElementRef.current = node;
    },
    [handleWheel],
  );

  const handlePointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0 || buckets.length <= CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS) {
        return;
      }
      dragPreviewOffsetRef.current = 0;
      if (dragPreviewLayerRef.current) {
        dragPreviewLayerRef.current.style.transform = "";
      }
      const normalized = normalizeConversationActivityViewport(viewport, buckets.length);
      dragRef.current = {
        pointerId: event.pointerId,
        startClientX: event.clientX,
        startClientY: event.clientY,
        currentClientX: event.clientX,
        currentClientY: event.clientY,
        axis: "pending",
        viewport: normalized,
      };
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    [buckets.length, viewport],
  );

  const scheduleDragPreview = useCallback(() => {
    if (dragPreviewFrameRef.current != null) return;

    dragPreviewFrameRef.current = window.requestAnimationFrame(() => {
      dragPreviewFrameRef.current = null;
      const drag = dragRef.current;
      if (!drag) return;

      const previewOffsetPx = drag.currentClientX - drag.startClientX;
      if (previewOffsetPx === dragPreviewOffsetRef.current) return;
      dragPreviewOffsetRef.current = previewOffsetPx;
      if (dragPreviewLayerRef.current) {
        dragPreviewLayerRef.current.style.transform =
          previewOffsetPx === 0 ? "" : `translate3d(${previewOffsetPx}px, 0, 0)`;
      }
    });
  }, []);

  const handlePointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      drag.currentClientX = event.clientX;
      drag.currentClientY = event.clientY;

      if (drag.axis === "pending") {
        const deltaX = Math.abs(drag.currentClientX - drag.startClientX);
        const deltaY = Math.abs(drag.currentClientY - drag.startClientY);
        const distance = Math.hypot(deltaX, deltaY);

        if (distance < CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_THRESHOLD_PX) return;

        if (deltaX >= deltaY * CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_RATIO) {
          drag.axis = "horizontal";
        } else if (deltaY >= deltaX * CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_RATIO) {
          drag.axis = "vertical";
          dragRef.current = null;
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId);
          }
          return;
        } else if (
          Math.min(deltaX, deltaY) >=
          Math.max(deltaX, deltaY) * CONVERSATION_ACTIVITY_POINTER_FREE_DIAGONAL_RATIO
        ) {
          drag.axis = "free";
        } else {
          return;
        }
      }

      if (drag.axis === "vertical") return;
      scheduleDragPreview();
    },
    [scheduleDragPreview],
  );

  const handlePointerEnd = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;

      if (drag.axis === "horizontal" || drag.axis === "free") {
        const width = interactionRef.current?.getBoundingClientRect().width ?? 1;
        const span = drag.viewport.endIndex - drag.viewport.startIndex + 1;
        const deltaIndexes = Math.round(
          ((drag.startClientX - drag.currentClientX) / Math.max(1, width)) * span,
        );
        setViewport((current) => {
          const next = shiftConversationActivityViewport(
            drag.viewport,
            buckets.length,
            deltaIndexes,
          );
          return isSameConversationActivityViewport(
            normalizeConversationActivityViewport(current, buckets.length),
            next,
          )
            ? current
            : next;
        });
      }

      dragRef.current = null;
      dragPreviewOffsetRef.current = 0;
      if (dragPreviewLayerRef.current) {
        dragPreviewLayerRef.current.style.transform = "";
      }
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    },
    [buckets.length],
  );

  useEffect(
    () => () => {
      if (dragPreviewFrameRef.current != null) {
        window.cancelAnimationFrame(dragPreviewFrameRef.current);
      }
    },
    [],
  );
  const chartColors = useMemo(() => {
    const base = chartBaseTokens(themeMode);
    const status = chartStatusTokens(themeMode);
    return {
      ...base,
      success: status.success,
      failure: status.failure,
      inFlight: metricAccent("totalCount", themeMode),
      neutral: themeMode === "dark" ? "#94a3b8" : "#64748b",
      firstByte: themeMode === "dark" ? "#cbd5e1" : "#475569",
    };
  }, [themeMode]);
  const maxCount = Math.max(
    1,
    ...visibleBuckets.map((bucket) =>
      Math.max(bucket.success + bucket.inFlight + bucket.neutral, bucket.failure),
    ),
  );
  const formatMetricValue = (value: number) => {
    if (metric === "totalCost") return currencyFormatter.format(value);
    return numberFormatter.format(value);
  };
  const countUnit = t("unit.calls");
  const legendLabels = {
    success: t("live.conversations.activity.legendSuccess"),
    failure: t("live.conversations.activity.legendFailure"),
    inFlight: t("live.conversations.activity.legendInFlight"),
    neutral: t("live.conversations.activity.legendNeutral"),
    duration: t("table.details.totalLatency"),
  };
  const renderTooltip = (bucket: ConversationActivityBucket) => [
    {
      label: legendLabels.success,
      value:
        `${formatMetricValue(bucket.success)} ${metric === "totalCount" ? countUnit : ""}`.trim(),
      color: chartColors.success,
    },
    {
      label: legendLabels.failure,
      value:
        `${formatMetricValue(bucket.failure)} ${metric === "totalCount" ? countUnit : ""}`.trim(),
      color: chartColors.failure,
    },
    {
      label: legendLabels.inFlight,
      value:
        `${formatMetricValue(bucket.inFlight)} ${metric === "totalCount" ? countUnit : ""}`.trim(),
      color: chartColors.inFlight,
    },
    {
      label: legendLabels.neutral,
      value:
        `${formatMetricValue(bucket.neutral)} ${metric === "totalCount" ? countUnit : ""}`.trim(),
      color: chartColors.neutral,
    },
    {
      label: legendLabels.duration,
      value: bucket.avgTotalMs == null ? "-" : `${numberFormatter.format(bucket.avgTotalMs)} ms`,
      color: chartColors.firstByte,
    },
  ];

  if (loading && buckets.length === 0) {
    return (
      <div className="flex h-80 items-center justify-center gap-2 rounded-xl border border-base-300/75 bg-base-200/40 text-sm text-base-content/60">
        <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
        <span>{t("chart.loadingDetailed")}</span>
      </div>
    );
  }

  return (
    <div
      className="overscroll-x-contain rounded-xl border border-base-300/75 bg-base-200/40 p-4"
      data-testid="conversation-activity-chart"
      data-chart-kind="conversation-activity"
      data-chart-metric={metric}
      data-visible-start-index={visibleWindow.startIndex}
      data-visible-end-index={visibleWindow.endIndex}
      data-visible-span={viewportSpan}
      data-visible-total-count={visibleTotalCount}
      data-zoomed={isZoomed ? "true" : "false"}
      data-chart-range-start={
        typeof rangeStartMs === "number" && Number.isFinite(rangeStartMs)
          ? new Date(rangeStartMs).toISOString()
          : undefined
      }
      data-chart-range-end={
        typeof rangeEndMs === "number" && Number.isFinite(rangeEndMs)
          ? new Date(rangeEndMs).toISOString()
          : undefined
      }
    >
      <div
        ref={setInteractionLayerRef}
        className="h-80 w-full cursor-grab touch-pan-y overflow-hidden overscroll-x-contain select-none active:cursor-grabbing"
        role="img"
        aria-label={t("live.conversations.activity.chartAria")}
        data-testid="conversation-activity-chart-interaction-layer"
        data-chart-kind="conversation-activity"
        data-min-visible-buckets={CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerEnd}
        onPointerCancel={handlePointerEnd}
        onLostPointerCapture={handlePointerEnd}
      >
        <div
          ref={dragPreviewLayerRef}
          data-testid="conversation-activity-chart-drag-layer"
          className="h-full w-full will-change-transform"
          style={{ transform: undefined }}
        >
          <ResponsiveContainer width="100%" height={320}>
            <ComposedChart
              data={visibleBuckets}
              margin={{ top: 12, right: 24, left: 0, bottom: 8 }}
              barGap="-100%"
              stackOffset="sign"
            >
              <CartesianGrid stroke={chartColors.gridLine} strokeDasharray="3 3" />
              <XAxis
                dataKey="index"
                type="number"
                domain={xDomain}
                minTickGap={28}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
                tickFormatter={(value: number) => {
                  const bucket =
                    buckets[Math.max(0, Math.min(buckets.length - 1, Math.round(value)))];
                  return bucket?.label ?? String(value);
                }}
              />
              <YAxis
                yAxisId="count"
                domain={[-maxCount, maxCount]}
                allowDecimals={false}
                tickFormatter={(value) => numberFormatter.format(Math.abs(Number(value)))}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <YAxis
                yAxisId="latency"
                orientation="right"
                tickFormatter={(value) => `${numberFormatter.format(Number(value))}ms`}
                width={72}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <Tooltip
                labelFormatter={(value) => {
                  const bucket =
                    buckets[Math.max(0, Math.min(buckets.length - 1, Math.round(Number(value))))];
                  return bucket?.tooltipLabel ?? String(value);
                }}
                content={(props) => (
                  <ConversationActivityTooltipContent
                    active={props.active}
                    label={props.label}
                    payload={
                      props.payload as unknown as
                        | ConversationActivityTooltipPayloadEntry[]
                        | undefined
                    }
                    renderValue={renderTooltip}
                  />
                )}
              />
              <ReferenceLine yAxisId="count" y={0} stroke={chartColors.gridLine} />
              <Bar
                yAxisId="count"
                dataKey="failureNegative"
                name={legendLabels.failure}
                stackId="positive"
                fill={chartColors.failure}
                barSize={barSize}
                radius={[0, 0, 3, 3]}
                shape={(props: ConversationActivityBarShapeProps) =>
                  renderAlignedFailureBarShape({
                    ...props,
                    fill: chartColors.failure,
                  })
                }
                isAnimationActive={false}
              />
              <Bar
                yAxisId="count"
                dataKey="success"
                name={legendLabels.success}
                stackId="positive"
                fill={chartColors.success}
                barSize={barSize}
                radius={[0, 0, 0, 0]}
                isAnimationActive={false}
              />
              <Bar
                yAxisId="count"
                dataKey="inFlight"
                name={legendLabels.inFlight}
                stackId="positive"
                fill={chartColors.inFlight}
                barSize={barSize}
                radius={[0, 0, 0, 0]}
                isAnimationActive={false}
              />
              <Bar
                yAxisId="count"
                dataKey="neutral"
                name={legendLabels.neutral}
                stackId="positive"
                fill={chartColors.neutral}
                barSize={barSize}
                radius={[3, 3, 0, 0]}
                isAnimationActive={false}
              />
              <Line
                yAxisId="latency"
                type="monotone"
                dataKey="avgTotalMs"
                name={legendLabels.duration}
                stroke={chartColors.firstByte}
                strokeOpacity={0.72}
                strokeWidth={1.25}
                dot={{
                  r: 1.25,
                  strokeWidth: 0,
                  fill: chartColors.firstByte,
                  fillOpacity: 0.72,
                }}
                connectNulls={false}
                isAnimationActive={false}
              />
            </ComposedChart>
          </ResponsiveContainer>
        </div>
        <div className="flex flex-wrap items-center justify-center gap-x-4 gap-y-1 text-xs text-base-content/70">
          <span className="inline-flex items-center gap-1.5">
            <span className="h-2.5 w-2.5 rounded-sm bg-success" />
            {legendLabels.success}
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span className="h-2.5 w-2.5 rounded-sm bg-error" />
            {legendLabels.failure}
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span
              className="h-2.5 w-2.5 rounded-sm"
              style={{ backgroundColor: chartColors.inFlight }}
            />
            {legendLabels.inFlight}
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span
              className="h-2.5 w-2.5 rounded-sm"
              style={{ backgroundColor: chartColors.neutral }}
            />
            {legendLabels.neutral}
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span className="h-px w-5 bg-base-content/70" />
            {legendLabels.duration}
          </span>
        </div>
      </div>
    </div>
  );
}

function PromptCacheConversationActivityOverview({
  open,
  conversationKey,
  historyQueryForConversationKey,
  realtimePayload,
  isRealtimeLoading,
  allowHttpFallback,
  t,
}: {
  open: boolean;
  conversationKey: string | null;
  historyQueryForConversationKey?: ConversationHistoryQueryBuilder;
  realtimePayload: InvocationHistoryOverviewTopicPayload | null;
  isRealtimeLoading: boolean;
  allowHttpFallback: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  const { locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const activeRange: ConversationActivityRange = "history";
  const [activeMetric, setActiveMetric] = useState<ConversationActivityMetric>("totalCount");
  const [summary, setSummary] = useState<InvocationRecordsSummaryResponse | null>(null);
  const [records, setRecords] = useState<ApiInvocation[]>([]);
  const [chartRangeStartMs, setChartRangeStartMs] = useState<number | null>(null);
  const [chartRangeEndMs, setChartRangeEndMs] = useState<number | null>(null);
  const [chartTotal, setChartTotal] = useState(0);
  const [chartIsSampled, setChartIsSampled] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fallbackRequestSeqRef = useRef(0);

  const numberFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        maximumFractionDigits: 2,
        notation: "compact",
      }),
    [localeTag],
  );
  const fullNumberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 2 }),
    [localeTag],
  );
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: "currency",
        currency: "USD",
        minimumFractionDigits: 2,
        maximumFractionDigits: 4,
      }),
    [localeTag],
  );

  useEffect(() => {
    if (!open || !conversationKey) {
      setSummary(null);
      setRecords([]);
      setChartRangeStartMs(null);
      setChartRangeEndMs(null);
      setChartTotal(0);
      setChartIsSampled(false);
      setIsLoading(false);
      setError(null);
      return;
    }
    setSummary(null);
    setRecords([]);
    setChartRangeStartMs(null);
    setChartRangeEndMs(null);
    setChartTotal(0);
    setChartIsSampled(false);
    setError(null);
    setIsLoading(isRealtimeLoading);
  }, [conversationKey, isRealtimeLoading, open]);

  useEffect(() => {
    if (!realtimePayload || allowHttpFallback) return;
    fallbackRequestSeqRef.current += 1;
    setSummary(realtimePayload.summary);
    setRecords(realtimePayload.records);
    setChartRangeStartMs(
      realtimePayload.chartRangeStart ? Date.parse(realtimePayload.chartRangeStart) : null,
    );
    setChartRangeEndMs(
      realtimePayload.chartRangeEnd ? Date.parse(realtimePayload.chartRangeEnd) : null,
    );
    setChartTotal(realtimePayload.chartTotal);
    setChartIsSampled(realtimePayload.chartIsSampled);
    setIsLoading(false);
    setError(null);
  }, [allowHttpFallback, realtimePayload]);

  useEffect(() => {
    if (!open || !conversationKey || !allowHttpFallback) return;
    const requestSeq = fallbackRequestSeqRef.current + 1;
    fallbackRequestSeqRef.current = requestSeq;
    const controller = new AbortController();
    const baseQuery = historyQueryForConversationKey?.(conversationKey) ?? {
      promptCacheKey: conversationKey,
    };
    const { page, pageSize, snapshotId, sortBy, sortOrder, signal, ...filters } = baseQuery;
    void page;
    void pageSize;
    void snapshotId;
    void sortBy;
    void sortOrder;
    void signal;
    setIsLoading(true);
    setError(null);
    void (async () => {
      const latestFirstPage = await fetchInvocationRecords({
        ...filters,
        page: 1,
        pageSize: PROMPT_CACHE_ACTIVITY_MAX_CHART_RECORDS,
        sortBy: "occurredAt",
        sortOrder: "desc",
        signal: controller.signal,
      });
      const firstPage = await fetchInvocationRecords({
        ...filters,
        page: 1,
        pageSize: latestFirstPage.pageSize,
        snapshotId: latestFirstPage.snapshotId,
        sortBy: "occurredAt",
        sortOrder: "desc",
        signal: controller.signal,
      });
      const nextSummary = await fetchInvocationRecordsSummary({
        ...filters,
        snapshotId: firstPage.snapshotId,
        signal: controller.signal,
      });
      const records = firstPage.records.slice(0, PROMPT_CACHE_ACTIVITY_MAX_CHART_RECORDS);
      const pageSize = Math.max(1, firstPage.pageSize);
      const targetCount = Math.min(
        PROMPT_CACHE_ACTIVITY_MAX_CHART_RECORDS,
        Math.max(0, firstPage.total),
      );
      let page = 2;
      let previousPageCount = firstPage.records.length;
      while (records.length < targetCount && previousPageCount >= pageSize) {
        if (controller.signal.aborted) return;
        const nextPage = await fetchInvocationRecords({
          ...filters,
          page,
          pageSize,
          snapshotId: firstPage.snapshotId,
          sortBy: "occurredAt",
          sortOrder: "desc",
          signal: controller.signal,
        });
        previousPageCount = nextPage.records.length;
        records.push(
          ...nextPage.records.slice(0, PROMPT_CACHE_ACTIVITY_MAX_CHART_RECORDS - records.length),
        );
        page += 1;
      }
      const rangeRecords = [...records];
      if (firstPage.total > records.length) {
        const oldestPage = Math.max(1, Math.ceil(firstPage.total / pageSize));
        const oldestResponse = await fetchInvocationRecords({
          ...filters,
          page: oldestPage,
          pageSize,
          snapshotId: firstPage.snapshotId,
          sortBy: "occurredAt",
          sortOrder: "desc",
          signal: controller.signal,
        });
        rangeRecords.push(...oldestResponse.records);
      }
      const occurredAt = rangeRecords
        .map((record) => Date.parse(record.occurredAt))
        .filter((value) => Number.isFinite(value));
      return {
        nextSummary,
        records,
        chartRangeStartMs: occurredAt.length > 0 ? Math.min(...occurredAt) : null,
        chartRangeEndMs: occurredAt.length > 0 ? Math.max(...occurredAt) : null,
        chartTotal: firstPage.total,
      };
    })()
      .then((response) => {
        if (controller.signal.aborted || requestSeq !== fallbackRequestSeqRef.current) return;
        if (!response) return;
        setSummary(response.nextSummary);
        setRecords(response.records);
        setChartRangeStartMs(response.chartRangeStartMs);
        setChartRangeEndMs(response.chartRangeEndMs);
        setChartTotal(response.chartTotal);
        setChartIsSampled(response.records.length < response.chartTotal);
        setIsLoading(false);
      })
      .catch((err) => {
        if (controller.signal.aborted || requestSeq !== fallbackRequestSeqRef.current) return;
        if (
          (err instanceof DOMException && err.name === "AbortError") ||
          (err instanceof Error && err.name === "AbortError")
        ) {
          return;
        }
        setError(err instanceof Error ? err.message : String(err));
        setIsLoading(false);
      });
    return () => controller.abort();
  }, [allowHttpFallback, conversationKey, historyQueryForConversationKey, open]);

  const bucketSet = useMemo(
    () =>
      buildConversationActivityBuckets({
        records,
        range: activeRange,
        metric: activeMetric,
        localeTag,
        rangeStartMs: chartRangeStartMs,
        rangeEndMs: chartRangeEndMs,
      }),
    [activeMetric, chartRangeEndMs, chartRangeStartMs, localeTag, records],
  );

  const metrics = [
    {
      label: t("live.conversations.activity.metricRequests"),
      value: formatCompactNumber(summary?.totalCount, numberFormatter),
      toneClass: "text-primary",
    },
    {
      label: t("live.conversations.activity.metricSuccess"),
      value: formatCompactNumber(summary?.successCount, numberFormatter),
      toneClass: "text-success",
    },
    {
      label: t("live.conversations.activity.metricFailures"),
      value: formatCompactNumber(summary?.failureCount, numberFormatter),
      toneClass: "text-error",
    },
    {
      label: t("live.conversations.activity.metricAborts"),
      value: formatCompactNumber(summary?.exception.clientAbortCount, numberFormatter),
      toneClass: "text-warning",
    },
    {
      label: t("live.conversations.activity.metricTokens"),
      value: formatCompactNumber(summary?.token.totalTokens, numberFormatter),
      toneClass: "text-info",
    },
    {
      label: t("live.conversations.activity.metricCost"),
      value: summary == null ? FALLBACK_CELL : currencyFormatter.format(summary.token.totalCost),
      toneClass: "text-primary",
    },
    {
      label: t("live.conversations.activity.metricAvgDuration"),
      value: formatDurationMs(summary?.network.avgTotalMs, fullNumberFormatter),
      toneClass: "text-base-content",
    },
  ];

  return (
    <section className="space-y-3 rounded-xl border border-base-300/70 bg-base-100/55 p-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <h3 className="text-sm font-semibold">{t("live.conversations.activity.title")}</h3>
        <SegmentedControl size="compact" role="tablist" aria-label={t("heatmap.metricsToggleAria")}>
          {CONVERSATION_ACTIVITY_METRICS.map((metric) => (
            <SegmentedControlItem
              key={metric.key}
              active={activeMetric === metric.key}
              role="tab"
              aria-selected={activeMetric === metric.key}
              onClick={() => setActiveMetric(metric.key)}
            >
              {t(metric.labelKey)}
            </SegmentedControlItem>
          ))}
        </SegmentedControl>
      </div>
      {error ? (
        <Alert variant="error">
          <span>{t("records.summary.loadError", { error })}</span>
        </Alert>
      ) : null}
      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4 xl:grid-cols-7">
        {metrics.map((metric) => (
          <div
            key={metric.label}
            className="rounded-lg border border-base-300/60 bg-base-200/25 px-3 py-2"
          >
            <div className="text-[11px] font-semibold uppercase tracking-[0.12em] text-base-content/55">
              {metric.label}
            </div>
            <div className={`mt-1 text-lg font-semibold ${metric.toneClass}`}>
              {isLoading && summary == null ? "…" : metric.value}
            </div>
          </div>
        ))}
      </div>
      <ConversationActivityChart
        buckets={bucketSet.buckets}
        rangeStartMs={bucketSet.rangeStartMs}
        rangeEndMs={bucketSet.rangeEndMs}
        metric={activeMetric}
        loading={isLoading}
        numberFormatter={numberFormatter}
        currencyFormatter={currencyFormatter}
        t={t}
      />
      {chartIsSampled ? (
        <p className="text-xs text-base-content/60">
          {t("live.conversations.activity.sampledChart", {
            loaded: formatCompactNumber(records.length, fullNumberFormatter),
            total: formatCompactNumber(chartTotal, fullNumberFormatter),
          })}
        </p>
      ) : null}
    </section>
  );
}

export function PromptCacheConversationHistoryDrawer({
  open,
  conversationKey,
  conversationLabel,
  initialTab = "overview",
  presentation = "overlay",
  onClose,
  onTabChange,
  t,
  onOpenUpstreamAccount,
  historyQueryForConversationKey,
}: {
  open: boolean;
  conversationKey: string | null;
  conversationLabel?: string | null;
  initialTab?: PromptCacheConversationDrawerTab;
  presentation?: "overlay" | "page";
  onClose: () => void;
  onTabChange?: (tab: PromptCacheConversationDrawerTab) => void;
  t: (key: string, values?: Record<string, string | number>) => string;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  historyQueryForConversationKey?: ConversationHistoryQueryBuilder;
}) {
  const titleId = useId();
  const requestSeqRef = useRef(0);
  const hasHydratedRef = useRef(false);
  const inFlightRef = useRef(false);
  const pendingLoadRef = useRef<{ silent?: boolean; append?: boolean } | null>(null);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const activeLoadControllerRef = useRef<AbortController | null>(null);
  const operationsLoadControllerRef = useRef<AbortController | null>(null);
  const operationsRequestSeqRef = useRef(0);
  const bindingHydrationSeqRef = useRef(0);
  const historySnapshotIdRef = useRef<number | undefined>(undefined);
  const historyHttpSnapshotInitializedRef = useRef(false);
  const historyNextPageRef = useRef(1);
  const historyHasMoreRef = useRef(false);
  const liveHistoryTotalRef = useRef(0);
  const recordsRef = useRef<ApiInvocation[]>([]);
  const callTopicInitializedRef = useRef(false);
  const frozenHistoryStableKeysRef = useRef<Set<string>>(new Set());
  const pendingCallRecordsRef = useRef<ApiInvocation[]>([]);
  const bindingDraftDirtyRef = useRef(false);
  const bindingTopicLoadingRef = useRef(false);
  const bindingTopicSseUnavailableCapturedRef = useRef(false);
  const bindingTopicSseUnavailableCaptureKeyRef = useRef<string | null>(null);
  const staleBindingTopicPayloadRef = useRef<PromptCacheConversationBindingResponse | null>(null);
  const operationEventsRef = useRef<PromptCacheConversationOperationEvent[]>([]);
  const operationsTopicKeyRef = useRef<string | null>(null);
  const operationsPageRef = useRef(1);
  const operationsTotalRef = useRef(0);
  const [drawerBodyElement, setDrawerBodyElement] = useState<HTMLDivElement | null>(null);
  const [records, setRecords] = useState<ApiInvocation[]>([]);
  const [liveRecords, setLiveRecords] = useState<ApiInvocation[]>([]);
  const [total, setTotal] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [binding, setBinding] = useState<PromptCacheConversationBindingResponse | null>(null);
  const [bindingKind, setBindingKind] = useState<ConversationBindingDraftKind>("none");
  const [bindingGroupName, setBindingGroupName] = useState("");
  const [bindingAccountId, setBindingAccountId] = useState("");
  const [bindingAccounts, setBindingAccounts] = useState<UpstreamAccountSummary[]>([]);
  const [bindingGroups, setBindingGroups] = useState<string[]>([]);
  const [bindingProxyNodes, setBindingProxyNodes] = useState<ForwardProxyBindingNode[]>([]);
  const [bindingLoading, setBindingLoading] = useState(false);
  const [bindingSaving, setBindingSaving] = useState(false);
  const [bindingError, setBindingError] = useState<string | null>(null);
  const [allowSwitchUpstreamDraft, setAllowSwitchUpstreamDraft] =
    useState<OptionalBooleanDraft>("inherit");
  const [fastModeDraft, setFastModeDraft] = useState<RewriteModeDraft>("keep_original");
  const [imageToolDraft, setImageToolDraft] = useState<RewriteModeDraft>("keep_original");
  const [codexImagegenDraft, setCodexImagegenDraft] = useState<RewriteModeDraft>("keep_original");
  const [availableModelsMode, setAvailableModelsMode] = useState<"inherit" | "override">("inherit");
  const [availableModelsDraft, setAvailableModelsDraft] = useState("");
  const [forwardProxyKeysDraft, setForwardProxyKeysDraft] = useState<string[]>([]);
  const [inlinePolicyBusyField, setInlinePolicyBusyField] =
    useState<ConversationInlinePolicyField | null>(null);
  const [inlinePolicyErrors, setInlinePolicyErrors] = useState<
    Partial<Record<ConversationInlinePolicyField, string | null>>
  >({});
  const [bindingOwnerConfirmOpen, setBindingOwnerConfirmOpen] = useState(false);
  const [bindingRemoteConflict, setBindingRemoteConflict] =
    useState<PromptCacheConversationBindingResponse | null>(null);
  const [bindingOwnerConfirmAllowsRemoteOverwrite, setBindingOwnerConfirmAllowsRemoteOverwrite] =
    useState(false);
  const [activeTab, setActiveTab] = useState<PromptCacheConversationDrawerTab>(initialTab);
  const [operationEvents, setOperationEvents] = useState<PromptCacheConversationOperationEvent[]>(
    [],
  );
  const [operationsTotal, setOperationsTotal] = useState(0);
  const [operationsPage, setOperationsPage] = useState(1);
  const [operationsLoading, setOperationsLoading] = useState(false);
  const [operationsLoadingMore, setOperationsLoadingMore] = useState(false);
  const [operationsError, setOperationsError] = useState<string | null>(null);
  const [operationsFilter, setOperationsFilter] =
    useState<PromptCacheConversationOperationFilter>("all");
  const conversationDetailScope = useMemo(
    () =>
      resolveConversationDetailScope(
        conversationKey,
        conversationKey ? historyQueryForConversationKey?.(conversationKey) : undefined,
      ),
    [conversationKey, historyQueryForConversationKey],
  );
  const {
    calls: callsTopic,
    overview: overviewTopic,
    binding: bindingTopic,
    operations: operationsTopic,
    isSseUnavailable,
  } = useConversationDetailTopics({
    open,
    activeTab,
    scope: conversationDetailScope,
    operationsInfoType: operationsFilter === "all" ? undefined : operationsFilter,
  });
  bindingTopicLoadingRef.current = bindingTopic.isLoading;

  useEffect(() => {
    if (!open) {
      setBindingOwnerConfirmOpen(false);
    }
  }, [open]);

  useEffect(() => {
    setBindingOwnerConfirmOpen(false);
  }, []);

  useEffect(() => {
    if (!open) return;
    setActiveTab(initialTab);
  }, [initialTab, open]);

  const handleSelectTab = useCallback(
    (nextTab: PromptCacheConversationDrawerTab) => {
      setActiveTab(nextTab);
      onTabChange?.(nextTab);
    },
    [onTabChange],
  );

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return;
    clearTimeout(refreshTimerRef.current);
    refreshTimerRef.current = null;
  }, []);

  // Run before topic hydration so a cached SSE snapshot cannot be cleared by
  // the same open/scope transition that made it available.
  useEffect(() => {
    requestSeqRef.current += 1;
    hasHydratedRef.current = false;
    inFlightRef.current = false;
    pendingLoadRef.current = null;
    activeLoadControllerRef.current?.abort();
    activeLoadControllerRef.current = null;
    historySnapshotIdRef.current = undefined;
    historyHttpSnapshotInitializedRef.current = false;
    historyNextPageRef.current = 1;
    historyHasMoreRef.current = false;
    liveHistoryTotalRef.current = 0;
    recordsRef.current = [];
    callTopicInitializedRef.current = false;
    frozenHistoryStableKeysRef.current = new Set();
    pendingCallRecordsRef.current = [];
    clearPendingRefreshTimer();

    if (!open || !conversationKey) {
      setRecords([]);
      setLiveRecords([]);
      setTotal(0);
      setIsLoading(false);
      setIsLoadingMore(false);
      setError(null);
      return;
    }

    setRecords([]);
    setLiveRecords([]);
    setTotal(0);
    setIsLoading(false);
    setIsLoadingMore(false);
    setError(null);
  }, [clearPendingRefreshTimer, conversationKey, open]);

  useEffect(() => {
    recordsRef.current = records;
  }, [records]);

  useEffect(() => {
    operationEventsRef.current = operationEvents;
  }, [operationEvents]);

  useEffect(() => {
    operationsPageRef.current = operationsPage;
  }, [operationsPage]);

  useEffect(() => {
    operationsTotalRef.current = operationsTotal;
  }, [operationsTotal]);

  useEffect(() => {
    const response = callsTopic.data;
    if (!open || activeTab !== "calls") return;
    if (!response || isSseUnavailable) {
      setIsLoading(callsTopic.isLoading);
      return;
    }
    const current = recordsRef.current;
    const incomingByKey = new Map(
      response.records.map((record) => [invocationStableKey(record), record]),
    );
    const updatedCurrent = current.map(
      (record) => incomingByKey.get(invocationStableKey(record)) ?? record,
    );
    const pendingByKey = new Map(
      pendingCallRecordsRef.current.map((record) => [invocationStableKey(record), record]),
    );
    const updatedPending = pendingCallRecordsRef.current.map(
      (record) => incomingByKey.get(invocationStableKey(record)) ?? record,
    );
    const loadedKeys = new Set(updatedCurrent.map(invocationStableKey));
    const newlyVisible = response.records.filter(
      (record) =>
        !loadedKeys.has(invocationStableKey(record)) &&
        !pendingByKey.has(invocationStableKey(record)),
    );
    const isInitialTopicSnapshot = !callTopicInitializedRef.current;
    const isAuthoritativeSnapshot = callsTopic.lastKind === "snapshot";
    const canInsert =
      isInitialTopicSnapshot ||
      !drawerBodyElement ||
      drawerBodyElement.scrollTop <= PROMPT_CACHE_HISTORY_TOP_INSERT_THRESHOLD_PX;
    const { nextRecords, nextPending } = isAuthoritativeSnapshot
      ? (() => {
          const frozenHistoryRecords = current.filter((record) =>
            frozenHistoryStableKeysRef.current.has(invocationStableKey(record)),
          );
          if (canInsert) {
            return {
              nextRecords: mergeInvocationRecordCollections(response.records, frozenHistoryRecords),
              nextPending: [],
            };
          }
          return {
            nextRecords: mergeInvocationRecordCollections(updatedCurrent, frozenHistoryRecords),
            nextPending: mergeInvocationRecordCollections(newlyVisible, updatedPending),
          };
        })()
      : {
          nextRecords: canInsert
            ? mergeInvocationRecordCollections(response.records, updatedCurrent)
            : updatedCurrent,
          nextPending: canInsert
            ? []
            : mergeInvocationRecordCollections(newlyVisible, updatedPending),
        };

    callTopicInitializedRef.current = true;
    pendingCallRecordsRef.current = nextPending;
    recordsRef.current = nextRecords;
    // The topic owns only the current live head. Deep pagination is pinned to
    // a separately captured HTTP page so runtime-only rows cannot skew offsets.
    if (!historyHttpSnapshotInitializedRef.current) {
      historyNextPageRef.current = 1;
    }
    historyHasMoreRef.current = nextRecords.length < response.total;
    liveHistoryTotalRef.current = response.total;
    setRecords(nextRecords);
    setLiveRecords(nextPending);
    setTotal(response.total);
    hasHydratedRef.current = true;
    setIsLoading(false);
    setError(null);
  }, [
    activeTab,
    callsTopic.data,
    callsTopic.isLoading,
    callsTopic.lastKind,
    drawerBodyElement,
    isSseUnavailable,
    open,
  ]);

  useEffect(() => {
    const response = operationsTopic.data;
    if (!response || isSseUnavailable || !open || activeTab !== "operations") return;
    operationsRequestSeqRef.current += 1;
    operationsLoadControllerRef.current?.abort();
    const topicKey = operationsTopic.descriptorKey ?? operationsFilter;
    const previousItems =
      operationsTopicKeyRef.current === topicKey ? operationEventsRef.current : [];
    const nextItems = Array.from(
      new Map([...response.items, ...previousItems].map((item) => [item.id, item])).values(),
    );
    operationsTopicKeyRef.current = topicKey;
    operationEventsRef.current = nextItems;
    operationsPageRef.current = Math.max(operationsPageRef.current, response.page);
    operationsTotalRef.current = response.total;
    setOperationEvents(nextItems);
    setOperationsTotal(response.total);
    setOperationsPage((current) => Math.max(current, response.page));
    setOperationsLoading(false);
    setOperationsError(null);
  }, [
    activeTab,
    isSseUnavailable,
    open,
    operationsFilter,
    operationsTopic.data,
    operationsTopic.descriptorKey,
  ]);

  const runLoad = useCallback(
    async ({ silent = false, append = false }: { silent?: boolean; append?: boolean } = {}) => {
      if (!open || !conversationKey) return;
      if (append && !historyHasMoreRef.current) return;

      inFlightRef.current = true;
      const requestSeq = requestSeqRef.current + 1;
      requestSeqRef.current = requestSeq;
      activeLoadControllerRef.current?.abort();
      const controller = new AbortController();
      activeLoadControllerRef.current = controller;
      const shouldShowLoading = !append && !(silent && hasHydratedRef.current);
      if (shouldShowLoading) setIsLoading(true);
      if (append) setIsLoadingMore(true);
      try {
        const historyFilters = historyQueryForConversationKey?.(conversationKey) ?? {
          promptCacheKey: conversationKey,
        };
        const fetchHistoryPage = (page: number, snapshotId?: number) =>
          fetchInvocationRecords({
            ...historyFilters,
            page,
            pageSize: PROMPT_CACHE_HISTORY_PAGE_SIZE,
            sortBy: "occurredAt",
            sortOrder: "desc",
            ...(snapshotId != null ? { snapshotId } : {}),
            signal: controller.signal,
          });
        let page = append ? historyNextPageRef.current : 1;
        let response: InvocationRecordsResponse;
        let capturedHttpHead: InvocationRecordsResponse | null = null;

        if (append && !historyHttpSnapshotInitializedRef.current) {
          const latestHttpHead = await fetchHistoryPage(1);
          if (requestSeq !== requestSeqRef.current) return;

          capturedHttpHead = await fetchHistoryPage(1, latestHttpHead.snapshotId);
          if (requestSeq !== requestSeqRef.current) return;

          historySnapshotIdRef.current = capturedHttpHead.snapshotId;
          historyHttpSnapshotInitializedRef.current = true;
          page = capturedHttpHead.page;
          response = capturedHttpHead;
        } else {
          response = await fetchHistoryPage(
            page,
            append ? historySnapshotIdRef.current : undefined,
          );
        }
        if (requestSeq !== requestSeqRef.current) return;

        const previousSnapshotId = historySnapshotIdRef.current;
        const previousNextPage = historyNextPageRef.current;
        const snapshotChanged =
          silent &&
          hasHydratedRef.current &&
          previousSnapshotId != null &&
          response.snapshotId !== previousSnapshotId;
        if (historyHttpSnapshotInitializedRef.current) {
          historySnapshotIdRef.current = response.snapshotId;
        }
        const effectiveResponseTotal = Math.max(response.total, liveHistoryTotalRef.current);
        const responseRecords = capturedHttpHead
          ? mergeInvocationRecordCollections(capturedHttpHead.records, response.records)
          : response.records;
        if (historyHttpSnapshotInitializedRef.current) {
          for (const record of responseRecords) {
            frozenHistoryStableKeysRef.current.add(invocationStableKey(record));
          }
        }
        const loaded = snapshotChanged
          ? mergeInvocationRecordCollections(responseRecords, recordsRef.current).slice(
              0,
              recordsRef.current.length + PROMPT_CACHE_HISTORY_PAGE_SIZE,
            )
          : append
            ? mergeInvocationRecordCollections(recordsRef.current, responseRecords)
            : silent && hasHydratedRef.current
              ? mergeInvocationRecordCollections(responseRecords, recordsRef.current).slice(
                  0,
                  Math.max(recordsRef.current.length, responseRecords.length),
                )
              : mergeInvocationRecordCollections(responseRecords, recordsRef.current);
        recordsRef.current = loaded;
        historyNextPageRef.current = snapshotChanged
          ? 2
          : append || !silent || !hasHydratedRef.current
            ? page + 1
            : Math.max(
                previousNextPage,
                Math.floor(loaded.length / PROMPT_CACHE_HISTORY_PAGE_SIZE) + 1,
              );
        historyHasMoreRef.current =
          loaded.length < effectiveResponseTotal &&
          (append ? response.records.length > 0 : loaded.length > 0);
        setRecords(loaded);
        setTotal(effectiveResponseTotal);

        if (requestSeq !== requestSeqRef.current) return;
        hasHydratedRef.current = true;
        const loadedStableKeys = new Set(loaded.map(invocationStableKey));
        setLiveRecords((current) =>
          current.filter((record) => !loadedStableKeys.has(invocationStableKey(record))),
        );
        if (capturedHttpHead && historyHasMoreRef.current) {
          pendingLoadRef.current = { append: true, silent: true };
        }
        setError(null);
      } catch (err) {
        if (requestSeq !== requestSeqRef.current) return;
        if (
          (err instanceof DOMException && err.name === "AbortError") ||
          (err instanceof Error && err.name === "AbortError")
        ) {
          return;
        }
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (requestSeq === requestSeqRef.current && shouldShowLoading) {
          setIsLoading(false);
        }
        if (requestSeq === requestSeqRef.current && append) {
          setIsLoadingMore(false);
        }
        if (requestSeq === requestSeqRef.current) {
          inFlightRef.current = false;
        }
        const pendingLoad = pendingLoadRef.current;
        if (requestSeq === requestSeqRef.current && pendingLoad) {
          pendingLoadRef.current = null;
          void runLoad(pendingLoad);
        }
      }
    },
    [conversationKey, historyQueryForConversationKey, open],
  );

  const load = useCallback(
    async (options: { silent?: boolean; append?: boolean } = {}) => {
      const silent = options.silent ?? false;
      const append = options.append ?? false;
      if (inFlightRef.current) {
        const pendingSilent = pendingLoadRef.current?.silent ?? true;
        pendingLoadRef.current = {
          silent: pendingSilent && silent,
          append: pendingLoadRef.current?.append || append,
        };
        return;
      }
      await runLoad({ silent, append });
    },
    [runLoad],
  );

  useEffect(() => {
    if (
      !open ||
      !conversationKey ||
      activeTab !== "calls" ||
      (callsTopic.data && !isSseUnavailable) ||
      !isSseUnavailable
    ) {
      return;
    }
    void load();
  }, [activeTab, callsTopic.data, conversationKey, isSseUnavailable, load, open]);

  const loadOperationEvents = useCallback(
    async ({ append = false }: { append?: boolean } = {}) => {
      if (!open || !conversationKey || activeTab !== "operations") return;
      if (append && operationEventsRef.current.length >= operationsTotalRef.current) return;

      operationsRequestSeqRef.current += 1;
      const requestSeq = operationsRequestSeqRef.current;
      operationsLoadControllerRef.current?.abort();
      const controller = new AbortController();
      operationsLoadControllerRef.current = controller;
      if (append) {
        setOperationsLoadingMore(true);
      } else {
        setOperationsLoading(true);
      }
      setOperationsError(null);
      try {
        const response = await fetchPromptCacheConversationOperationEvents(conversationKey, {
          page: append ? operationsPageRef.current + 1 : 1,
          pageSize: PROMPT_CACHE_OPERATION_EVENT_PAGE_SIZE,
          infoType: operationsFilter === "all" ? undefined : operationsFilter,
          signal: controller.signal,
        });
        if (controller.signal.aborted || requestSeq !== operationsRequestSeqRef.current) return;
        const nextItems = append
          ? Array.from(
              new Map(
                [...operationEventsRef.current, ...response.items].map((item) => [item.id, item]),
              ).values(),
            )
          : response.items;
        operationEventsRef.current = nextItems;
        setOperationEvents(nextItems);
        setOperationsTotal(response.total);
        setOperationsPage(response.page);
      } catch (err) {
        if (
          controller.signal.aborted ||
          (err instanceof DOMException && err.name === "AbortError") ||
          (err instanceof Error && err.name === "AbortError")
        ) {
          return;
        }
        setOperationsError(err instanceof Error ? err.message : String(err));
      } finally {
        if (requestSeq === operationsRequestSeqRef.current) {
          setOperationsLoading(false);
          setOperationsLoadingMore(false);
        }
      }
    },
    [activeTab, conversationKey, open, operationsFilter],
  );

  useEffect(() => {
    if (!open || !conversationKey) {
      bindingDraftDirtyRef.current = false;
      setBindingRemoteConflict(null);
      setBindingOwnerConfirmAllowsRemoteOverwrite(false);
      setInlinePolicyBusyField(null);
      setInlinePolicyErrors({});
      return;
    }
    bindingDraftDirtyRef.current = false;
    setBindingRemoteConflict(null);
    setBindingOwnerConfirmAllowsRemoteOverwrite(false);
    setInlinePolicyBusyField(null);
    setInlinePolicyErrors({});
  }, [conversationKey, open]);

  useEffect(() => {
    if (!open || !conversationKey) {
      bindingDraftDirtyRef.current = false;
      setActiveTab("overview");
      setBinding(null);
      setBindingKind("none");
      setBindingGroupName("");
      setBindingAccountId("");
      setBindingAccounts([]);
      setBindingGroups([]);
      setBindingProxyNodes([]);
      setBindingLoading(false);
      setBindingSaving(false);
      setBindingOwnerConfirmAllowsRemoteOverwrite(false);
      setBindingError(null);
      setAllowSwitchUpstreamDraft("inherit");
      setFastModeDraft("keep_original");
      setImageToolDraft("keep_original");
      setCodexImagegenDraft("keep_original");
      setAvailableModelsMode("inherit");
      setAvailableModelsDraft("");
      setForwardProxyKeysDraft([]);
      setInlinePolicyBusyField(null);
      setInlinePolicyErrors({});
      operationsLoadControllerRef.current?.abort();
      operationEventsRef.current = [];
      operationsTopicKeyRef.current = null;
      operationsPageRef.current = 1;
      operationsTotalRef.current = 0;
      setOperationEvents([]);
      setOperationsTotal(0);
      setOperationsPage(1);
      setOperationsLoading(false);
      setOperationsLoadingMore(false);
      setOperationsError(null);
      setOperationsFilter("all");
      return;
    }

    if (activeTab !== "settings") {
      return;
    }

    const controller = new AbortController();
    const hydrationSeq = bindingHydrationSeqRef.current + 1;
    bindingHydrationSeqRef.current = hydrationSeq;
    setBindingLoading(isSseUnavailable || bindingTopicLoadingRef.current);
    setBindingError(null);
    void Promise.all([
      isSseUnavailable
        ? fetchPromptCacheConversationBinding(conversationKey, controller.signal)
        : Promise.resolve(null),
      fetchUpstreamAccounts({ includeAll: true, pageSize: 500 }),
    ])
      .then(([nextBinding, accountList]) => {
        if (controller.signal.aborted) return;
        const accounts = accountList.items.filter(accountCanBePromptCacheBindingTarget);
        const groups = Array.from(
          new Set(
            accounts
              .map((account) => account.groupName ?? "")
              .map((groupName) => groupName.trim())
              .filter((groupName) => groupName.length > 0),
          ),
        ).sort((left, right) => left.localeCompare(right));
        setBindingAccounts(accounts);
        setBindingGroups(groups);
        setBindingProxyNodes(
          (accountList.forwardProxyNodes ?? []).filter((node) => node.selectable),
        );
        if (!nextBinding || hydrationSeq !== bindingHydrationSeqRef.current) return;
        setBinding(nextBinding);
        setBindingKind(nextBinding.bindingKind);
        applyBindingPolicyDraft(nextBinding, {
          setAllowSwitchUpstreamDraft,
          setFastModeDraft,
          setImageToolDraft,
          setCodexImagegenDraft,
          setAvailableModelsMode,
          setAvailableModelsDraft,
          setForwardProxyKeysDraft,
        });
        setBindingGroupName(nextBinding.groupName ?? groups[0] ?? "");
        setBindingAccountId(
          nextBinding.upstreamAccountId != null
            ? String(nextBinding.upstreamAccountId)
            : accounts[0]
              ? String(accounts[0].id)
              : "",
        );
        setInlinePolicyErrors({});
      })
      .catch((err) => {
        if (controller.signal.aborted || hydrationSeq !== bindingHydrationSeqRef.current) return;
        setBindingError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (
          isSseUnavailable &&
          !controller.signal.aborted &&
          hydrationSeq === bindingHydrationSeqRef.current
        ) {
          setBindingLoading(false);
        }
      });

    return () => controller.abort();
  }, [activeTab, conversationKey, isSseUnavailable, open]);

  useEffect(() => {
    if (!isSseUnavailable) {
      bindingTopicSseUnavailableCapturedRef.current = false;
      bindingTopicSseUnavailableCaptureKeyRef.current = null;
      staleBindingTopicPayloadRef.current = null;
      return;
    }
    if (bindingTopicSseUnavailableCaptureKeyRef.current !== conversationKey) {
      // Keep the payload that was already cached when transport failed. A later
      // in-flight topic update is newer than this fallback baseline.
      staleBindingTopicPayloadRef.current = bindingTopic.data;
      bindingTopicSseUnavailableCapturedRef.current = true;
      bindingTopicSseUnavailableCaptureKeyRef.current = conversationKey;
    }
  }, [bindingTopic.data, conversationKey, isSseUnavailable]);

  useEffect(() => {
    const nextBinding = bindingTopic.data;
    const isCachedFallbackPayload =
      isSseUnavailable &&
      bindingTopicSseUnavailableCapturedRef.current &&
      nextBinding === staleBindingTopicPayloadRef.current;
    if (!nextBinding || isCachedFallbackPayload || !open || activeTab !== "settings") return;
    bindingHydrationSeqRef.current += 1;
    if (bindingDraftDirtyRef.current) {
      setBindingRemoteConflict(nextBinding);
      return;
    }
    setBinding(nextBinding);
    setBindingKind(nextBinding.bindingKind);
    applyBindingPolicyDraft(nextBinding, {
      setAllowSwitchUpstreamDraft,
      setFastModeDraft,
      setImageToolDraft,
      setCodexImagegenDraft,
      setAvailableModelsMode,
      setAvailableModelsDraft,
      setForwardProxyKeysDraft,
    });
    setBindingGroupName(nextBinding.groupName ?? bindingGroups[0] ?? "");
    setBindingAccountId(
      nextBinding.upstreamAccountId != null
        ? String(nextBinding.upstreamAccountId)
        : bindingAccounts[0]
          ? String(bindingAccounts[0].id)
          : "",
    );
    setBindingLoading(false);
    setBindingError(null);
  }, [activeTab, bindingAccounts, bindingGroups, bindingTopic.data, isSseUnavailable, open]);

  useEffect(
    () => () => {
      activeLoadControllerRef.current?.abort();
      operationsLoadControllerRef.current?.abort();
      clearPendingRefreshTimer();
      pendingLoadRef.current = null;
    },
    [clearPendingRefreshTimer],
  );

  useEffect(() => {
    if (!open || !conversationKey || activeTab !== "operations") {
      return;
    }
    if (operationsTopic.data && !isSseUnavailable) {
      return;
    }
    operationsRequestSeqRef.current += 1;
    operationsLoadControllerRef.current?.abort();
    operationsTopicKeyRef.current = operationsTopic.descriptorKey ?? operationsFilter;
    operationEventsRef.current = [];
    operationsPageRef.current = 1;
    operationsTotalRef.current = 0;
    setOperationEvents([]);
    setOperationsTotal(0);
    setOperationsPage(1);
    setOperationsError(null);
    if (isSseUnavailable) {
      void loadOperationEvents();
    }
  }, [
    activeTab,
    conversationKey,
    isSseUnavailable,
    loadOperationEvents,
    open,
    operationsFilter,
    operationsTopic.data,
    operationsTopic.descriptorKey,
  ]);

  useEffect(() => {
    if (!open || activeTab !== "calls" || !drawerBodyElement) return;
    const maybeLoadMore = () => {
      if (isLoading || isLoadingMore || inFlightRef.current || !historyHasMoreRef.current) {
        return;
      }
      const remaining =
        drawerBodyElement.scrollHeight -
        drawerBodyElement.scrollTop -
        drawerBodyElement.clientHeight;
      if (remaining <= 420) {
        void load({ append: true, silent: true });
      }
    };
    drawerBodyElement.addEventListener("scroll", maybeLoadMore, {
      passive: true,
    });
    return () => {
      drawerBodyElement.removeEventListener("scroll", maybeLoadMore);
    };
  }, [activeTab, drawerBodyElement, isLoading, isLoadingMore, load, open]);

  const visibleRecords = records;
  const displayTitle = conversationLabel?.trim() || conversationKey || FALLBACK_CELL;
  const shouldShowConversationKey =
    Boolean(conversationLabel?.trim()) &&
    Boolean(conversationKey?.trim()) &&
    conversationLabel?.trim() !== conversationKey?.trim();
  const effectiveTotal = total;
  const loadedCount = visibleRecords.length;
  const availableModelsOverrideList = useMemo(
    () => splitConversationModelsDraft(availableModelsDraft),
    [availableModelsDraft],
  );
  const availableModelsOverrideEmpty =
    availableModelsMode === "override" && availableModelsOverrideList.length === 0;
  const bindingSubmitDisabled =
    !conversationKey ||
    !binding ||
    bindingLoading ||
    bindingSaving ||
    inlinePolicyBusyField != null ||
    (bindingKind === "group" && !bindingGroupName) ||
    (bindingKind === "upstreamAccount" && !bindingAccountId);
  const timeoutFieldLabels = useMemo(
    () =>
      ({
        responsesFirstByteTimeoutSecs: t(
          "accountPool.upstreamAccounts.routing.timeout.responsesFirstByte",
        ),
        compactFirstByteTimeoutSecs: t(
          "accountPool.upstreamAccounts.routing.timeout.compactFirstByte",
        ),
        imageFirstByteTimeoutSecs: t("accountPool.upstreamAccounts.routing.timeout.imageFirstByte"),
        responsesStreamTimeoutSecs: t(
          "accountPool.upstreamAccounts.routing.timeout.responsesStream",
        ),
        compactStreamTimeoutSecs: t("accountPool.upstreamAccounts.routing.timeout.compactStream"),
      }) as const,
    [t],
  );
  const bindingStatusLabel = currentBindingLabel(binding, t);
  const encryptedOwnerStatusLabel = encryptedOwnerLabel(binding);
  const bindingOwnerConfirmLabel =
    encryptedOwnerStatusLabel ?? t("live.conversations.drawer.binding.ownerConfirm.unknownOwner");
  const rewriteModeOptions = useMemo(
    () => [
      {
        value: "force_remove",
        label: t("live.conversations.drawer.policy.rewrite.forceRemove"),
      },
      {
        value: "keep_original",
        label: t("live.conversations.drawer.policy.rewrite.keepOriginal"),
      },
      {
        value: "fill_missing",
        label: t("live.conversations.drawer.policy.rewrite.fillMissing"),
      },
      {
        value: "force_add",
        label: t("live.conversations.drawer.policy.rewrite.forceAdd"),
      },
    ],
    [t],
  );
  const buildCurrentBindingPayloadBase = useCallback(() => {
    if (binding?.bindingKind === "group" && binding.groupName) {
      return {
        bindingKind: "group" as const,
        groupName: binding.groupName,
      };
    }
    if (binding?.bindingKind === "upstreamAccount" && binding.upstreamAccountId != null) {
      return {
        bindingKind: "upstreamAccount" as const,
        upstreamAccountId: binding.upstreamAccountId,
      };
    }
    return {
      bindingKind: "none" as const,
    };
  }, [binding]);
  const saveConversationInlinePolicy = useCallback(
    async (
      field: ConversationInlinePolicyField,
      patch:
        | { allowSwitchUpstream: boolean | null }
        | { fastModeRewriteMode: PromptCacheConversationRewriteMode | null }
        | { imageToolRewriteMode: PromptCacheConversationRewriteMode | null }
        | { codexImagegenRewriteMode: PromptCacheConversationRewriteMode | null }
        | { availableModels: string[] | null }
        | { forwardProxyKeys: string[] | null }
        | { timeouts: NonNullable<UpdateGroupAccountRoutingRulePayload["timeouts"]> },
    ) => {
      if (!conversationKey || !binding || bindingSaving || inlinePolicyBusyField != null) return;
      bindingDraftDirtyRef.current = true;
      setInlinePolicyErrors((current) => ({ ...current, [field]: null }));
      setBindingError(null);
      setInlinePolicyBusyField(field);
      try {
        const nextBinding = await updatePromptCacheConversationBinding(conversationKey, {
          ...buildCurrentBindingPayloadBase(),
          ...patch,
        });
        setBinding(nextBinding);
        applyBindingPolicyDraft(nextBinding, {
          setAllowSwitchUpstreamDraft,
          setFastModeDraft,
          setImageToolDraft,
          setCodexImagegenDraft,
          setAvailableModelsMode,
          setAvailableModelsDraft,
          setForwardProxyKeysDraft,
        });
        setInlinePolicyErrors((current) => ({ ...current, [field]: null }));
        const hasUnsavedManualBindingDraft =
          nextBinding.bindingKind !== bindingKind ||
          (bindingKind === "group" && nextBinding.groupName !== bindingGroupName) ||
          (bindingKind === "upstreamAccount" &&
            String(nextBinding.upstreamAccountId ?? "") !== bindingAccountId);
        bindingDraftDirtyRef.current = hasUnsavedManualBindingDraft;
        if (!hasUnsavedManualBindingDraft) {
          setBindingRemoteConflict(null);
        }
      } catch (err) {
        bindingDraftDirtyRef.current = true;
        setInlinePolicyErrors((current) => ({
          ...current,
          [field]: err instanceof Error ? err.message : String(err),
        }));
      } finally {
        setInlinePolicyBusyField((current) => (current === field ? null : current));
      }
    },
    [
      binding,
      bindingAccountId,
      bindingGroupName,
      bindingKind,
      bindingSaving,
      buildCurrentBindingPayloadBase,
      conversationKey,
      inlinePolicyBusyField,
    ],
  );
  const conversationEffectiveRoutingRule = useMemo(
    () => buildConversationEffectiveRoutingRule(binding),
    [binding],
  );
  const conversationRowValueOverrides = useMemo(() => {
    const rowOverrides = buildConversationRowValueOverrides(binding, t);
    rowOverrides.allowCutOut = {
      ...(rowOverrides.allowCutOut ?? {}),
      editor: (
        <SelectField
          value={allowSwitchUpstreamDraft}
          disabled={inlinePolicyBusyField != null}
          aria-label={t("live.conversations.drawer.policy.cutOut")}
          size="sm"
          options={[
            {
              value: "true",
              label: t("live.conversations.drawer.policy.cutOutAllow"),
            },
            {
              value: "false",
              label: t("live.conversations.drawer.policy.cutOutDeny"),
            },
          ]}
          onValueChange={(value) => {
            setAllowSwitchUpstreamDraft(value as OptionalBooleanDraft);
            void saveConversationInlinePolicy("allowCutOut", {
              allowSwitchUpstream: value === "true",
            });
          }}
        />
      ),
    };
    rowOverrides.fastModeRewriteMode = {
      ...(rowOverrides.fastModeRewriteMode ?? {}),
      editor: (
        <SelectField
          value={fastModeDraft}
          disabled={inlinePolicyBusyField != null}
          aria-label={t("live.conversations.drawer.policy.fastMode")}
          size="sm"
          options={rewriteModeOptions}
          onValueChange={(value) => {
            setFastModeDraft(value as RewriteModeDraft);
            void saveConversationInlinePolicy("fastModeRewriteMode", {
              fastModeRewriteMode: value as PromptCacheConversationRewriteMode,
            });
          }}
        />
      ),
    };
    rowOverrides.imageToolRewriteMode = {
      ...(rowOverrides.imageToolRewriteMode ?? {}),
      editor: (
        <SelectField
          value={imageToolDraft}
          disabled={inlinePolicyBusyField != null}
          aria-label={t("live.conversations.drawer.policy.imageTool")}
          size="sm"
          options={rewriteModeOptions}
          onValueChange={(value) => {
            setImageToolDraft(value as RewriteModeDraft);
            void saveConversationInlinePolicy("imageToolRewriteMode", {
              imageToolRewriteMode: value as PromptCacheConversationRewriteMode,
            });
          }}
        />
      ),
    };
    rowOverrides.codexImagegenRewriteMode = {
      ...(rowOverrides.codexImagegenRewriteMode ?? {}),
      editor: (
        <SelectField
          value={codexImagegenDraft}
          disabled={inlinePolicyBusyField != null}
          aria-label="Codex imagegen"
          size="sm"
          options={rewriteModeOptions}
          onValueChange={(value) => {
            setCodexImagegenDraft(value as RewriteModeDraft);
            void saveConversationInlinePolicy("codexImagegenRewriteMode", {
              codexImagegenRewriteMode: value as PromptCacheConversationRewriteMode,
            });
          }}
        />
      ),
    };
    rowOverrides.availableModels = {
      ...(rowOverrides.availableModels ?? {}),
      editor: (
        <div className="space-y-2">
          <Input
            value={availableModelsDraft}
            disabled={inlinePolicyBusyField != null}
            aria-label={t("live.conversations.drawer.policy.availableModels")}
            placeholder={t("live.conversations.drawer.policy.availableModelsPlaceholder")}
            className="h-9"
            onChange={(event) => {
              bindingDraftDirtyRef.current = true;
              setAvailableModelsMode("override");
              setAvailableModelsDraft(event.target.value);
            }}
          />
          {availableModelsOverrideEmpty ? (
            <p className="text-xs text-error">
              {t("live.conversations.drawer.policy.availableModelsRequired")}
            </p>
          ) : null}
          <Button
            type="button"
            size="sm"
            disabled={inlinePolicyBusyField != null || availableModelsOverrideList.length === 0}
            onClick={() =>
              void saveConversationInlinePolicy("availableModels", {
                availableModels: availableModelsOverrideList,
              })
            }
          >
            {t("live.conversations.drawer.policy.applyField")}
          </Button>
        </div>
      ),
    };
    return rowOverrides;
  }, [
    allowSwitchUpstreamDraft,
    availableModelsDraft,
    availableModelsOverrideEmpty,
    availableModelsOverrideList,
    binding,
    fastModeDraft,
    imageToolDraft,
    codexImagegenDraft,
    inlinePolicyBusyField,
    rewriteModeOptions,
    saveConversationInlinePolicy,
    t,
  ]);
  const bindingKindOptions = [
    {
      value: "none",
      label: t("live.conversations.drawer.binding.kindNone"),
    },
    {
      value: "group",
      label: t("live.conversations.drawer.binding.kindGroup"),
      disabled: bindingGroups.length === 0,
    },
    {
      value: "upstreamAccount",
      label: t("live.conversations.drawer.binding.kindAccount"),
      disabled: bindingAccounts.length === 0,
    },
  ];
  const tabListLabel = t("live.conversations.drawer.tabs.label");
  const bindingPanel = (
    <div className="space-y-4 text-sm">
      <section className="rounded-xl border border-base-content/10 bg-base-200/50 p-4">
        <div className="flex items-start justify-between gap-3">
          <div>
            <p className="font-semibold text-base-content">
              {t("live.conversations.drawer.binding.title")}
            </p>
          </div>
          {bindingLoading ? (
            <Spinner size="sm" aria-label={t("live.conversations.drawer.binding.loading")} />
          ) : null}
        </div>
        <p className="mt-2 text-xs text-base-content/70">{bindingStatusLabel}</p>
        {encryptedOwnerStatusLabel ? (
          <p className="mt-1 text-xs text-warning">
            {t("live.conversations.drawer.binding.encryptedOwner", {
              owner: encryptedOwnerStatusLabel,
            })}
          </p>
        ) : null}
        {binding?.hasEncryptedSessionOwner && binding.bindingKind === "none" ? (
          <p className="mt-1 text-xs text-base-content/60">
            {t("live.conversations.drawer.binding.encryptedOwnerHint")}
          </p>
        ) : null}
        <div className="mt-3 grid gap-2 sm:grid-cols-[8.5rem_minmax(0,1fr)_auto]">
          <SelectField
            value={bindingKind}
            disabled={bindingLoading || bindingSaving || inlinePolicyBusyField != null}
            aria-label={t("live.conversations.drawer.binding.kind")}
            size="sm"
            options={bindingKindOptions}
            onValueChange={(value) => {
              bindingDraftDirtyRef.current = true;
              setBindingKind(value as ConversationBindingDraftKind);
            }}
          />
          {bindingKind === "group" ? (
            <SelectField
              value={bindingGroupName}
              disabled={bindingLoading || bindingSaving || inlinePolicyBusyField != null}
              aria-label={t("live.conversations.drawer.binding.group")}
              size="sm"
              options={bindingGroups.map((groupName) => ({
                value: groupName,
                label: groupName,
              }))}
              onValueChange={(value) => {
                bindingDraftDirtyRef.current = true;
                setBindingGroupName(value);
              }}
            />
          ) : bindingKind === "upstreamAccount" ? (
            <SelectField
              value={bindingAccountId}
              disabled={bindingLoading || bindingSaving || inlinePolicyBusyField != null}
              aria-label={t("live.conversations.drawer.binding.account")}
              size="sm"
              options={bindingAccounts.map((account) => ({
                value: String(account.id),
                label: conversationBindingAccountLabel(account),
              }))}
              onValueChange={(value) => {
                bindingDraftDirtyRef.current = true;
                setBindingAccountId(value);
              }}
            />
          ) : (
            <div className="hidden sm:block" aria-hidden="true" />
          )}
          <Button
            type="button"
            size="sm"
            disabled={bindingSubmitDisabled}
            onClick={() => void saveBinding()}
          >
            {bindingSaving
              ? t("live.conversations.drawer.binding.saving")
              : t("live.conversations.drawer.binding.save")}
          </Button>
        </div>
        {bindingError ? <p className="mt-2 text-xs text-error">{bindingError}</p> : null}
        {bindingRemoteConflict ? (
          <Alert className="mt-3" variant="warning">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span>{t("live.conversations.drawer.binding.remoteConflict")}</span>
              <div className="flex gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  onClick={() => {
                    const latest = bindingRemoteConflict;
                    bindingDraftDirtyRef.current = false;
                    setBinding(latest);
                    setBindingKind(latest.bindingKind);
                    applyBindingPolicyDraft(latest, {
                      setAllowSwitchUpstreamDraft,
                      setFastModeDraft,
                      setImageToolDraft,
                      setCodexImagegenDraft,
                      setAvailableModelsMode,
                      setAvailableModelsDraft,
                      setForwardProxyKeysDraft,
                    });
                    setBindingGroupName(latest.groupName ?? bindingGroups[0] ?? "");
                    setBindingAccountId(
                      latest.upstreamAccountId != null
                        ? String(latest.upstreamAccountId)
                        : bindingAccounts[0]
                          ? String(bindingAccounts[0].id)
                          : "",
                    );
                    setBindingRemoteConflict(null);
                  }}
                >
                  {t("live.conversations.drawer.binding.adoptLatest")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  onClick={() => void saveBinding({ allowRemoteOverwrite: true })}
                >
                  {t("live.conversations.drawer.binding.keepSaving")}
                </Button>
              </div>
            </div>
          </Alert>
        ) : null}
      </section>
      {conversationEffectiveRoutingRule ? (
        <EffectiveRoutingRuleCard
          rule={conversationEffectiveRoutingRule}
          identityKey={conversationKey}
          localOverrideSource="conversation"
          visibleRows={[
            "allowCutOut",
            "fastModeRewriteMode",
            "imageToolRewriteMode",
            "codexImagegenRewriteMode",
            "availableModels",
            "proxyBindings",
          ]}
          visibleSections={{
            statusChangeReasons: false,
            sourceTags: false,
          }}
          rowValueOverrides={conversationRowValueOverrides}
          editablePolicy={{
            busyField: inlinePolicyBusyField,
            errorByField: inlinePolicyErrors,
            onChange: (field, payload) => {
              if (field === "allowCutOut") {
                void saveConversationInlinePolicy("allowCutOut", {
                  allowSwitchUpstream: payload.allowCutOut ?? null,
                });
                return;
              }
              if (field === "fastModeRewriteMode") {
                void saveConversationInlinePolicy("fastModeRewriteMode", {
                  fastModeRewriteMode: payload.fastModeRewriteMode ?? null,
                });
                return;
              }
              if (field === "imageToolRewriteMode") {
                void saveConversationInlinePolicy("imageToolRewriteMode", {
                  imageToolRewriteMode: payload.imageToolRewriteMode ?? null,
                });
                return;
              }
              if (field === "codexImagegenRewriteMode") {
                void saveConversationInlinePolicy("codexImagegenRewriteMode", {
                  codexImagegenRewriteMode: payload.codexImagegenRewriteMode ?? null,
                });
                return;
              }
              if (field === "availableModels") {
                void saveConversationInlinePolicy("availableModels", {
                  availableModels: payload.availableModels ?? null,
                });
                return;
              }
              const timeoutPatch = payload.timeouts;
              if (!timeoutPatch) return;
              const [timeoutKey] = Object.keys(timeoutPatch) as Array<keyof typeof timeoutPatch>;
              if (!timeoutKey) return;
              void saveConversationInlinePolicy(mapTimeoutFieldToInlineField(timeoutKey), {
                timeouts: timeoutPatch,
              });
            },
          }}
          proxyBindings={{
            source: conversationProxySource(binding),
            items: forwardProxyKeysDraft.map((key) => {
              const node = bindingProxyNodes.find((candidate) => candidate.key === key);
              return {
                key,
                label: node ? conversationForwardProxyLabel(node) : key,
              };
            }),
            busy: inlinePolicyBusyField === "proxyBindings",
            disabled:
              bindingLoading ||
              bindingSaving ||
              (inlinePolicyBusyField != null && inlinePolicyBusyField !== "proxyBindings"),
            onClear: () =>
              void saveConversationInlinePolicy("proxyBindings", { forwardProxyKeys: null }),
            onRemove: (key) => {
              const nextKeys = toggleConversationProxyKey(forwardProxyKeysDraft, key);
              setForwardProxyKeysDraft(nextKeys);
              void saveConversationInlinePolicy("proxyBindings", {
                forwardProxyKeys: nextKeys.length > 0 ? nextKeys : null,
              });
            },
            editor: (
              <div className="space-y-2">
                <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto]">
                  <SelectField
                    value=""
                    disabled={bindingLoading || bindingSaving || inlinePolicyBusyField != null}
                    aria-label={t("live.conversations.drawer.policy.proxy")}
                    size="sm"
                    options={[
                      {
                        value: "",
                        label: t("live.conversations.drawer.policy.proxyAddPlaceholder"),
                        disabled: true,
                      },
                      ...bindingProxyNodes.map((node) => ({
                        value: node.key,
                        label: conversationForwardProxyLabel(node),
                        disabled: forwardProxyKeysDraft.includes(node.key),
                      })),
                    ]}
                    onValueChange={(value) => {
                      if (!value) return;
                      const nextKeys = toggleConversationProxyKey(forwardProxyKeysDraft, value);
                      setForwardProxyKeysDraft(nextKeys);
                      void saveConversationInlinePolicy("proxyBindings", {
                        forwardProxyKeys: nextKeys,
                      });
                    }}
                  />
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    disabled={inlinePolicyBusyField != null || forwardProxyKeysDraft.length === 0}
                    onClick={() => {
                      setForwardProxyKeysDraft([]);
                      void saveConversationInlinePolicy("proxyBindings", {
                        forwardProxyKeys: null,
                      });
                    }}
                  >
                    {t("live.conversations.drawer.policy.inherit")}
                  </Button>
                </div>
                <div className="flex flex-wrap gap-1.5">
                  {forwardProxyKeysDraft.length === 0 ? (
                    <span className="text-xs text-base-content/60">
                      {t("live.conversations.drawer.policy.proxyInherited")}
                    </span>
                  ) : (
                    forwardProxyKeysDraft.map((key) => {
                      const node = bindingProxyNodes.find((candidate) => candidate.key === key);
                      return (
                        <span
                          key={key}
                          className="inline-flex min-w-0 items-center gap-1.5 rounded-full border border-base-300 bg-base-100 px-2 py-1 text-xs"
                        >
                          <span className="max-w-48 truncate">
                            {node ? conversationForwardProxyLabel(node) : key}
                          </span>
                          <button
                            type="button"
                            className="rounded-full px-1 text-base-content/55 hover:bg-base-200 hover:text-base-content"
                            disabled={inlinePolicyBusyField != null}
                            aria-label={t("live.conversations.drawer.policy.proxyRemove")}
                            onClick={() => {
                              const nextKeys = toggleConversationProxyKey(
                                forwardProxyKeysDraft,
                                key,
                              );
                              setForwardProxyKeysDraft(nextKeys);
                              void saveConversationInlinePolicy("proxyBindings", {
                                forwardProxyKeys: nextKeys.length > 0 ? nextKeys : null,
                              });
                            }}
                          >
                            x
                          </button>
                        </span>
                      );
                    })
                  )}
                </div>
              </div>
            ),
            labels: {
              field: t("live.conversations.drawer.policy.proxy"),
              add: t("live.conversations.drawer.policy.proxyAddPlaceholder"),
              clear: t("live.conversations.drawer.policy.inherit"),
              empty: t("live.conversations.drawer.policy.proxyInherited"),
              hint: t("accountPool.upstreamAccounts.proxyBindings.failoverHint"),
              remove: t("live.conversations.drawer.policy.proxyRemove"),
            },
          }}
          labels={{
            title: t("live.conversations.drawer.policy.title"),
            description: t("live.conversations.drawer.policy.description"),
            noTags: t("accountPool.upstreamAccounts.effectiveRule.noTags"),
            allowCutOut: t("live.conversations.drawer.policy.cutOutAllow"),
            denyCutOut: t("live.conversations.drawer.policy.cutOutDeny"),
            allowCutIn: t("accountPool.upstreamAccounts.effectiveRule.allowCutIn"),
            denyCutIn: t("accountPool.upstreamAccounts.effectiveRule.denyCutIn"),
            sourceTags: t("accountPool.upstreamAccounts.effectiveRule.sourceTags"),
            priorityPrimary: t("accountPool.upstreamAccounts.effectiveRule.priorityPrimary"),
            priorityNormal: t("accountPool.upstreamAccounts.effectiveRule.priorityNormal"),
            priorityFallback: t("accountPool.upstreamAccounts.effectiveRule.priorityFallback"),
            priorityNoNew: t("accountPool.tags.dialog.priorityNoNew"),
            fastModeKeepOriginal: t("live.conversations.drawer.policy.rewrite.keepOriginal"),
            fastModeFillMissing: t("live.conversations.drawer.policy.rewrite.fillMissing"),
            fastModeForceAdd: t("live.conversations.drawer.policy.rewrite.forceAdd"),
            fastModeForceRemove: t("live.conversations.drawer.policy.rewrite.forceRemove"),
            imageToolKeepOriginal: t("live.conversations.drawer.policy.rewrite.keepOriginal"),
            imageToolFillMissing: t("live.conversations.drawer.policy.rewrite.fillMissing"),
            imageToolForceAdd: t("live.conversations.drawer.policy.rewrite.forceAdd"),
            imageToolForceRemove: t("live.conversations.drawer.policy.rewrite.forceRemove"),
            availableModelsInherited: t(
              "accountPool.upstreamAccounts.effectiveRule.availableModelsInherited",
            ),
            availableModelsNoneAllowed: t(
              "accountPool.upstreamAccounts.effectiveRule.availableModelsNoneAllowed",
            ),
            availableModelsEmpty: t("accountPool.tags.dialog.availableModelsEmpty"),
            availableModelsField: t("live.conversations.drawer.policy.availableModels"),
            systemDeniedModelsField: t(
              "accountPool.upstreamAccounts.effectiveRule.fieldSystemDeniedModels",
            ),
            systemDeniedModelsEmpty: t(
              "accountPool.upstreamAccounts.effectiveRule.systemDeniedModelsEmpty",
            ),
            sourceBreakdownTitle: t(
              "accountPool.upstreamAccounts.effectiveRule.sourceBreakdownTitle",
            ),
            fieldAllowCutOut: t("live.conversations.drawer.policy.cutOut"),
            fieldAllowCutIn: t("accountPool.upstreamAccounts.effectiveRule.fieldAllowCutIn"),
            fieldPriority: t("accountPool.upstreamAccounts.effectiveRule.fieldPriority"),
            fieldFastMode: t("live.conversations.drawer.policy.fastMode"),
            fieldImageToolRewriteMode: t("live.conversations.drawer.policy.imageTool"),
            imageToolRewriteHint: t(
              "accountPool.upstreamAccounts.groupNotes.routingPolicy.imageToolRewriteHint",
            ),
            fieldCodexImagegenRewriteMode: t("live.conversations.drawer.policy.codexImagegen"),
            codexImagegenRewriteHint: t(
              "accountPool.upstreamAccounts.groupNotes.routingPolicy.codexImagegenRewriteHint",
            ),
            fieldConcurrency: t("accountPool.upstreamAccounts.effectiveRule.fieldConcurrency"),
            fieldUpstream429: t("accountPool.upstreamAccounts.effectiveRule.fieldUpstream429"),
            fieldAvailableModels: t("live.conversations.drawer.policy.availableModels"),
            fieldSystemDeniedModels: t(
              "accountPool.upstreamAccounts.effectiveRule.fieldSystemDeniedModels",
            ),
            fieldProxyBindings: t("live.conversations.drawer.policy.proxy"),
            fieldRequestCompression: t(
              "accountPool.upstreamAccounts.effectiveRule.fieldRequestCompression",
            ),
            timeoutSectionTitle: t("accountPool.upstreamAccounts.routing.timeout.sectionTitle"),
            timeoutInheritedValue: t("accountPool.upstreamAccounts.timeoutEditor.inherited"),
            timeoutOverrideValue: t(
              "accountPool.upstreamAccounts.timeoutEditor.conversationOverride",
            ),
            timeoutResponsesFirstByte: timeoutFieldLabels.responsesFirstByteTimeoutSecs,
            timeoutCompactFirstByte: timeoutFieldLabels.compactFirstByteTimeoutSecs,
            timeoutImageFirstByte: timeoutFieldLabels.imageFirstByteTimeoutSecs,
            timeoutResponsesStream: timeoutFieldLabels.responsesStreamTimeoutSecs,
            timeoutCompactStream: timeoutFieldLabels.compactStreamTimeoutSecs,
            sourceRoot: t("accountPool.upstreamAccounts.effectiveRule.sourceRoot"),
            sourceGroup: t("accountPool.upstreamAccounts.effectiveRule.sourceGroup"),
            sourceTag: t("accountPool.upstreamAccounts.effectiveRule.sourceTag"),
            sourceAccount: t("accountPool.upstreamAccounts.effectiveRule.sourceAccount"),
            sourceConversation: t("accountPool.upstreamAccounts.effectiveRule.sourceConversation"),
            sourceSystem: t("accountPool.upstreamAccounts.effectiveRule.sourceSystem"),
            overrideEdit: t("live.conversations.drawer.policy.editField"),
            overrideClear: t("live.conversations.drawer.policy.clearField"),
            overrideSaving: t("live.conversations.drawer.binding.saving"),
            inheritValue: t("live.conversations.drawer.policy.inherit"),
            cutOutLabel: t("live.conversations.drawer.policy.cutOut"),
            cutInLabel: t("accountPool.upstreamAccounts.effectiveRule.fieldCutIn"),
            requestCompressionFollow: t("accountPool.requestCompression.follow"),
            requestCompressionIdentity: t("accountPool.requestCompression.identity"),
            requestCompressionGzip: t("accountPool.requestCompression.gzip"),
            requestCompressionDeflate: t("accountPool.requestCompression.deflate"),
            requestCompressionZstd: t("accountPool.requestCompression.zstd"),
            upstream429RetryCountValue: (count) => String(count),
            availableModelsAddCustom: t("accountPool.tags.dialog.availableModelsAddCustom"),
            availableModelsCustomLabel: (value) =>
              t("accountPool.tags.dialog.availableModelsCustomLabel", { value }),
            availableModelsRemove: t("accountPool.tags.dialog.availableModelsRemove"),
            availableModelsPlaceholder: t(
              "accountPool.tags.dialog.availableModelsSearchPlaceholder",
            ),
            currentValue: t("accountPool.tags.dialog.currentValue"),
          }}
        />
      ) : null}
    </div>
  );
  const hasMoreOperationEvents = operationEvents.length < operationsTotal;
  const operationsPanel = (
    <div className="space-y-4 text-sm">
      <section className="rounded-xl border border-base-content/10 bg-base-200/50 p-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="space-y-1">
            <p className="font-semibold text-base-content">
              {t("live.conversations.drawer.operations.title")}
            </p>
            <p className="text-xs text-base-content/68">
              {t("live.conversations.drawer.operations.description")}
            </p>
          </div>
          {operationsLoading && operationEvents.length === 0 ? (
            <Spinner size="sm" aria-label={t("live.conversations.drawer.operations.loading")} />
          ) : null}
        </div>
        <div className="mt-3 flex flex-wrap gap-2">
          {PROMPT_CACHE_OPERATION_FILTER_OPTIONS.map((option) => (
            <Button
              key={option.value}
              type="button"
              size="sm"
              variant={operationsFilter === option.value ? "default" : "secondary"}
              onClick={() => setOperationsFilter(option.value)}
            >
              {t(option.labelKey)}
            </Button>
          ))}
        </div>
      </section>
      {operationsError ? (
        <Alert variant="error">
          <span>{operationsError}</span>
        </Alert>
      ) : null}
      {operationEvents.length === 0 && !operationsLoading ? (
        <div className="rounded-xl border border-dashed border-base-300/75 bg-base-100/55 px-4 py-5 text-sm text-base-content/65">
          {t("live.conversations.drawer.operations.empty")}
        </div>
      ) : null}
      {operationEvents.map((event) => (
        <article
          key={event.id}
          className="space-y-3 rounded-xl border border-base-content/10 bg-base-100/80 p-4"
        >
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0 flex-1 space-y-2">
              <div className="flex flex-wrap gap-2">
                {event.infoTypes.map((infoType) => (
                  <Badge
                    key={`${event.id}-${infoType}`}
                    variant={conversationOperationInfoTypeBadgeVariant(infoType)}
                  >
                    {conversationOperationInfoTypeLabel(infoType, t)}
                  </Badge>
                ))}
                <Badge variant={conversationOperationOriginBadgeVariant(event.origin)}>
                  {conversationOperationOriginLabel(event.origin, t)}
                </Badge>
              </div>
              <p className="break-words text-sm font-semibold text-base-content">
                {conversationOperationActionLabel(event.action, event.headline, t)}
              </p>
            </div>
            <span className="text-xs text-base-content/58">
              {formatConversationOperationOccurredAt(event.occurredAt)}
            </span>
          </div>
          {event.changedFields.length > 0 ? (
            <p className="text-xs text-base-content/70">
              {t("live.conversations.drawer.operations.changedFields", {
                fields: event.changedFields
                  .map((field) => conversationOperationChangedFieldLabel(field, t))
                  .join(" / "),
              })}
            </p>
          ) : null}
          {event.bindingBefore || event.bindingAfter ? (
            <p className="text-xs text-base-content/72">
              {t("live.conversations.drawer.operations.bindingTransition", {
                from: conversationOperationBindingSnapshotLabel(event.bindingBefore, t),
                to: conversationOperationBindingSnapshotLabel(event.bindingAfter, t),
              })}
            </p>
          ) : null}
          {event.stickyBefore || event.stickyAfter ? (
            <p className="text-xs text-base-content/72">
              {t("live.conversations.drawer.operations.stickyTransition", {
                from: conversationOperationStickySnapshotLabel(event.stickyBefore, t),
                to: conversationOperationStickySnapshotLabel(event.stickyAfter, t),
              })}
            </p>
          ) : null}
          {event.infoTypes.includes("routing") && conversationOperationShowsRoutingReason(event) ? (
            <div className="space-y-1 text-xs text-base-content/70">
              <p>{conversationOperationRoutingReasonLabel(event, t)}</p>
              {event.routingContext?.routingSource ? (
                <p>
                  {t("live.conversations.drawer.operations.routingContext.source", {
                    source: t(
                      `live.conversations.drawer.operations.routingContext.sources.${event.routingContext.routingSource}`,
                    ),
                  })}
                </p>
              ) : null}
              {event.routingContext?.routingSelectionAudit ? (
                <div className="space-y-1 rounded border border-info/25 bg-info/5 p-2">
                  <p className="font-medium text-base-content">
                    {t("table.poolAttempts.routingDecision.summary", {
                      account: event.routingContext.routingSelectionAudit.selectedAccountName,
                      count: event.routingContext.routingSelectionAudit.eligibleCandidateCount,
                    })}
                  </p>
                  <p>
                    {routingSelectionWinnerLabel(event.routingContext.routingSelectionAudit, t)}
                  </p>
                  {event.routingContext.routingSelectionAudit.selectedScore ? (
                    <p data-testid="conversation-routing-selection-score">
                      {routingSelectionScoreLabel(
                        event.routingContext.routingSelectionAudit.selectedAccountName,
                        event.routingContext.routingSelectionAudit.selectedScore,
                        t,
                      )}
                    </p>
                  ) : null}
                  {event.routingContext.routingSelectionAudit.comparedScore &&
                  event.routingContext.routingSelectionAudit.comparedAccountName ? (
                    <p>
                      {routingSelectionScoreLabel(
                        event.routingContext.routingSelectionAudit.comparedAccountName,
                        event.routingContext.routingSelectionAudit.comparedScore,
                        t,
                      )}
                    </p>
                  ) : null}
                  {event.routingContext.routingSelectionAudit.excludedCandidates.map(
                    (candidate) => (
                      <p key={`${candidate.accountId}-${candidate.reasonCode}`}>
                        {routingSelectionExclusionLabel(
                          candidate.accountName,
                          candidate.reasonCode,
                          t,
                        )}
                      </p>
                    ),
                  )}
                </div>
              ) : null}
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                {event.routingContext?.causingAttemptId ? (
                  <Link
                    className="inline-flex font-mono text-[11px] text-primary underline underline-offset-2"
                    to={routingAttemptHref(event, event.routingContext.causingAttemptId)}
                  >
                    {t("live.conversations.drawer.operations.routingContext.causeAttempt", {
                      attemptId: event.routingContext.causingAttemptId,
                    })}
                  </Link>
                ) : null}
                {event.routingContext?.triggerAttemptId ? (
                  <Link
                    className="inline-flex font-mono text-[11px] text-primary underline underline-offset-2"
                    to={routingAttemptHref(event, event.routingContext.triggerAttemptId)}
                  >
                    {t(
                      event.routingContext.routingSelectionAudit
                        ? "live.conversations.drawer.operations.routingContext.routingDecisionAttempt"
                        : "live.conversations.drawer.operations.routingContext.triggerAttempt",
                      {
                        attemptId: event.routingContext.triggerAttemptId,
                      },
                    )}
                  </Link>
                ) : null}
                {routingInvocationRecordHref(event) ? (
                  <Link
                    className="inline-flex font-mono text-[11px] text-primary underline underline-offset-2"
                    to={routingInvocationRecordHref(event) ?? "#"}
                    aria-label={t(
                      "live.conversations.drawer.operations.routingContext.invocationRecordLabel",
                      {
                        id: event.invokeId ?? event.routingContext?.triggerAttemptId ?? "",
                      },
                    )}
                    title={t(
                      "live.conversations.drawer.operations.routingContext.invocationRecordLabel",
                      {
                        id: event.invokeId ?? event.routingContext?.triggerAttemptId ?? "",
                      },
                    )}
                  >
                    {t("live.conversations.drawer.operations.routingContext.invocationRecord", {
                      id: event.invokeId ?? event.routingContext?.triggerAttemptId ?? "",
                    })}
                  </Link>
                ) : null}
              </div>
            </div>
          ) : null}
          {event.invokeId ? (
            <p className="break-all font-mono text-[11px] text-base-content/58">
              {t("live.conversations.drawer.operations.invokeId", {
                invokeId: event.invokeId,
              })}
            </p>
          ) : null}
        </article>
      ))}
      {hasMoreOperationEvents ? (
        <div className="flex items-center justify-center">
          <Button
            type="button"
            size="sm"
            variant="secondary"
            disabled={operationsLoadingMore}
            onClick={() => void loadOperationEvents({ append: true })}
          >
            {operationsLoadingMore
              ? t("live.conversations.drawer.operations.loadingMore")
              : t("live.conversations.drawer.operations.loadMore")}
          </Button>
        </div>
      ) : null}
    </div>
  );
  const saveBinding = useCallback(
    async (options?: { skipOwnerWarning?: boolean; allowRemoteOverwrite?: boolean }) => {
      if (!conversationKey || bindingSubmitDisabled) return;
      if (bindingRemoteConflict && !options?.allowRemoteOverwrite) return;
      if (
        !options?.skipOwnerWarning &&
        nextBindingWouldOverrideEncryptedOwner(
          binding,
          bindingKind,
          bindingGroupName,
          bindingAccountId,
        )
      ) {
        setBindingOwnerConfirmAllowsRemoteOverwrite(Boolean(options?.allowRemoteOverwrite));
        setBindingOwnerConfirmOpen(true);
        return;
      }
      setBindingSaving(true);
      setBindingError(null);
      setBindingRemoteConflict(null);
      try {
        const nextBinding = await updatePromptCacheConversationBinding(
          conversationKey,
          bindingKind === "group"
            ? {
                bindingKind: "group",
                groupName: bindingGroupName,
              }
            : bindingKind === "upstreamAccount"
              ? {
                  bindingKind: "upstreamAccount",
                  upstreamAccountId: Number(bindingAccountId),
                }
              : { bindingKind: "none" },
        );
        setBinding(nextBinding);
        setBindingKind(nextBinding.bindingKind);
        applyBindingPolicyDraft(nextBinding, {
          setAllowSwitchUpstreamDraft,
          setFastModeDraft,
          setImageToolDraft,
          setCodexImagegenDraft,
          setAvailableModelsMode,
          setAvailableModelsDraft,
          setForwardProxyKeysDraft,
        });
        setBindingGroupName(nextBinding.groupName ?? bindingGroups[0] ?? "");
        setBindingAccountId(
          nextBinding.upstreamAccountId != null
            ? String(nextBinding.upstreamAccountId)
            : bindingAccounts[0]
              ? String(bindingAccounts[0].id)
              : "",
        );
        bindingDraftDirtyRef.current = false;
        setBindingRemoteConflict(null);
      } catch (err) {
        bindingDraftDirtyRef.current = true;
        setBindingError(err instanceof Error ? err.message : String(err));
      } finally {
        setBindingSaving(false);
      }
    },
    [
      binding,
      bindingAccountId,
      bindingAccounts,
      bindingGroupName,
      bindingGroups,
      bindingKind,
      bindingRemoteConflict,
      bindingSubmitDisabled,
      conversationKey,
    ],
  );
  const revealPendingCalls = useCallback(() => {
    if (liveRecords.length === 0) return;
    const nextRecords = mergeInvocationRecordCollections(liveRecords, recordsRef.current);
    pendingCallRecordsRef.current = [];
    recordsRef.current = nextRecords;
    setRecords(nextRecords);
    setLiveRecords([]);
    drawerBodyElement?.scrollTo({ top: 0, behavior: "smooth" });
  }, [drawerBodyElement, liveRecords]);

  return (
    <>
      <AccountDetailDrawerShell
        open={open}
        presentation={presentation}
        labelledBy={titleId}
        closeLabel={t("live.conversations.drawer.close")}
        onClose={onClose}
        closeDisabled={bindingOwnerConfirmOpen}
        onBodyElementChange={setDrawerBodyElement}
        shellClassName="drawer-shell--detail-wide"
        header={
          <div className="space-y-4">
            <div className="space-y-3">
              <div className="section-heading">
                <p className="text-xs font-semibold uppercase tracking-[0.2em] text-primary/75">
                  {t("live.conversations.drawer.eyebrow")}
                </p>
                <h2 id={titleId} className="section-title break-all">
                  {displayTitle}
                </h2>
                {shouldShowConversationKey ? (
                  <p className="break-all font-mono text-xs text-base-content/62">
                    {conversationKey}
                  </p>
                ) : null}
                <p className="section-description">{t("live.conversations.drawer.description")}</p>
              </div>
              <div className="text-sm text-base-content/70">
                {effectiveTotal > 0 && loadedCount >= effectiveTotal
                  ? t("live.conversations.drawer.progressComplete", {
                      count: effectiveTotal,
                    })
                  : t("live.conversations.drawer.progress", {
                      loaded: loadedCount,
                      total: effectiveTotal,
                    })}
              </div>
            </div>
            <SegmentedControl
              size="compact"
              role="tablist"
              aria-label={tabListLabel}
              className="w-fit max-w-full overflow-x-auto"
            >
              <SegmentedControlItem
                active={activeTab === "overview"}
                role="tab"
                aria-selected={activeTab === "overview"}
                aria-controls={`${titleId}-panel-overview`}
                id={`${titleId}-tab-overview`}
                onClick={() => handleSelectTab("overview")}
              >
                {t("live.conversations.drawer.tabs.overview")}
              </SegmentedControlItem>
              <SegmentedControlItem
                active={activeTab === "calls"}
                role="tab"
                aria-selected={activeTab === "calls"}
                aria-controls={`${titleId}-panel-calls`}
                id={`${titleId}-tab-calls`}
                onClick={() => handleSelectTab("calls")}
              >
                {t("live.conversations.drawer.tabs.calls")}
              </SegmentedControlItem>
              <SegmentedControlItem
                active={activeTab === "settings"}
                role="tab"
                aria-selected={activeTab === "settings"}
                aria-controls={`${titleId}-panel-settings`}
                id={`${titleId}-tab-settings`}
                onClick={() => handleSelectTab("settings")}
              >
                {t("live.conversations.drawer.tabs.settings")}
              </SegmentedControlItem>
              <SegmentedControlItem
                active={activeTab === "operations"}
                role="tab"
                aria-selected={activeTab === "operations"}
                aria-controls={`${titleId}-panel-operations`}
                id={`${titleId}-tab-operations`}
                onClick={() => handleSelectTab("operations")}
              >
                {t("live.conversations.drawer.tabs.operations")}
              </SegmentedControlItem>
            </SegmentedControl>
          </div>
        }
      >
        {activeTab === "overview" ? (
          <div
            id={`${titleId}-panel-overview`}
            role="tabpanel"
            aria-labelledby={`${titleId}-tab-overview`}
          >
            <PromptCacheConversationActivityOverview
              open={open}
              conversationKey={conversationKey}
              historyQueryForConversationKey={historyQueryForConversationKey}
              realtimePayload={overviewTopic.data}
              isRealtimeLoading={overviewTopic.isLoading}
              allowHttpFallback={isSseUnavailable}
              t={t}
            />
          </div>
        ) : null}
        {activeTab === "calls" ? (
          <div
            id={`${titleId}-panel-calls`}
            role="tabpanel"
            aria-labelledby={`${titleId}-tab-calls`}
            className="space-y-3"
          >
            {liveRecords.length > 0 ? (
              <div className="sticky top-2 z-10 flex justify-center">
                <Button type="button" size="sm" onClick={revealPendingCalls}>
                  {t("live.conversations.drawer.calls.newRecords", {
                    count: liveRecords.length,
                  })}
                </Button>
              </div>
            ) : null}
            <PromptCacheConversationInvocationTable
              records={visibleRecords}
              isLoading={isLoading}
              error={error}
              emptyLabel={t("live.conversations.drawer.empty")}
              onOpenUpstreamAccount={onOpenUpstreamAccount}
              scrollElement={drawerBodyElement}
            />
            {isLoadingMore ? (
              <div className="flex items-center justify-center gap-2 py-2 text-sm text-base-content/60">
                <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
                <span>{t("live.conversations.drawer.loadingMore")}</span>
              </div>
            ) : null}
          </div>
        ) : null}
        {activeTab === "settings" ? (
          <div
            id={`${titleId}-panel-settings`}
            role="tabpanel"
            aria-labelledby={`${titleId}-tab-settings`}
          >
            {bindingPanel}
          </div>
        ) : null}
        {activeTab === "operations" ? (
          <div
            id={`${titleId}-panel-operations`}
            role="tabpanel"
            aria-labelledby={`${titleId}-tab-operations`}
          >
            {operationsPanel}
          </div>
        ) : null}
      </AccountDetailDrawerShell>
      <Dialog
        open={open && bindingOwnerConfirmOpen}
        onOpenChange={(nextOpen) => {
          if (!bindingSaving) {
            setBindingOwnerConfirmOpen(nextOpen);
            if (!nextOpen) setBindingOwnerConfirmAllowsRemoteOverwrite(false);
          }
        }}
      >
        <DialogContent
          role="alertdialog"
          container={drawerBodyElement}
          className="flex max-h-[calc(100dvh-0.75rem)] flex-col overflow-hidden p-0 desktop:max-h-[calc(100dvh-2rem)]"
        >
          <div className="shrink-0 border-b border-base-300/80 px-5 py-4 desktop:px-6">
            <DialogHeader>
              <DialogTitle>{t("live.conversations.drawer.binding.ownerConfirm.title")}</DialogTitle>
              <DialogDescription>
                {t("live.conversations.drawer.binding.ownerConfirm.description", {
                  owner: bindingOwnerConfirmLabel,
                })}
              </DialogDescription>
            </DialogHeader>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5 desktop:px-6">
            <p className="rounded-xl border border-warning/25 bg-warning/10 px-3 py-2 text-sm leading-6 text-base-content/82">
              {t("live.conversations.drawer.binding.ownerConfirm.risk")}
            </p>
          </div>
          <DialogFooter className="shrink-0 border-t border-base-300/80 bg-base-100/94 px-5 pb-[max(env(safe-area-inset-bottom),1rem)] pt-4 backdrop-blur desktop:px-6 desktop:py-4">
            <Button
              type="button"
              variant="ghost"
              disabled={bindingSaving}
              onClick={() => setBindingOwnerConfirmOpen(false)}
            >
              {t("live.conversations.drawer.binding.ownerConfirm.cancel")}
            </Button>
            <Button
              type="button"
              variant="destructive"
              disabled={bindingSaving}
              onClick={() => {
                setBindingOwnerConfirmOpen(false);
                void saveBinding({
                  skipOwnerWarning: true,
                  allowRemoteOverwrite: bindingOwnerConfirmAllowsRemoteOverwrite,
                });
              }}
            >
              {bindingSaving
                ? t("live.conversations.drawer.binding.saving")
                : t("live.conversations.drawer.binding.ownerConfirm.confirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

export function PromptCacheConversationTable({
  stats,
  isLoading,
  error,
  expandedPromptCacheKeys,
  onToggleExpandedPromptCacheKey,
  onOpenUpstreamAccount,
  keyColumnLabel,
  emptyLabel,
  historyQueryForConversationKey,
}: PromptCacheConversationTableProps) {
  const { t, locale } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  const [historyDrawerPromptCacheKey, setHistoryDrawerPromptCacheKey] = useState<string | null>(
    null,
  );
  const [internalExpandedPromptCacheKeys, setInternalExpandedPromptCacheKeys] = useState<string[]>(
    [],
  );
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const isExpansionControlled = expandedPromptCacheKeys != null;

  useEffect(() => {
    const timer = setInterval(() => {
      setNow(Date.now());
    }, PROMPT_CACHE_NOW_TICK_MS);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!stats) return;
    setNow(Date.now());
  }, [stats]);

  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag]);
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: "currency",
        currency: "USD",
        maximumFractionDigits: 4,
      }),
    [localeTag],
  );
  const dateFormatter = useMemo(
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

  const chartRangeOverride = useMemo(() => {
    if (!stats || stats.conversations.length === 0) return null;
    const earliestCreatedAt = stats.conversations.reduce<number | null>(
      (earliest, conversation) => {
        const createdAt = parseEpoch(conversation.createdAt);
        if (createdAt == null) return earliest;
        return earliest == null ? createdAt : Math.min(earliest, createdAt);
      },
      null,
    );
    if (earliestCreatedAt == null) return null;
    const chartRangeStart = Math.max(earliestCreatedAt, now - PROMPT_CACHE_CHART_MAX_WINDOW_MS);
    return {
      rangeStart: new Date(chartRangeStart).toISOString(),
      rangeEnd: new Date(now).toISOString(),
    };
  }, [now, stats]);

  const chartHours = useMemo(() => {
    const rangeStartEpoch = parseEpoch(chartRangeOverride?.rangeStart ?? stats?.rangeStart ?? "");
    const rangeEndEpoch = parseEpoch(chartRangeOverride?.rangeEnd ?? stats?.rangeEnd ?? "");
    if (rangeStartEpoch == null || rangeEndEpoch == null || rangeEndEpoch <= rangeStartEpoch) {
      return 24;
    }
    return Math.max(1, Math.ceil((rangeEndEpoch - rangeStartEpoch) / 3_600_000));
  }, [
    chartRangeOverride?.rangeEnd,
    chartRangeOverride?.rangeStart,
    stats?.rangeEnd,
    stats?.rangeStart,
  ]);

  const footerNote = useMemo(() => {
    if (!stats || stats.implicitFilter.filteredCount <= 0 || stats.implicitFilter.kind == null) {
      return null;
    }
    if (stats.implicitFilter.kind === "inactiveOutside24h") {
      if (stats.selectionMode === "activityWindow" && stats.selectedActivityHours != null) {
        return t("live.conversations.implicitFilter.inactiveOutsideActivityWindow", {
          count: stats.implicitFilter.filteredCount,
          hours: stats.selectedActivityHours,
        });
      }
      return t("live.conversations.implicitFilter.inactiveOutside24h", {
        count: stats.implicitFilter.filteredCount,
      });
    }
    return t("live.conversations.implicitFilter.cappedTo50", {
      count: stats.implicitFilter.filteredCount,
    });
  }, [stats, t]);

  const tooltipLabels = useMemo(
    () => ({
      status: t("live.conversations.chart.tooltip.status"),
      requestTokens: t("live.conversations.chart.tooltip.requestTokens"),
      cumulativeTokens: t("live.conversations.chart.tooltip.cumulativeTokens"),
    }),
    [t],
  );
  const chartInteractionHint = t("live.chart.tooltip.instructions");
  const resolvedKeyColumnLabel = keyColumnLabel ?? t("live.conversations.table.promptCacheKey");
  const resolvedEmptyLabel = emptyLabel ?? t("live.conversations.empty");
  const chartAriaLabel = t("live.conversations.chartAria", {
    hours: chartHours,
  });
  const chartColumnLabel = t("live.conversations.table.chartWindow", {
    hours: chartHours,
  });
  const rangeStart = chartRangeOverride?.rangeStart ?? stats?.rangeStart ?? "";
  const rangeEnd = chartRangeOverride?.rangeEnd ?? stats?.rangeEnd ?? "";
  const conversationChartMax = useMemo(
    () => findVisibleConversationChartMax(stats?.conversations ?? [], rangeStart, rangeEnd),
    [rangeEnd, rangeStart, stats?.conversations],
  );
  const totalLabels = useMemo(
    () => ({
      requestCount: t("live.conversations.table.requestCount"),
      totalTokens: t("live.conversations.table.totalTokens"),
      totalCost: t("live.conversations.table.totalCost"),
      requestCountCompact: t("live.conversations.table.requestCountCompact"),
      totalTokensCompact: t("live.conversations.table.totalTokensCompact"),
      time: t("live.conversations.table.time"),
      createdAtShort: t("live.conversations.table.createdAtShort"),
      lastActivityAtShort: t("live.conversations.table.lastActivityAtShort"),
    }),
    [t],
  );
  const previewLabels = useMemo(
    () => ({
      empty: t("live.conversations.preview.empty"),
      expandAction: t("live.conversations.actions.expandPreview"),
      collapseAction: t("live.conversations.actions.collapsePreview"),
      historyAction: t("live.conversations.actions.openHistory"),
    }),
    [t],
  );
  const fallbackAccountLabel = useMemo(
    () => (id: number) =>
      t("live.conversations.accountLabel.idFallback", {
        id: String(Math.trunc(id)),
      }),
    [t],
  );
  const effectiveExpandedPromptCacheKeys = isExpansionControlled
    ? expandedPromptCacheKeys
    : internalExpandedPromptCacheKeys;
  const expandedPromptCacheKeySet = useMemo(
    () => new Set(effectiveExpandedPromptCacheKeys),
    [effectiveExpandedPromptCacheKeys],
  );

  useEffect(() => {
    if (isExpansionControlled || !stats) return;
    const visiblePromptCacheKeys = new Set(
      stats.conversations.map((conversation) => conversation.promptCacheKey),
    );
    setInternalExpandedPromptCacheKeys((current) =>
      current.filter((promptCacheKey) => visiblePromptCacheKeys.has(promptCacheKey)),
    );
  }, [isExpansionControlled, stats]);

  const openAccountDrawer = (account: PromptCacheConversationUpstreamAccount) => {
    if (!canOpenPromptCacheUpstreamAccount(account)) return;
    setHistoryDrawerPromptCacheKey(null);
    onOpenUpstreamAccount?.(
      Math.trunc(Number(account.upstreamAccountId)),
      resolveUpstreamAccountLabel(account, fallbackAccountLabel),
    );
  };
  const openHistoryDrawer = (promptCacheKey: string) => {
    setHistoryDrawerPromptCacheKey(promptCacheKey);
  };
  const closeHistoryDrawer = () => {
    setHistoryDrawerPromptCacheKey(null);
  };
  const togglePromptCachePreview = (promptCacheKey: string) => {
    if (!isExpansionControlled) {
      setInternalExpandedPromptCacheKeys((current) =>
        current.includes(promptCacheKey)
          ? current.filter((value) => value !== promptCacheKey)
          : [...current, promptCacheKey],
      );
    }
    onToggleExpandedPromptCacheKey?.(promptCacheKey);
  };

  if (error) {
    return (
      <Alert variant="error">
        <span>{error}</span>
      </Alert>
    );
  }

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <Spinner size="lg" aria-label={t("chart.loadingDetailed")} />
      </div>
    );
  }

  if (!stats || stats.conversations.length === 0) {
    return (
      <div className="space-y-2">
        <Alert>{resolvedEmptyLabel}</Alert>
        {footerNote ? <p className="px-1 text-[11px] text-base-content/55">{footerNote}</p> : null}
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <div className="overflow-hidden rounded-xl border border-base-300/75 bg-base-100/55">
        <div className="space-y-3 p-3 sm:hidden">
          {stats.conversations.map((conversation) => {
            const createdAtLabel = formatDateLabel(conversation.createdAt, dateFormatter);
            const lastActivityLabel = formatDateLabel(conversation.lastActivityAt, dateFormatter);
            const isExpanded = expandedPromptCacheKeySet.has(conversation.promptCacheKey);

            return (
              <article
                key={`${conversation.promptCacheKey}-mobile`}
                className="space-y-3 rounded-lg border border-base-300/70 bg-base-100/70 p-3"
              >
                <div className="space-y-2">
                  <div className="space-y-2">
                    <div className="min-w-0 space-y-1">
                      <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                        {resolvedKeyColumnLabel}
                      </div>
                      <div className="break-all font-mono text-xs">
                        {conversation.promptCacheKey}
                      </div>
                    </div>
                    <div className="flex items-center gap-1">
                      <button
                        type="button"
                        className="inline-flex h-8 w-8 items-center justify-center rounded-full border border-base-300/70 bg-base-100/80 text-base-content/72 transition hover:border-primary/40 hover:text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                        aria-label={
                          isExpanded ? previewLabels.collapseAction : previewLabels.expandAction
                        }
                        aria-expanded={isExpanded}
                        onClick={() => togglePromptCachePreview(conversation.promptCacheKey)}
                      >
                        <AppIcon
                          name={isExpanded ? "chevron-up" : "chevron-down"}
                          className="h-4 w-4"
                          aria-hidden
                        />
                      </button>
                      <button
                        type="button"
                        className="inline-flex h-8 w-8 items-center justify-center rounded-full border border-base-300/70 bg-base-100/80 text-base-content/72 transition hover:border-primary/40 hover:text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                        aria-label={previewLabels.historyAction}
                        onClick={() => openHistoryDrawer(conversation.promptCacheKey)}
                      >
                        <AppIcon name="account-details-outline" className="h-4 w-4" aria-hidden />
                      </button>
                    </div>
                  </div>
                  {isExpanded ? (
                    <div className="rounded-lg border border-base-300/70 bg-base-200/30 p-3">
                      <PromptCacheConversationInvocationTable
                        records={conversation.recentInvocations.map(
                          buildInvocationFromPromptCachePreview,
                        )}
                        isLoading={false}
                        emptyLabel={previewLabels.empty}
                        onOpenUpstreamAccount={onOpenUpstreamAccount}
                      />
                    </div>
                  ) : null}
                </div>

                <div className="space-y-1">
                  <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {t("live.conversations.table.upstreamAccounts")}
                  </div>
                  <UpstreamAccountsBlock
                    upstreamAccounts={conversation.upstreamAccounts}
                    labels={totalLabels}
                    numberFormatter={numberFormatter}
                    currencyFormatter={currencyFormatter}
                    fallbackAccountLabel={fallbackAccountLabel}
                    onOpenAccountDetail={openAccountDrawer}
                  />
                </div>

                <div className="space-y-1">
                  <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {t("live.conversations.table.summary")}
                  </div>
                  <SummaryBlock
                    conversation={conversation}
                    labels={totalLabels}
                    numberFormatter={numberFormatter}
                    currencyFormatter={currencyFormatter}
                  />
                </div>

                <div className="space-y-1">
                  <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {totalLabels.time}
                  </div>
                  <dl className="space-y-1 text-xs">
                    <div className="flex items-center justify-between gap-3">
                      <dt className="text-base-content/60">{totalLabels.createdAtShort}</dt>
                      <dd className="text-right">{createdAtLabel}</dd>
                    </div>
                    <div className="flex items-center justify-between gap-3">
                      <dt className="text-base-content/60">{totalLabels.lastActivityAtShort}</dt>
                      <dd className="text-right">{lastActivityLabel}</dd>
                    </div>
                  </dl>
                </div>

                <div className="space-y-1">
                  <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {chartColumnLabel}
                  </div>
                  <ConversationSparkline
                    conversation={conversation}
                    rangeStart={rangeStart}
                    rangeEnd={rangeEnd}
                    maxCumulativeTokens={conversationChartMax}
                    localeTag={localeTag}
                    tooltipLabels={tooltipLabels}
                    interactionHint={chartInteractionHint}
                    ariaLabel={`${conversation.promptCacheKey} ${chartAriaLabel}`}
                    conversationKey={conversation.promptCacheKey}
                  />
                </div>
              </article>
            );
          })}
        </div>

        <table className="hidden w-full table-fixed text-xs sm:table">
          <thead className="bg-base-200/70 uppercase tracking-[0.08em] text-base-content/65">
            <tr>
              <th className="w-[18%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {resolvedKeyColumnLabel}
              </th>
              <th className="w-[34%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {t("live.conversations.table.upstreamAccounts")}
              </th>
              <th className="w-[15%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {t("live.conversations.table.summary")}
              </th>
              <th className="w-[15%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {totalLabels.time}
              </th>
              <th className="w-[18%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {chartColumnLabel}
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-base-300/65">
            {stats.conversations.map((conversation) => {
              const isExpanded = expandedPromptCacheKeySet.has(conversation.promptCacheKey);

              return (
                <Fragment key={conversation.promptCacheKey}>
                  <tr className="transition-colors hover:bg-primary/6">
                    <td className="max-w-0 px-2 py-2 align-top sm:px-3 sm:py-3">
                      <div className="space-y-2">
                        <div
                          className="truncate font-mono text-xs"
                          title={conversation.promptCacheKey}
                        >
                          {conversation.promptCacheKey}
                        </div>
                        <div className="flex items-center gap-1">
                          <button
                            type="button"
                            className="inline-flex h-8 w-8 items-center justify-center rounded-full border border-base-300/70 bg-base-100/80 text-base-content/72 transition hover:border-primary/40 hover:text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                            aria-label={
                              isExpanded ? previewLabels.collapseAction : previewLabels.expandAction
                            }
                            aria-expanded={isExpanded}
                            onClick={() => togglePromptCachePreview(conversation.promptCacheKey)}
                          >
                            <AppIcon
                              name={isExpanded ? "chevron-up" : "chevron-down"}
                              className="h-4 w-4"
                              aria-hidden
                            />
                          </button>
                          <button
                            type="button"
                            className="inline-flex h-8 w-8 items-center justify-center rounded-full border border-base-300/70 bg-base-100/80 text-base-content/72 transition hover:border-primary/40 hover:text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                            aria-label={previewLabels.historyAction}
                            onClick={() => openHistoryDrawer(conversation.promptCacheKey)}
                          >
                            <AppIcon
                              name="account-details-outline"
                              className="h-4 w-4"
                              aria-hidden
                            />
                          </button>
                        </div>
                      </div>
                    </td>
                    <td className="px-2 py-2 align-top sm:px-3 sm:py-3">
                      <UpstreamAccountsBlock
                        upstreamAccounts={conversation.upstreamAccounts}
                        labels={totalLabels}
                        numberFormatter={numberFormatter}
                        currencyFormatter={currencyFormatter}
                        fallbackAccountLabel={fallbackAccountLabel}
                        onOpenAccountDetail={openAccountDrawer}
                      />
                    </td>
                    <td className="px-2 py-2 align-top sm:px-3 sm:py-3">
                      <SummaryBlock
                        conversation={conversation}
                        labels={totalLabels}
                        numberFormatter={numberFormatter}
                        currencyFormatter={currencyFormatter}
                      />
                    </td>
                    <td className="px-2 py-2 align-top sm:px-3 sm:py-3">
                      <div className="space-y-1.5 text-[11px]">
                        <div className="grid grid-cols-[2rem_minmax(0,1fr)] items-center gap-x-2">
                          <span className="text-base-content/60">{totalLabels.createdAtShort}</span>
                          <span className="whitespace-nowrap font-medium tabular-nums">
                            {formatDateLabel(conversation.createdAt, dateFormatter)}
                          </span>
                        </div>
                        <div className="grid grid-cols-[2rem_minmax(0,1fr)] items-center gap-x-2">
                          <span className="text-base-content/60">
                            {totalLabels.lastActivityAtShort}
                          </span>
                          <span className="whitespace-nowrap font-medium tabular-nums">
                            {formatDateLabel(conversation.lastActivityAt, dateFormatter)}
                          </span>
                        </div>
                      </div>
                    </td>
                    <td className="px-2 py-2 align-top sm:px-3 sm:py-3">
                      <ConversationSparkline
                        conversation={conversation}
                        rangeStart={rangeStart}
                        rangeEnd={rangeEnd}
                        maxCumulativeTokens={conversationChartMax}
                        localeTag={localeTag}
                        tooltipLabels={tooltipLabels}
                        interactionHint={chartInteractionHint}
                        ariaLabel={`${conversation.promptCacheKey} ${chartAriaLabel}`}
                        conversationKey={conversation.promptCacheKey}
                      />
                    </td>
                  </tr>
                  {isExpanded ? (
                    <tr className="bg-base-200/20">
                      <td colSpan={5} className="px-3 pb-4 pt-0">
                        <div className="border-t border-base-300/60 pt-3">
                          <PromptCacheConversationInvocationTable
                            records={conversation.recentInvocations.map(
                              buildInvocationFromPromptCachePreview,
                            )}
                            isLoading={false}
                            emptyLabel={previewLabels.empty}
                            onOpenUpstreamAccount={onOpenUpstreamAccount}
                          />
                        </div>
                      </td>
                    </tr>
                  ) : null}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </div>
      {footerNote ? <p className="px-1 text-[11px] text-base-content/55">{footerNote}</p> : null}
      <PromptCacheConversationHistoryDrawer
        open={historyDrawerPromptCacheKey != null}
        conversationKey={historyDrawerPromptCacheKey}
        onClose={closeHistoryDrawer}
        t={t}
        onOpenUpstreamAccount={onOpenUpstreamAccount}
        historyQueryForConversationKey={historyQueryForConversationKey}
      />
    </div>
  );
}
