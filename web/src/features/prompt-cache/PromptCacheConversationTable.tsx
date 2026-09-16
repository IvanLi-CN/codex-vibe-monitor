import { Fragment, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import { Chip } from "../../components/ui/chip";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../../components/ui/dialog";
import { Input } from "../../components/ui/input";
import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import { SelectField, type SelectFieldOption } from "../../components/ui/select-field";
import { Spinner } from "../../components/ui/spinner";
import {
  resolveConversationDetailScope,
  useConversationDetailTopics,
} from "../../hooks/useConversationDetailTopics";
import { useLatestDebouncedMutation } from "../../hooks/useLatestDebouncedMutation";
import { useTranslation } from "../../i18n";
import type {
  ApiInvocation,
  EffectiveRoutingRule,
  EffectiveRoutingRuleSource,
  ForwardProxyBindingNode,
  InvocationRecordsQuery,
  InvocationRecordsResponse,
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
  fetchPromptCacheConversationBinding,
  fetchPromptCacheConversationOperationEvents,
  fetchUpstreamAccounts,
  resetPromptCacheConversationAffinity,
  updatePromptCacheConversationBinding,
} from "../../lib/api";
import { invocationStableKey } from "../../lib/invocation";
import { mergeInvocationRecordCollections } from "../../lib/invocationLiveMerge";
import { buildInvocationFromPromptCachePreview } from "../../lib/promptCacheLive";
import { AccountDetailDrawerShell } from "../account-pool/AccountDetailDrawerShell";
import {
  EffectiveRoutingRuleCard,
  type EffectiveRoutingRuleCardRowKey,
  type EffectiveRoutingRuleCardRowValueOverride,
} from "../account-pool/EffectiveRoutingRuleCard";
import { InvocationCardList } from "../invocations/InvocationTable";
import { AppIcon } from "../shared/AppIcon";
import { ConversationSparkline } from "./KeyedConversationTable";
import { FALLBACK_CELL, findVisibleConversationChartMax } from "./keyedConversationChart";
import { PromptCacheConversationActivityOverview } from "./PromptCacheActivityOverview";

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

type PromptCacheConversationStickyRoute = NonNullable<
  PromptCacheConversationBindingResponse["stickyRoutes"]
>[number];

const PROMPT_CACHE_NOW_TICK_MS = 30_000;
const PROMPT_CACHE_CHART_MAX_WINDOW_MS = 24 * 3_600_000;
const PROMPT_CACHE_HISTORY_PAGE_SIZE = 50;
const PROMPT_CACHE_OPERATION_EVENT_PAGE_SIZE = 20;

const PROMPT_CACHE_HISTORY_TOP_INSERT_THRESHOLD_PX = 96;

type ConversationBindingDraftKind = PromptCacheConversationBindingKind;
export type PromptCacheConversationDrawerTab =
  | "overview"
  | "calls"
  | "routing"
  | "settings"
  | "operations";
type PromptCacheConversationOperationFilter = "all" | PromptCacheConversationOperationInfoType;
type PromptCacheConversationRoutingModelFilter = "any" | "all" | string;
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

type ConversationInlinePatch =
  | { allowSwitchUpstream: boolean | null }
  | { fastModeRewriteMode: PromptCacheConversationRewriteMode | null }
  | { imageToolRewriteMode: PromptCacheConversationRewriteMode | null }
  | { codexImagegenRewriteMode: PromptCacheConversationRewriteMode | null }
  | {
      availableModels: string[] | null;
      availableModelsMode?: "allowlist" | "denylist" | null;
    }
  | { forwardProxyKeys: string[] | null }
  | { timeouts: NonNullable<UpdateGroupAccountRoutingRulePayload["timeouts"]> };

type ConversationInlineMutationEntry = {
  conversationKey: string;
  fields: ConversationInlinePolicyField[];
  patch: ConversationInlinePatch;
};

function isOlderBindingSnapshot(
  nextBinding: PromptCacheConversationBindingResponse,
  confirmedBinding: PromptCacheConversationBindingResponse | null,
): boolean {
  if (!nextBinding.updatedAt || !confirmedBinding?.updatedAt) return false;
  const nextTimestamp = Date.parse(nextBinding.updatedAt);
  const confirmedTimestamp = Date.parse(confirmedBinding.updatedAt);
  return Number.isFinite(nextTimestamp) && Number.isFinite(confirmedTimestamp)
    ? nextTimestamp < confirmedTimestamp
    : false;
}

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
    availableModelsMode:
      binding.availableModelsMode ?? (binding.availableModels == null ? "denylist" : "allowlist"),
    availableModelsDefined: binding.availableModels != null || binding.availableModelsMode != null,
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

function applyConversationInlinePatch(
  binding: PromptCacheConversationBindingResponse,
  patch: ConversationInlinePatch,
): PromptCacheConversationBindingResponse {
  const next = { ...binding };
  const nextPolicyFieldSources: NonNullable<
    PromptCacheConversationBindingResponse["policyFieldSources"]
  > = {
    allowSwitchUpstream: binding.policyFieldSources?.allowSwitchUpstream ?? "account",
    fastModeRewriteMode: binding.policyFieldSources?.fastModeRewriteMode ?? "account",
    imageToolRewriteMode: binding.policyFieldSources?.imageToolRewriteMode ?? "account",
    codexImagegenRewriteMode: binding.policyFieldSources?.codexImagegenRewriteMode ?? "account",
    availableModels: binding.policyFieldSources?.availableModels ?? "account",
    availableModelsMode: binding.policyFieldSources?.availableModelsMode ?? "account",
    forwardProxyKey: binding.policyFieldSources?.forwardProxyKey ?? "account",
  };
  const nextTimeoutFieldSources = { ...binding.timeoutFieldSources };
  next.policyFieldSources = nextPolicyFieldSources;
  next.timeoutFieldSources = nextTimeoutFieldSources;

  if ("allowSwitchUpstream" in patch) {
    nextPolicyFieldSources.allowSwitchUpstream =
      patch.allowSwitchUpstream == null ? "account" : "conversation";
    next.allowSwitchUpstream = patch.allowSwitchUpstream;
  }
  if ("fastModeRewriteMode" in patch) {
    nextPolicyFieldSources.fastModeRewriteMode =
      patch.fastModeRewriteMode == null ? "account" : "conversation";
    next.fastModeRewriteMode = patch.fastModeRewriteMode;
  }
  if ("imageToolRewriteMode" in patch) {
    nextPolicyFieldSources.imageToolRewriteMode =
      patch.imageToolRewriteMode == null ? "account" : "conversation";
    next.imageToolRewriteMode = patch.imageToolRewriteMode;
  }
  if ("codexImagegenRewriteMode" in patch) {
    nextPolicyFieldSources.codexImagegenRewriteMode =
      patch.codexImagegenRewriteMode == null ? "account" : "conversation";
    next.codexImagegenRewriteMode = patch.codexImagegenRewriteMode;
  }
  if ("availableModels" in patch) {
    nextPolicyFieldSources.availableModels =
      patch.availableModels == null ? "account" : "conversation";
    next.availableModels = patch.availableModels ? [...patch.availableModels] : null;
    if ("availableModelsMode" in patch) next.availableModelsMode = patch.availableModelsMode;
  }
  if ("forwardProxyKeys" in patch) {
    nextPolicyFieldSources.forwardProxyKey =
      patch.forwardProxyKeys == null ? "account" : "conversation";
    next.forwardProxyKeys =
      patch.forwardProxyKeys == null ? undefined : [...patch.forwardProxyKeys];
  }
  if ("timeouts" in patch) {
    next.timeouts = {
      ...(binding.timeouts ?? {}),
      ...patch.timeouts,
    } as PromptCacheConversationBindingResponse["timeouts"];
    for (const [key, value] of Object.entries(patch.timeouts)) {
      nextTimeoutFieldSources[
        key as keyof NonNullable<PromptCacheConversationBindingResponse["timeoutFieldSources"]>
      ] = value == null ? "account" : "conversation";
    }
  }

  return next;
}

function conversationBindingPayloadBase(
  binding: PromptCacheConversationBindingResponse | null,
):
  | { bindingKind: "group"; groupName: string }
  | { bindingKind: "upstreamAccount"; upstreamAccountId: number }
  | { bindingKind: "none" } {
  if (binding?.bindingKind === "group" && binding.groupName) {
    return { bindingKind: "group", groupName: binding.groupName };
  }
  if (binding?.bindingKind === "upstreamAccount" && binding.upstreamAccountId != null) {
    return { bindingKind: "upstreamAccount", upstreamAccountId: binding.upstreamAccountId };
  }
  return { bindingKind: "none" };
}

function mergeConversationInlinePatch(
  base: ConversationInlinePatch | null,
  patch: ConversationInlinePatch,
): ConversationInlinePatch {
  if (!base) return patch;
  if ("timeouts" in base && "timeouts" in patch) {
    return { timeouts: { ...base.timeouts, ...patch.timeouts } };
  }
  return { ...base, ...patch } as ConversationInlinePatch;
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

function conversationOperationInfoTypeChipTone(
  infoType: PromptCacheConversationOperationInfoType,
): "primary" | "info" | "accent" {
  switch (infoType) {
    case "routing":
      return "primary";
    case "forwardProxy":
      return "info";
    case "requestRewrite":
      return "accent";
  }
}

function conversationOperationOriginChipTone(origin: string): "secondary" | "warning" | "info" {
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

function routingSelectionHandoffLabel(
  audit: PoolRoutingSelectionAudit,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  const admission = audit.handoffAdmission;
  if (!admission) return null;
  const decisionKey = `live.routing.record.handoffDecisions.${admission.decision}`;
  const phaseKey = `live.routing.record.handoffPhases.${admission.phase}`;
  const decision = t(decisionKey) === decisionKey ? admission.decision : t(decisionKey);
  const phase = t(phaseKey) === phaseKey ? admission.phase : t(phaseKey);
  const triggerKey = `live.routing.record.handoffTriggers.${admission.trigger ?? ""}`;
  const trigger = admission.trigger
    ? t(triggerKey) === triggerKey
      ? admission.trigger
      : t(triggerKey)
    : null;
  return t(
    trigger
      ? "live.routing.record.handoffAdmissionValueWithTrigger"
      : "live.routing.record.handoffAdmissionValue",
    {
      decision,
      phase,
      count: admission.verificationSuccessCount,
      ...(trigger ? { trigger } : {}),
    },
  );
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
        <InvocationCardList
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
    <InvocationCardList
      records={records}
      isLoading={isLoading}
      error={error}
      emptyLabel={emptyLabel}
      onOpenUpstreamAccount={onOpenUpstreamAccount}
      scrollElement={scrollElement}
    />
  );
}

type ConversationOperationTranslation = (
  key: string,
  values?: Record<string, string | number>,
) => string;

function ConversationOperationEventHeader({
  event,
  t,
}: {
  event: PromptCacheConversationOperationEvent;
  t: ConversationOperationTranslation;
}) {
  return (
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div className="min-w-0 flex-1 space-y-2">
        <div className="flex flex-wrap gap-2">
          {event.infoTypes.map((infoType) => (
            <Chip
              key={`${event.id}-${infoType}`}
              tone={conversationOperationInfoTypeChipTone(infoType)}
            >
              {conversationOperationInfoTypeLabel(infoType, t)}
            </Chip>
          ))}
          <Chip tone={conversationOperationOriginChipTone(event.origin)}>
            {conversationOperationOriginLabel(event.origin, t)}
          </Chip>
          {event.routingScope ? (
            <Chip tone="secondary">
              {event.routingScope.kind === "all"
                ? t("live.conversations.drawer.operations.modelScope.all")
                : t("live.conversations.drawer.operations.modelScope.model", {
                    model: event.routingScope.modelKey ?? FALLBACK_CELL,
                  })}
            </Chip>
          ) : null}
        </div>
        {event.routingScope?.kind === "model" &&
        event.routingScope.requestModel &&
        event.routingScope.requestModel !== event.routingScope.modelKey ? (
          <p className="font-mono text-[11px] text-base-content/55">
            {t("live.conversations.drawer.operations.requestModel", {
              model: event.routingScope.requestModel,
            })}
          </p>
        ) : null}
        <p className="break-words text-sm font-semibold text-base-content">
          {conversationOperationActionLabel(event.action, event.headline, t)}
        </p>
      </div>
      <span className="text-xs text-base-content/58">
        {formatConversationOperationOccurredAt(event.occurredAt)}
      </span>
    </div>
  );
}

function ConversationOperationEventChanges({
  event,
  t,
}: {
  event: PromptCacheConversationOperationEvent;
  t: ConversationOperationTranslation;
}) {
  return (
    <>
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
      {(event.stickyTransitions?.length ?? 0) > 0 ? (
        <div className="space-y-1 rounded border border-base-content/10 bg-base-200/35 p-2 text-xs text-base-content/72">
          {(event.stickyTransitions ?? []).map((transition) => (
            <p key={`${event.id}-${transition.modelKey ?? "all"}`}>
              {t("live.conversations.drawer.operations.modelTransition", {
                model:
                  transition.modelKey ?? t("live.conversations.drawer.operations.modelScope.all"),
                from: conversationOperationStickySnapshotLabel(transition.before, t),
                to: conversationOperationStickySnapshotLabel(transition.after, t),
              })}
            </p>
          ))}
        </div>
      ) : null}
    </>
  );
}

function ConversationOperationRoutingAudit({
  audit,
  t,
}: {
  audit: NonNullable<
    NonNullable<PromptCacheConversationOperationEvent["routingContext"]>["routingSelectionAudit"]
  >;
  t: ConversationOperationTranslation;
}) {
  return (
    <div className="space-y-1 rounded border border-info/25 bg-info/5 p-2">
      <p className="font-medium text-base-content">
        {t("table.poolAttempts.routingDecision.summary", {
          account: audit.selectedAccountName,
          count: audit.eligibleCandidateCount,
        })}
      </p>
      <p>{routingSelectionWinnerLabel(audit, t)}</p>
      {routingSelectionHandoffLabel(audit, t) ? (
        <p>
          {t("live.routing.record.handoffAdmission")}: {routingSelectionHandoffLabel(audit, t)}
        </p>
      ) : null}
      {audit.selectedScore ? (
        <p data-testid="conversation-routing-selection-score">
          {routingSelectionScoreLabel(audit.selectedAccountName, audit.selectedScore, t)}
        </p>
      ) : null}
      {audit.comparedScore && audit.comparedAccountName ? (
        <p>{routingSelectionScoreLabel(audit.comparedAccountName, audit.comparedScore, t)}</p>
      ) : null}
      {audit.excludedCandidates.map((candidate) => (
        <p key={`${candidate.accountId}-${candidate.reasonCode}`}>
          {routingSelectionExclusionLabel(candidate.accountName, candidate.reasonCode, t)}
        </p>
      ))}
    </div>
  );
}

function ConversationOperationRoutingLinks({
  event,
  t,
}: {
  event: PromptCacheConversationOperationEvent;
  t: ConversationOperationTranslation;
}) {
  return (
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
              : "live.conversations.drawer.operations.routingContext.trigger",
            { attemptId: event.routingContext.triggerAttemptId },
          )}
        </Link>
      ) : null}
      {routingInvocationRecordHref(event) ? (
        <Link
          className="inline-flex font-mono text-[11px] text-primary underline underline-offset-2"
          to={routingInvocationRecordHref(event) ?? "#"}
          aria-label={t(
            "live.conversations.drawer.operations.routingContext.invocationRecordLabel",
            { id: event.invokeId ?? event.routingContext?.triggerAttemptId ?? "" },
          )}
          title={t("live.conversations.drawer.operations.routingContext.invocationRecordLabel", {
            id: event.invokeId ?? event.routingContext?.triggerAttemptId ?? "",
          })}
        >
          {t("live.conversations.drawer.operations.routingContext.invocationRecord", {
            id: event.invokeId ?? event.routingContext?.triggerAttemptId ?? "",
          })}
        </Link>
      ) : null}
    </div>
  );
}

function ConversationOperationEventRouting({
  event,
  t,
}: {
  event: PromptCacheConversationOperationEvent;
  t: ConversationOperationTranslation;
}) {
  if (!event.infoTypes.includes("routing") || !conversationOperationShowsRoutingReason(event)) {
    return null;
  }
  const audit = event.routingContext?.routingSelectionAudit;
  return (
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
      {audit ? <ConversationOperationRoutingAudit audit={audit} t={t} /> : null}
      <ConversationOperationRoutingLinks event={event} t={t} />
    </div>
  );
}

function ConversationOperationEventCard({
  event,
  t,
}: {
  event: PromptCacheConversationOperationEvent;
  t: ConversationOperationTranslation;
}) {
  return (
    <article className="space-y-3 rounded-xl border border-base-content/10 bg-base-100/80 p-4">
      <ConversationOperationEventHeader event={event} t={t} />
      <ConversationOperationEventChanges event={event} t={t} />
      <ConversationOperationEventRouting event={event} t={t} />
      {event.invokeId ? (
        <p className="break-all font-mono text-[11px] text-base-content/58">
          {t("live.conversations.drawer.operations.invokeId", { invokeId: event.invokeId })}
        </p>
      ) : null}
    </article>
  );
}

function ConversationPolicySelectEditor({
  value,
  disabled,
  ariaLabel,
  options,
  onChange,
}: {
  value: string;
  disabled: boolean;
  ariaLabel: string;
  options: readonly SelectFieldOption[];
  onChange: (value: string) => void;
}) {
  return (
    <SelectField
      value={value}
      disabled={disabled}
      aria-label={ariaLabel}
      size="sm"
      options={options}
      onValueChange={onChange}
    />
  );
}

function ConversationAvailableModelsEditor({
  value,
  disabled,
  ariaLabel,
  placeholder,
  isEmpty,
  requiredLabel,
  applyLabel,
  applyDisabled,
  onChange,
  onApply,
}: {
  value: string;
  disabled: boolean;
  ariaLabel: string;
  placeholder: string;
  isEmpty: boolean;
  requiredLabel: string;
  applyLabel: string;
  applyDisabled: boolean;
  onChange: (value: string) => void;
  onApply: () => void;
}) {
  return (
    <div className="space-y-2">
      <Input
        value={value}
        disabled={disabled}
        aria-label={ariaLabel}
        placeholder={placeholder}
        className="h-9"
        onChange={(event) => onChange(event.target.value)}
      />
      {isEmpty ? <p className="text-xs text-error">{requiredLabel}</p> : null}
      <Button type="button" size="sm" disabled={applyDisabled} onClick={onApply}>
        {applyLabel}
      </Button>
    </div>
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
  const bindingHydratedRef = useRef(false);
  const bindingScopeRef = useRef<{ conversationKey: string | null; open: boolean }>({
    conversationKey,
    open,
  });
  const bindingTopicSseUnavailableCapturedRef = useRef(false);
  const bindingTopicSseUnavailableCaptureKeyRef = useRef<string | null>(null);
  const staleBindingTopicPayloadRef = useRef<PromptCacheConversationBindingResponse | null>(null);
  const confirmedBindingRef = useRef<PromptCacheConversationBindingResponse | null>(null);
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
  const [inlinePolicyErrors, setInlinePolicyErrors] = useState<
    Partial<Record<ConversationInlinePolicyField, string | null>>
  >({});
  const inlinePolicyDraftRef = useRef<{
    conversationKey: string;
    patch: ConversationInlinePatch;
    fields: Set<ConversationInlinePolicyField>;
  } | null>(null);
  const inlinePolicyMutation = useLatestDebouncedMutation<
    ConversationInlineMutationEntry,
    PromptCacheConversationBindingResponse
  >({
    resourceKey: conversationKey,
    mutate: (entry) =>
      updatePromptCacheConversationBinding(entry.conversationKey, {
        ...conversationBindingPayloadBase(binding),
        ...entry.patch,
      }),
    onSuccess: (nextBinding) => {
      confirmedBindingRef.current = nextBinding;
      inlinePolicyDraftRef.current = null;
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
      setInlinePolicyErrors({});
    },
    onError: (error, entry) => {
      bindingDraftDirtyRef.current = true;
      setInlinePolicyErrors((current) => ({
        ...current,
        ...Object.fromEntries(
          entry.fields.map((field) => [
            field,
            error instanceof Error ? error.message : String(error),
          ]),
        ),
      }));
    },
    onRevert: (confirmedBinding) => {
      if (!confirmedBinding) return;
      confirmedBindingRef.current = confirmedBinding;
      inlinePolicyDraftRef.current = null;
      setBinding(confirmedBinding);
      setBindingKind(confirmedBinding.bindingKind);
      applyBindingPolicyDraft(confirmedBinding, {
        setAllowSwitchUpstreamDraft,
        setFastModeDraft,
        setImageToolDraft,
        setCodexImagegenDraft,
        setAvailableModelsMode,
        setAvailableModelsDraft,
        setForwardProxyKeysDraft,
      });
      setBindingGroupName(confirmedBinding.groupName ?? bindingGroups[0] ?? "");
      setBindingAccountId(
        confirmedBinding.upstreamAccountId != null
          ? String(confirmedBinding.upstreamAccountId)
          : bindingAccounts[0]
            ? String(bindingAccounts[0].id)
            : "",
      );
      bindingDraftDirtyRef.current = false;
      setBindingRemoteConflict(null);
      setInlinePolicyErrors({});
    },
  });
  const inlinePolicySaving =
    inlinePolicyMutation.status === "pending" || inlinePolicyMutation.status === "saving";
  const inlinePolicyStatusByField = useMemo(() => {
    if (!inlinePolicySaving || !inlinePolicyDraftRef.current) return {};
    return Object.fromEntries(
      Array.from(inlinePolicyDraftRef.current.fields).map((field) => [
        field,
        inlinePolicyMutation.status,
      ]),
    ) as Partial<Record<ConversationInlinePolicyField, "pending" | "saving">>;
  }, [inlinePolicyMutation.status, inlinePolicySaving]);
  const retryInlinePolicy = useCallback(() => {
    setInlinePolicyErrors((current) => {
      const next = { ...current };
      for (const field of inlinePolicyDraftRef.current?.fields ?? []) delete next[field];
      return next;
    });
    inlinePolicyMutation.retry();
  }, [inlinePolicyMutation.retry]);
  const revertInlinePolicy = useCallback(() => {
    inlinePolicyMutation.revert();
  }, [inlinePolicyMutation.revert]);

  useEffect(() => {
    if (open) return;
    void inlinePolicyMutation.flush();
  }, [inlinePolicyMutation.flush, open]);
  const [bindingOwnerConfirmOpen, setBindingOwnerConfirmOpen] = useState(false);
  const [affinityResetConfirmOpen, setAffinityResetConfirmOpen] = useState(false);
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
  const [operationsRoutingModel, setOperationsRoutingModel] =
    useState<PromptCacheConversationRoutingModelFilter>("any");
  const [operationsRoutingModelFacets, setOperationsRoutingModelFacets] = useState<string[]>([]);
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
      setAffinityResetConfirmOpen(false);
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
    if (
      !response ||
      isSseUnavailable ||
      !open ||
      activeTab !== "operations" ||
      operationsRoutingModel !== "any"
    ) {
      return;
    }
    operationsRequestSeqRef.current += 1;
    operationsLoadControllerRef.current?.abort();
    const topicKey = `${operationsTopic.descriptorKey ?? operationsFilter}:${operationsRoutingModel}`;
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
    setOperationsRoutingModelFacets(response.routingModelFacets ?? []);
    setOperationsLoading(false);
    setOperationsError(null);
  }, [
    activeTab,
    isSseUnavailable,
    open,
    operationsFilter,
    operationsRoutingModel,
    operationsTopic.data,
    operationsTopic.descriptorKey,
  ]);

  const loadHistoryRecords = useCallback(
    async ({
      controller,
      requestSeq,
      silent,
      append,
      conversationKey,
    }: {
      controller: AbortController;
      requestSeq: number;
      silent: boolean;
      append: boolean;
      conversationKey: string;
    }) => {
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
        if (requestSeq !== requestSeqRef.current) return false;
        capturedHttpHead = await fetchHistoryPage(1, latestHttpHead.snapshotId);
        if (requestSeq !== requestSeqRef.current) return false;
        historySnapshotIdRef.current = capturedHttpHead.snapshotId;
        historyHttpSnapshotInitializedRef.current = true;
        page = capturedHttpHead.page;
        response = capturedHttpHead;
      } else {
        response = await fetchHistoryPage(page, append ? historySnapshotIdRef.current : undefined);
      }
      if (requestSeq !== requestSeqRef.current) return false;

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

      if (requestSeq !== requestSeqRef.current) return false;
      hasHydratedRef.current = true;
      const loadedStableKeys = new Set(loaded.map(invocationStableKey));
      setLiveRecords((current) =>
        current.filter((record) => !loadedStableKeys.has(invocationStableKey(record))),
      );
      setError(null);
      return capturedHttpHead != null && historyHasMoreRef.current;
    },
    [historyQueryForConversationKey],
  );

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
        const shouldQueueAppend = await loadHistoryRecords({
          controller,
          requestSeq,
          silent,
          append,
          conversationKey,
        });
        if (shouldQueueAppend) pendingLoadRef.current = { append: true, silent: true };
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
    [conversationKey, loadHistoryRecords, open],
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
          routingScope:
            operationsFilter === "routing" && operationsRoutingModel === "all" ? "all" : undefined,
          routingModel:
            operationsFilter === "routing" &&
            operationsRoutingModel !== "any" &&
            operationsRoutingModel !== "all"
              ? operationsRoutingModel
              : undefined,
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
        setOperationsRoutingModelFacets(response.routingModelFacets ?? []);
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
    [activeTab, conversationKey, open, operationsFilter, operationsRoutingModel],
  );

  useEffect(() => {
    if (
      bindingScopeRef.current.conversationKey !== conversationKey ||
      bindingScopeRef.current.open !== open
    ) {
      bindingScopeRef.current = { conversationKey, open };
      bindingHydratedRef.current = false;
      confirmedBindingRef.current = null;
    }
    if (inlinePolicyMutation.hasPending || inlinePolicyMutation.status === "error") return;
    inlinePolicyDraftRef.current = null;
    if (!open || !conversationKey) {
      bindingDraftDirtyRef.current = false;
      setBindingRemoteConflict(null);
      setBindingOwnerConfirmAllowsRemoteOverwrite(false);
      setInlinePolicyErrors({});
      return;
    }
    bindingDraftDirtyRef.current = false;
    setBindingRemoteConflict(null);
    setBindingOwnerConfirmAllowsRemoteOverwrite(false);
    setInlinePolicyErrors({});
  }, [conversationKey, inlinePolicyMutation.hasPending, inlinePolicyMutation.status, open]);

  const resetBindingHydrationState = useCallback(() => {
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
    setOperationsRoutingModel("any");
    setOperationsRoutingModelFacets([]);
  }, []);

  useEffect(() => {
    if (!open || !conversationKey) {
      resetBindingHydrationState();
      return;
    }

    if (activeTab !== "routing" && activeTab !== "settings") {
      return;
    }

    const controller = new AbortController();
    const hydrationSeq = bindingHydrationSeqRef.current + 1;
    bindingHydrationSeqRef.current = hydrationSeq;
    if (!bindingHydratedRef.current) {
      setBindingLoading(isSseUnavailable || bindingTopicLoadingRef.current);
    }
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
        if (
          !nextBinding ||
          hydrationSeq !== bindingHydrationSeqRef.current ||
          bindingDraftDirtyRef.current ||
          inlinePolicyMutation.hasPending
        )
          return;
        if (isOlderBindingSnapshot(nextBinding, confirmedBindingRef.current)) return;
        inlinePolicyMutation.reconcile(nextBinding);
        confirmedBindingRef.current = nextBinding;
        bindingHydratedRef.current = true;
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
  }, [
    activeTab,
    conversationKey,
    inlinePolicyMutation.hasPending,
    inlinePolicyMutation.reconcile,
    isSseUnavailable,
    open,
    resetBindingHydrationState,
  ]);

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
    if (
      !nextBinding ||
      isCachedFallbackPayload ||
      !open ||
      (activeTab !== "routing" && activeTab !== "settings")
    )
      return;
    bindingHydrationSeqRef.current += 1;
    if (bindingDraftDirtyRef.current) {
      setBindingRemoteConflict(nextBinding);
      return;
    }
    if (isOlderBindingSnapshot(nextBinding, confirmedBindingRef.current)) return;
    confirmedBindingRef.current = nextBinding;
    bindingHydratedRef.current = true;
    inlinePolicyMutation.reconcile(nextBinding);
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
  }, [
    activeTab,
    bindingAccounts,
    bindingGroups,
    bindingTopic.data,
    inlinePolicyMutation.reconcile,
    isSseUnavailable,
    open,
  ]);

  useEffect(() => {
    if (operationsFilter !== "routing") {
      setOperationsRoutingModel("any");
    }
  }, [operationsFilter]);

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
    if (operationsTopic.data && !isSseUnavailable && operationsRoutingModel === "any") {
      return;
    }
    operationsRequestSeqRef.current += 1;
    operationsLoadControllerRef.current?.abort();
    operationsTopicKeyRef.current = `${operationsTopic.descriptorKey ?? operationsFilter}:${operationsRoutingModel}`;
    operationEventsRef.current = [];
    operationsPageRef.current = 1;
    operationsTotalRef.current = 0;
    setOperationEvents([]);
    setOperationsTotal(0);
    setOperationsPage(1);
    setOperationsError(null);
    // The live topic represents the unfiltered event head. Scope/model filters
    // need a paged HTTP read even while SSE is healthy.
    if (isSseUnavailable || operationsRoutingModel !== "any") {
      void loadOperationEvents();
    }
  }, [
    activeTab,
    conversationKey,
    isSseUnavailable,
    loadOperationEvents,
    open,
    operationsFilter,
    operationsRoutingModel,
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
  const saveConversationInlinePolicy = useCallback(
    (field: ConversationInlinePolicyField, patch: ConversationInlinePatch) => {
      if (!conversationKey || !binding || bindingSaving) return;
      const existingDraft = inlinePolicyDraftRef.current;
      const nextPatch = mergeConversationInlinePatch(existingDraft?.patch ?? null, patch);
      const nextFields = new Set(existingDraft?.fields ?? []);
      nextFields.add(field);
      inlinePolicyDraftRef.current = { conversationKey, patch: nextPatch, fields: nextFields };
      bindingDraftDirtyRef.current = true;
      setInlinePolicyErrors((current) => ({ ...current, [field]: null }));
      setBindingError(null);
      const nextBinding = applyConversationInlinePatch(binding, nextPatch);
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
      inlinePolicyMutation.schedule({
        conversationKey,
        fields: Array.from(nextFields),
        patch: nextPatch,
      });
    },
    [binding, bindingSaving, conversationKey, inlinePolicyMutation.schedule],
  );
  const conversationEffectiveRoutingRule = useMemo(
    () => buildConversationEffectiveRoutingRule(binding),
    [binding],
  );
  const availableModelsEditor = useMemo(
    () => (
      <ConversationAvailableModelsEditor
        value={availableModelsDraft}
        disabled={bindingSaving}
        ariaLabel={t("live.conversations.drawer.policy.availableModels")}
        placeholder={t("live.conversations.drawer.policy.availableModelsPlaceholder")}
        isEmpty={availableModelsOverrideEmpty}
        requiredLabel={t("live.conversations.drawer.policy.availableModelsRequired")}
        applyLabel={t("live.conversations.drawer.policy.applyField")}
        applyDisabled={bindingSaving || availableModelsOverrideList.length === 0}
        onChange={(value) => {
          bindingDraftDirtyRef.current = true;
          setAvailableModelsMode("override");
          setAvailableModelsDraft(value);
        }}
        onApply={() =>
          void saveConversationInlinePolicy("availableModels", {
            availableModels: availableModelsOverrideList,
            availableModelsMode: binding?.availableModelsMode ?? "allowlist",
          })
        }
      />
    ),
    [
      availableModelsDraft,
      availableModelsOverrideEmpty,
      availableModelsOverrideList,
      binding,
      bindingSaving,
      saveConversationInlinePolicy,
      t,
    ],
  );
  const conversationRowValueOverrides = useMemo(() => {
    const rowOverrides = buildConversationRowValueOverrides(binding, t);
    rowOverrides.allowCutOut = {
      ...(rowOverrides.allowCutOut ?? {}),
      editor: (
        <ConversationPolicySelectEditor
          value={allowSwitchUpstreamDraft}
          disabled={bindingSaving}
          ariaLabel={t("live.conversations.drawer.policy.cutOut")}
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
          onChange={(value) => {
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
        <ConversationPolicySelectEditor
          value={fastModeDraft}
          disabled={bindingSaving}
          ariaLabel={t("live.conversations.drawer.policy.fastMode")}
          options={rewriteModeOptions}
          onChange={(value) => {
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
        <ConversationPolicySelectEditor
          value={imageToolDraft}
          disabled={bindingSaving}
          ariaLabel={t("live.conversations.drawer.policy.imageTool")}
          options={rewriteModeOptions}
          onChange={(value) => {
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
        <ConversationPolicySelectEditor
          value={codexImagegenDraft}
          disabled={bindingSaving}
          ariaLabel="Codex imagegen"
          options={rewriteModeOptions}
          onChange={(value) => {
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
      editor: availableModelsEditor,
    };
    return rowOverrides;
  }, [
    allowSwitchUpstreamDraft,
    availableModelsEditor,
    binding,
    fastModeDraft,
    imageToolDraft,
    codexImagegenDraft,
    bindingSaving,
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
  const routingStickyRoutes = binding?.stickyRoutes ?? [];
  const renderRoutingAccount = (route: PromptCacheConversationStickyRoute) => {
    const routeAccountId =
      typeof route.upstreamAccountId === "number" &&
      Number.isSafeInteger(route.upstreamAccountId) &&
      route.upstreamAccountId > 0
        ? route.upstreamAccountId
        : null;
    const routeAccountLabel = route.upstreamAccountName ?? `#${route.upstreamAccountId}`;
    const canOpenRouteAccount = routeAccountId != null && onOpenUpstreamAccount != null;

    return canOpenRouteAccount ? (
      <button
        type="button"
        className="max-w-full break-all text-left font-medium transition hover:text-primary hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
        aria-label={t("live.conversations.drawer.routing.openAccount", {
          account: routeAccountLabel,
        })}
        title={t("live.conversations.drawer.routing.openAccount", {
          account: routeAccountLabel,
        })}
        onClick={() => onOpenUpstreamAccount(routeAccountId, routeAccountLabel)}
      >
        {routeAccountLabel}
      </button>
    ) : (
      <span className="break-all">{routeAccountLabel}</span>
    );
  };
  const routingPanel = (
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
            disabled={bindingLoading || bindingSaving}
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
              disabled={bindingLoading || bindingSaving}
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
              disabled={bindingLoading || bindingSaving}
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
                    confirmedBindingRef.current = latest;
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
      <section
        className="rounded-xl border border-base-content/10 bg-base-100/80 p-4"
        data-testid="prompt-cache-current-routing"
      >
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="space-y-1">
            <p className="font-semibold text-base-content">
              {t("live.conversations.drawer.routing.currentTitle")}
            </p>
            <p className="text-xs text-base-content/68">
              {t("live.conversations.drawer.routing.currentDescription")}
            </p>
          </div>
          <Button
            type="button"
            size="sm"
            variant="secondary"
            disabled={bindingLoading || bindingSaving}
            onClick={() => setAffinityResetConfirmOpen(true)}
          >
            {t("live.conversations.drawer.routing.reset")}
          </Button>
        </div>
        {routingStickyRoutes.length ? (
          <>
            <div
              className="mt-3 hidden overflow-x-auto md:block"
              data-testid="prompt-cache-current-routing-table"
            >
              <table className="w-full min-w-[38rem] text-left text-xs">
                <thead className="text-base-content/55">
                  <tr className="border-b border-base-content/10">
                    <th className="px-2 py-2 font-medium">
                      {t("live.conversations.drawer.routing.model")}
                    </th>
                    <th className="px-2 py-2 font-medium">
                      {t("live.conversations.drawer.routing.account")}
                    </th>
                    <th className="px-2 py-2 font-medium">
                      {t("live.conversations.drawer.routing.created")}
                    </th>
                    <th className="px-2 py-2 font-medium">
                      {t("live.conversations.drawer.routing.updated")}
                    </th>
                    <th className="px-2 py-2 font-medium">
                      {t("live.conversations.drawer.routing.lastUsed")}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {routingStickyRoutes.map((route) => (
                    <tr
                      key={`${route.modelKey ?? "all"}-${route.upstreamAccountId}`}
                      className="border-b border-base-content/5 last:border-0"
                    >
                      <td className="px-2 py-2 font-mono text-base-content">
                        {route.modelKey ?? t("live.conversations.drawer.routing.allModels")}
                      </td>
                      <td className="px-2 py-2 text-base-content/80">
                        {renderRoutingAccount(route)}
                      </td>
                      <td className="whitespace-nowrap px-2 py-2 text-base-content/62">
                        {formatConversationOperationOccurredAt(route.createdAt)}
                      </td>
                      <td className="whitespace-nowrap px-2 py-2 text-base-content/62">
                        {formatConversationOperationOccurredAt(route.updatedAt)}
                      </td>
                      <td className="whitespace-nowrap px-2 py-2 text-base-content/62">
                        {formatConversationOperationOccurredAt(route.lastSeenAt)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <dl
              className="mt-3 divide-y divide-base-content/10 md:hidden"
              data-testid="prompt-cache-current-routing-mobile"
            >
              {routingStickyRoutes.map((route) => (
                <div
                  key={`${route.modelKey ?? "all"}-${route.upstreamAccountId}`}
                  className="grid grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)] gap-x-3 gap-y-1 py-3 first:pt-0 last:pb-0"
                >
                  <dt className="text-xs text-base-content/55">
                    {t("live.conversations.drawer.routing.model")}
                  </dt>
                  <dd className="min-w-0 break-all font-mono text-xs text-base-content">
                    {route.modelKey ?? t("live.conversations.drawer.routing.allModels")}
                  </dd>
                  <dt className="text-xs text-base-content/55">
                    {t("live.conversations.drawer.routing.account")}
                  </dt>
                  <dd className="min-w-0 text-xs text-base-content/80">
                    {renderRoutingAccount(route)}
                  </dd>
                  <dt className="text-xs text-base-content/55">
                    {t("live.conversations.drawer.routing.created")}
                  </dt>
                  <dd className="min-w-0 text-xs text-base-content/62">
                    {formatConversationOperationOccurredAt(route.createdAt)}
                  </dd>
                  <dt className="text-xs text-base-content/55">
                    {t("live.conversations.drawer.routing.updated")}
                  </dt>
                  <dd className="min-w-0 text-xs text-base-content/62">
                    {formatConversationOperationOccurredAt(route.updatedAt)}
                  </dd>
                  <dt className="text-xs text-base-content/55">
                    {t("live.conversations.drawer.routing.lastUsed")}
                  </dt>
                  <dd className="min-w-0 text-xs text-base-content/62">
                    {formatConversationOperationOccurredAt(route.lastSeenAt)}
                  </dd>
                </div>
              ))}
            </dl>
          </>
        ) : (
          <p className="mt-3 text-xs text-base-content/60">
            {t("live.conversations.drawer.routing.empty")}
          </p>
        )}
      </section>
    </div>
  );
  const settingsPanel = (
    <div className="space-y-4 text-sm">
      {bindingRemoteConflict ? (
        <Alert variant="warning">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span>{t("live.conversations.drawer.binding.remoteConflict")}</span>
            <Button
              type="button"
              size="sm"
              variant="secondary"
              onClick={() => handleSelectTab("routing")}
            >
              {t("live.conversations.drawer.tabs.routing")}
            </Button>
          </div>
        </Alert>
      ) : null}
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
            saveStatusByField: inlinePolicyStatusByField,
            errorByField: inlinePolicyErrors,
            onRetry: retryInlinePolicy,
            onRevert: revertInlinePolicy,
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
                  availableModelsMode: payload.availableModelsMode ?? null,
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
            busy: false,
            saving:
              inlinePolicySaving &&
              inlinePolicyDraftRef.current?.fields.has("proxyBindings") === true,
            error: inlinePolicyErrors.proxyBindings,
            disabled: bindingLoading || bindingSaving,
            onRetry: retryInlinePolicy,
            onRevert: revertInlinePolicy,
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
                    disabled={bindingLoading || bindingSaving}
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
                    disabled={bindingSaving || forwardProxyKeysDraft.length === 0}
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
                        <Chip
                          size="default"
                          tone="secondary"
                          key={key}
                          className="min-w-0 gap-1.5 px-2 py-1 text-xs"
                        >
                          <span className="max-w-48 truncate">
                            {node ? conversationForwardProxyLabel(node) : key}
                          </span>
                          <button
                            type="button"
                            className="rounded-full px-1 text-base-content/55 hover:bg-base-200 hover:text-base-content"
                            disabled={bindingSaving}
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
                        </Chip>
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
            overrideRetry: t("settings.retrySave"),
            overrideRevert: t("settings.revertSave"),
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
              onClick={() => {
                setOperationsFilter(option.value);
                if (option.value !== "routing") setOperationsRoutingModel("any");
              }}
            >
              {t(option.labelKey)}
            </Button>
          ))}
        </div>
        {operationsFilter === "routing" ? (
          <div className="mt-3 max-w-full sm:max-w-sm">
            <SelectField
              value={operationsRoutingModel}
              size="sm"
              aria-label={t("live.conversations.drawer.operations.modelFilter.label")}
              options={[
                {
                  value: "any",
                  label: t("live.conversations.drawer.operations.modelFilter.any"),
                },
                {
                  value: "all",
                  label: t("live.conversations.drawer.operations.modelFilter.allModels"),
                },
                ...operationsRoutingModelFacets.map((modelKey) => ({
                  value: modelKey,
                  label: modelKey,
                })),
              ]}
              onValueChange={setOperationsRoutingModel}
            />
          </div>
        ) : null}
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
        <ConversationOperationEventCard key={event.id} event={event} t={t} />
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
      if (inlinePolicyMutation.hasPending) await inlinePolicyMutation.flush();
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
        inlinePolicyMutation.reconcile(nextBinding);
        confirmedBindingRef.current = nextBinding;
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
      inlinePolicyMutation.hasPending,
      inlinePolicyMutation.flush,
      inlinePolicyMutation.reconcile,
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

  const resetAffinity = useCallback(async () => {
    if (!conversationKey || bindingSaving) return;
    if (inlinePolicyMutation.hasPending) await inlinePolicyMutation.flush();
    setBindingSaving(true);
    setBindingError(null);
    try {
      const nextBinding = await resetPromptCacheConversationAffinity(conversationKey);
      inlinePolicyMutation.reconcile(nextBinding);
      confirmedBindingRef.current = nextBinding;
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
      setAffinityResetConfirmOpen(false);
    } catch (err) {
      setBindingError(err instanceof Error ? err.message : String(err));
    } finally {
      setBindingSaving(false);
    }
  }, [
    bindingAccounts,
    bindingGroups,
    bindingSaving,
    conversationKey,
    inlinePolicyMutation.hasPending,
    inlinePolicyMutation.flush,
    inlinePolicyMutation.reconcile,
  ]);

  return (
    <>
      <AccountDetailDrawerShell
        open={open}
        presentation={presentation}
        labelledBy={titleId}
        closeLabel={t("live.conversations.drawer.close")}
        onClose={onClose}
        closeDisabled={bindingOwnerConfirmOpen || affinityResetConfirmOpen}
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
              className="grid w-full grid-cols-5 gap-1 rounded-xl p-1 sm:inline-flex sm:w-fit"
            >
              <SegmentedControlItem
                active={activeTab === "overview"}
                className="min-w-0 px-0 text-[11px] leading-none sm:px-3.5 sm:text-sm"
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
                className="min-w-0 px-0 text-[11px] leading-none sm:px-3.5 sm:text-sm"
                role="tab"
                aria-selected={activeTab === "calls"}
                aria-controls={`${titleId}-panel-calls`}
                id={`${titleId}-tab-calls`}
                onClick={() => handleSelectTab("calls")}
              >
                {t("live.conversations.drawer.tabs.calls")}
              </SegmentedControlItem>
              <SegmentedControlItem
                active={activeTab === "routing"}
                className="min-w-0 px-0 text-[11px] leading-none sm:px-3.5 sm:text-sm"
                role="tab"
                aria-selected={activeTab === "routing"}
                aria-controls={`${titleId}-panel-routing`}
                id={`${titleId}-tab-routing`}
                onClick={() => handleSelectTab("routing")}
              >
                {t("live.conversations.drawer.tabs.routing")}
              </SegmentedControlItem>
              <SegmentedControlItem
                active={activeTab === "settings"}
                className="min-w-0 px-0 text-[11px] leading-none sm:px-3.5 sm:text-sm"
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
                className="min-w-0 px-0 text-[11px] leading-none sm:px-3.5 sm:text-sm"
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
        {activeTab === "routing" ? (
          <div
            id={`${titleId}-panel-routing`}
            role="tabpanel"
            aria-labelledby={`${titleId}-tab-routing`}
          >
            {routingPanel}
          </div>
        ) : null}
        {activeTab === "settings" ? (
          <div
            id={`${titleId}-panel-settings`}
            role="tabpanel"
            aria-labelledby={`${titleId}-tab-settings`}
          >
            {settingsPanel}
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
      <Dialog
        open={open && affinityResetConfirmOpen}
        onOpenChange={(nextOpen) => {
          if (!bindingSaving) setAffinityResetConfirmOpen(nextOpen);
        }}
      >
        <DialogContent
          role="alertdialog"
          container={drawerBodyElement}
          className="overflow-hidden p-0"
        >
          <div
            data-testid="prompt-cache-affinity-reset-dialog-header"
            className="px-5 pb-4 pt-5 desktop:px-6 desktop:pb-5 desktop:pt-6"
          >
            <DialogHeader className="gap-2">
              <DialogTitle>{t("live.conversations.drawer.routing.resetConfirm.title")}</DialogTitle>
              <DialogDescription>
                {t("live.conversations.drawer.routing.resetConfirm.description")}
              </DialogDescription>
            </DialogHeader>
          </div>
          <DialogFooter
            data-testid="prompt-cache-affinity-reset-dialog-footer"
            className="border-t border-base-300/80 bg-base-100/94 px-5 pb-[max(env(safe-area-inset-bottom),1rem)] pt-4 backdrop-blur desktop:px-6 desktop:py-4"
          >
            <Button
              type="button"
              variant="ghost"
              disabled={bindingSaving}
              onClick={() => setAffinityResetConfirmOpen(false)}
            >
              {t("live.conversations.drawer.routing.resetConfirm.cancel")}
            </Button>
            <Button
              type="button"
              variant="destructive"
              disabled={bindingSaving}
              onClick={() => void resetAffinity()}
            >
              {bindingSaving
                ? t("live.conversations.drawer.binding.saving")
                : t("live.conversations.drawer.routing.resetConfirm.confirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

function ConversationPreviewActions({
  promptCacheKey,
  isExpanded,
  labels,
  onToggle,
  onHistory,
}: {
  promptCacheKey: string;
  isExpanded: boolean;
  labels: { expandAction: string; collapseAction: string; historyAction: string };
  onToggle: (promptCacheKey: string) => void;
  onHistory: (promptCacheKey: string) => void;
}) {
  return (
    <div className="flex items-center gap-1">
      <button
        type="button"
        className="inline-flex h-8 w-8 items-center justify-center rounded-full border border-base-300/70 bg-base-100/80 text-base-content/72 transition hover:border-primary/40 hover:text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
        aria-label={isExpanded ? labels.collapseAction : labels.expandAction}
        aria-expanded={isExpanded}
        onClick={() => onToggle(promptCacheKey)}
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
        aria-label={labels.historyAction}
        onClick={() => onHistory(promptCacheKey)}
      >
        <AppIcon name="account-details-outline" className="h-4 w-4" aria-hidden />
      </button>
    </div>
  );
}

function ConversationTimeDetails({
  conversation,
  labels,
  dateFormatter,
}: {
  conversation: PromptCacheConversation;
  labels: { createdAtShort: string; lastActivityAtShort: string };
  dateFormatter: Intl.DateTimeFormat;
}) {
  return (
    <dl className="space-y-1 text-xs">
      <div className="flex items-center justify-between gap-3">
        <dt className="text-base-content/60">{labels.createdAtShort}</dt>
        <dd className="text-right">{formatDateLabel(conversation.createdAt, dateFormatter)}</dd>
      </div>
      <div className="flex items-center justify-between gap-3">
        <dt className="text-base-content/60">{labels.lastActivityAtShort}</dt>
        <dd className="text-right">
          {formatDateLabel(conversation.lastActivityAt, dateFormatter)}
        </dd>
      </div>
    </dl>
  );
}

function ConversationDesktopTimeDetails({
  conversation,
  labels,
  dateFormatter,
}: {
  conversation: PromptCacheConversation;
  labels: { createdAtShort: string; lastActivityAtShort: string };
  dateFormatter: Intl.DateTimeFormat;
}) {
  return (
    <div className="space-y-1.5 text-[11px]">
      <div className="grid grid-cols-[2rem_minmax(0,1fr)] items-center gap-x-2">
        <span className="text-base-content/60">{labels.createdAtShort}</span>
        <span className="whitespace-nowrap font-medium tabular-nums">
          {formatDateLabel(conversation.createdAt, dateFormatter)}
        </span>
      </div>
      <div className="grid grid-cols-[2rem_minmax(0,1fr)] items-center gap-x-2">
        <span className="text-base-content/60">{labels.lastActivityAtShort}</span>
        <span className="whitespace-nowrap font-medium tabular-nums">
          {formatDateLabel(conversation.lastActivityAt, dateFormatter)}
        </span>
      </div>
    </div>
  );
}

function ConversationDesktopExpandedPreview({
  conversation,
  isExpanded,
  labels,
  onOpenUpstreamAccount,
}: {
  conversation: PromptCacheConversation;
  isExpanded: boolean;
  labels: { empty: string };
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}) {
  if (!isExpanded) return null;
  return (
    <tr className="bg-base-200/20">
      <td colSpan={5} className="px-3 pb-4 pt-0">
        <div className="border-t border-base-300/60 pt-3">
          <PromptCacheConversationInvocationTable
            records={conversation.recentInvocations.map(buildInvocationFromPromptCachePreview)}
            isLoading={false}
            emptyLabel={labels.empty}
            onOpenUpstreamAccount={onOpenUpstreamAccount}
          />
        </div>
      </td>
    </tr>
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
  const openAccountDrawerFromHistory = useCallback(
    (accountId: number, accountLabel: string) => {
      setHistoryDrawerPromptCacheKey(null);
      onOpenUpstreamAccount?.(accountId, accountLabel);
    },
    [onOpenUpstreamAccount],
  );
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
                    <ConversationPreviewActions
                      promptCacheKey={conversation.promptCacheKey}
                      isExpanded={isExpanded}
                      labels={previewLabels}
                      onToggle={togglePromptCachePreview}
                      onHistory={openHistoryDrawer}
                    />
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
                  <ConversationTimeDetails
                    conversation={conversation}
                    labels={totalLabels}
                    dateFormatter={dateFormatter}
                  />
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
                        <ConversationPreviewActions
                          promptCacheKey={conversation.promptCacheKey}
                          isExpanded={isExpanded}
                          labels={previewLabels}
                          onToggle={togglePromptCachePreview}
                          onHistory={openHistoryDrawer}
                        />
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
                      <ConversationDesktopTimeDetails
                        conversation={conversation}
                        labels={totalLabels}
                        dateFormatter={dateFormatter}
                      />
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
                  <ConversationDesktopExpandedPreview
                    conversation={conversation}
                    isExpanded={isExpanded}
                    labels={previewLabels}
                    onOpenUpstreamAccount={onOpenUpstreamAccount}
                  />
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
        onOpenUpstreamAccount={openAccountDrawerFromHistory}
        historyQueryForConversationKey={historyQueryForConversationKey}
      />
    </div>
  );
}
