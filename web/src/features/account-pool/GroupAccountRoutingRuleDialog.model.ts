import type {
  AvailableModelsMode,
  CodexImagegenRewriteMode,
  EffectiveRoutingTimeoutFieldSources,
  GroupAccountRoutingRule,
  ImageToolRewriteMode,
  PoolRoutingTimeoutSettings,
  RequestCompressionAlgorithm,
  TagFastModeRewriteMode,
  TagPriorityTier,
  UpdateGroupAccountRoutingRulePayload,
} from "../../lib/api";
import {
  apiConcurrencyLimitToSliderValue,
  sliderConcurrencyLimitToApiValue,
} from "../../lib/concurrencyLimit";
import {
  buildRoutingTimeoutOverrideDraft,
  buildRoutingTimeoutOverrideDraftForSource,
  buildRoutingTimeoutOverrideEnabledState,
  buildRoutingTimeoutOverrideEnabledStateForSource,
  diffRoutingTimeoutOverrideDraftWithEnabledState,
  parseRoutingTimeoutOverrideDraftWithEnabledState,
  type RoutingTimeoutFieldKey,
  type RoutingTimeoutOverrideDraft,
  type RoutingTimeoutOverrideEnabledState,
} from "../../lib/poolRoutingTimeouts";
import { REQUEST_COMPRESSION_INHERIT_VALUE } from "../../lib/requestCompression";
import {
  resolveStatusChangeReasons,
  STATUS_CHANGE_REASON_CODES,
  type StatusChangeReasonCode,
} from "../../lib/upstreamAccountStatusChangeReasons";

export const CODEX_IMAGEGEN_INHERIT_VALUE = "__inherit__";
export const AVAILABLE_MODE_INHERIT_VALUE = "__inherit_available_models_mode__";

export type GroupAccountRoutingRuleDraft = {
  allowCutOut: boolean;
  allowCutIn: boolean;
  priorityTier: TagPriorityTier;
  fastModeRewriteMode: TagFastModeRewriteMode;
  imageToolRewriteMode: ImageToolRewriteMode;
  codexImagegenRewriteMode: CodexImagegenRewriteMode | typeof CODEX_IMAGEGEN_INHERIT_VALUE;
  requestCompressionAlgorithm:
    | RequestCompressionAlgorithm
    | typeof REQUEST_COMPRESSION_INHERIT_VALUE;
  concurrencyLimit: number;
  upstream429RetryEnabled: boolean;
  upstream429MaxRetries: number;
  availableModels: string[];
  availableModelsMode: AvailableModelsMode | typeof AVAILABLE_MODE_INHERIT_VALUE;
  availableModelInput: string;
  availableModelsTouched: boolean;
  statusChangeReasons: Record<StatusChangeReasonCode, boolean>;
  timeoutOverrides: RoutingTimeoutOverrideDraft;
  timeoutOverrideEnabledFields: RoutingTimeoutOverrideEnabledState;
};

export interface GroupAccountRoutingRuleLabels {
  allowCutOut: string;
  allowCutIn: string;
  forbidCutOut?: string;
  forbidCutIn?: string;
  priorityTier: string;
  priorityPrimary: string;
  priorityNormal: string;
  priorityFallback: string;
  priorityNoNew?: string;
  fastModeRewriteMode: string;
  fastModeKeepOriginal: string;
  fastModeFillMissing: string;
  fastModeForceAdd: string;
  fastModeForceRemove: string;
  imageToolRewriteMode: string;
  imageToolKeepOriginal: string;
  imageToolFillMissing: string;
  imageToolForceAdd: string;
  imageToolForceRemove: string;
  imageToolRewriteHint?: string;
  codexImagegenRewriteMode?: string;
  codexImagegenRewriteHint?: string;
  requestCompressionAlgorithm: string;
  requestCompressionFollow: string;
  requestCompressionIdentity: string;
  requestCompressionGzip: string;
  requestCompressionDeflate: string;
  requestCompressionZstd: string;
  requestCompressionInherited: string;
  requestCompressionHint?: string;
  requestCompressionMixedGroupHint?: string;
  concurrencyLimit: string;
  concurrencyHint: string;
  currentValue: string;
  unlimited: string;
  upstream429Retry: string;
  upstream429RetryHint: string;
  upstream429RetryToggle: string;
  upstream429RetryCount: string;
  upstream429RetryCountOnce: string;
  upstream429RetryCountMany: (count: number) => string;
  availableModels: string;
  availableModelsMode?: string;
  availableModelsAllowlist?: string;
  availableModelsDenylist?: string;
  availableModelsModeInherited?: string;
  availableModelsHint: string;
  availableModelsSearchPlaceholder: string;
  availableModelsEmpty: string;
  availableModelsAll: string;
  availableModelsCustomLabel: (value: string) => string;
  availableModelsAddCustom: string;
  availableModelsInherited: string;
  availableModelsRemove: string;
  statusChangeReasonSectionTitle?: string;
  statusChangeReasonSectionHint?: string;
  statusChangeReasonLabel?: (reason: StatusChangeReasonCode) => string;
  statusChangeReasonToggleEnabled?: string;
  statusChangeReasonToggleDisabled?: string;
  timeoutSectionTitle: string;
  timeoutSectionHint?: string;
  timeoutResponsesFirstByte: string;
  timeoutCompactFirstByte: string;
  timeoutImageFirstByte: string;
  timeoutResponsesStream: string;
  timeoutCompactStream: string;
  timeoutInheritedValue: string;
  timeoutOverrideValue: string;
  timeoutClearField: string;
  timeoutInheritField: string;
  timeoutSourceGlobal?: string;
  timeoutSourceGroup?: string;
  timeoutSourceAccount?: string;
  timeoutSourceConversation?: string;
  cancel: string;
  validation: string;
}

export type GroupAccountRoutingRuleBuildOptions = {
  changedFieldsOnly?: boolean;
  effectiveTimeouts?: PoolRoutingTimeoutSettings | null;
  timeoutFieldSources?: EffectiveRoutingTimeoutFieldSources | null;
  timeoutOverrideSource?: "group" | "account";
};

export type GroupAccountRoutingRulePayloadOptions = GroupAccountRoutingRuleBuildOptions & {
  baseRule?: GroupAccountRoutingRule | null;
  timeoutFieldLabels?: Record<RoutingTimeoutFieldKey, string>;
};

export function normalizeRetryCount(value?: number | null): number {
  if (!Number.isFinite(value ?? NaN)) return 0;
  return Math.max(0, Math.min(5, Math.trunc(value ?? 0)));
}

export function normalizeModelIds(values: string[]): string[] {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const value of values) {
    const trimmed = value.trim();
    if (!trimmed || seen.has(trimmed)) continue;
    seen.add(trimmed);
    normalized.push(trimmed);
  }
  return normalized;
}

function buildStatusChangeReasonPayload(
  statusChangeReasons: Record<StatusChangeReasonCode, boolean>,
  options?: {
    changedFieldsOnly?: boolean;
    baseRule?: GroupAccountRoutingRule | null;
  },
): UpdateGroupAccountRoutingRulePayload["statusChangeReasons"] | undefined {
  if (!options?.changedFieldsOnly) return { ...statusChangeReasons };
  const baseReasons = resolveStatusChangeReasons(options.baseRule?.statusChangeReasons);
  const patch: Partial<Record<StatusChangeReasonCode, boolean | null>> = {};
  for (const reason of STATUS_CHANGE_REASON_CODES) {
    if (statusChangeReasons[reason] !== baseReasons[reason]) {
      patch[reason] = statusChangeReasons[reason];
    }
  }
  return Object.keys(patch).length > 0 ? patch : undefined;
}

export function buildDraft(
  rule?: GroupAccountRoutingRule | null,
  options?: GroupAccountRoutingRuleBuildOptions,
): GroupAccountRoutingRuleDraft {
  const effectiveTimeouts = options?.effectiveTimeouts;
  const timeoutFieldSources = options?.timeoutFieldSources;
  const timeoutOverrideSource = options?.timeoutOverrideSource ?? "group";
  const timeoutOverrides = options?.changedFieldsOnly
    ? buildRoutingTimeoutOverrideDraftForSource(
        effectiveTimeouts,
        timeoutFieldSources,
        timeoutOverrideSource,
      )
    : buildRoutingTimeoutOverrideDraft(rule?.timeouts);
  const timeoutOverrideEnabledFields = options?.changedFieldsOnly
    ? buildRoutingTimeoutOverrideEnabledStateForSource(timeoutFieldSources, timeoutOverrideSource)
    : buildRoutingTimeoutOverrideEnabledState(timeoutOverrides);
  return {
    allowCutOut: rule?.allowCutOut ?? true,
    allowCutIn: rule?.allowCutIn ?? true,
    priorityTier: rule?.priorityTier ?? "normal",
    fastModeRewriteMode: rule?.fastModeRewriteMode ?? "keep_original",
    imageToolRewriteMode: rule?.imageToolRewriteMode ?? "keep_original",
    codexImagegenRewriteMode: rule?.codexImagegenRewriteMode ?? CODEX_IMAGEGEN_INHERIT_VALUE,
    requestCompressionAlgorithm: options?.changedFieldsOnly
      ? (rule?.requestCompressionAlgorithm ?? REQUEST_COMPRESSION_INHERIT_VALUE)
      : (rule?.requestCompressionAlgorithm ?? "identity"),
    concurrencyLimit: apiConcurrencyLimitToSliderValue(rule?.concurrencyLimit),
    upstream429RetryEnabled: rule?.upstream429RetryEnabled === true,
    upstream429MaxRetries: normalizeRetryCount(rule?.upstream429MaxRetries),
    availableModels: normalizeModelIds(rule?.availableModels ?? []),
    availableModelsMode:
      rule?.availableModelsMode ??
      (rule?.availableModelsDefined === true ? "allowlist" : AVAILABLE_MODE_INHERIT_VALUE),
    availableModelInput: "",
    availableModelsTouched: false,
    statusChangeReasons: resolveStatusChangeReasons(rule?.statusChangeReasons),
    timeoutOverrides,
    timeoutOverrideEnabledFields,
  };
}

export function buildDraftResetKey(rule?: GroupAccountRoutingRule | null): string {
  return JSON.stringify(rule ?? null);
}

export function buildPayload(
  draft: GroupAccountRoutingRuleDraft,
  options?: GroupAccountRoutingRulePayloadOptions,
): UpdateGroupAccountRoutingRulePayload | null {
  const timeoutLabels = options?.timeoutFieldLabels;
  if (!timeoutLabels) return null;
  const parsedTimeouts = parseRoutingTimeoutOverrideDraftWithEnabledState(
    draft.timeoutOverrides,
    draft.timeoutOverrideEnabledFields,
    timeoutLabels,
  );
  if (!parsedTimeouts.ok) return null;
  const requestCompressionAlgorithm =
    draft.requestCompressionAlgorithm === REQUEST_COMPRESSION_INHERIT_VALUE
      ? null
      : draft.requestCompressionAlgorithm;
  const codexImagegenRewriteMode =
    draft.codexImagegenRewriteMode === CODEX_IMAGEGEN_INHERIT_VALUE
      ? null
      : draft.codexImagegenRewriteMode;
  const availableModelsMode =
    draft.availableModelsMode === AVAILABLE_MODE_INHERIT_VALUE ? null : draft.availableModelsMode;
  const payload: UpdateGroupAccountRoutingRulePayload = {
    allowCutOut: draft.allowCutOut,
    allowCutIn: draft.allowCutIn,
    priorityTier: draft.priorityTier,
    fastModeRewriteMode: draft.fastModeRewriteMode,
    imageToolRewriteMode: draft.imageToolRewriteMode,
    codexImagegenRewriteMode,
    ...(requestCompressionAlgorithm == null ? {} : { requestCompressionAlgorithm }),
    concurrencyLimit: sliderConcurrencyLimitToApiValue(draft.concurrencyLimit),
    upstream429RetryEnabled: draft.upstream429RetryEnabled,
    upstream429MaxRetries: draft.upstream429RetryEnabled
      ? Math.max(1, normalizeRetryCount(draft.upstream429MaxRetries) || 1)
      : 0,
    availableModels: normalizeModelIds(draft.availableModels),
    availableModelsMode,
    statusChangeReasons: buildStatusChangeReasonPayload(draft.statusChangeReasons),
    timeouts: parsedTimeouts.patch,
  };

  if (options?.changedFieldsOnly && options.baseRule) {
    return buildChangedFieldsPayload(draft, payload, options, timeoutLabels);
  }

  if (
    !draft.availableModelsTouched &&
    payload.availableModels?.length === 0 &&
    options?.baseRule?.availableModelsDefined !== true
  ) {
    delete payload.availableModels;
  }
  return payload;
}

function buildChangedFieldsPayload(
  draft: GroupAccountRoutingRuleDraft,
  payload: UpdateGroupAccountRoutingRulePayload,
  options: GroupAccountRoutingRulePayloadOptions,
  timeoutLabels: Record<RoutingTimeoutFieldKey, string>,
): UpdateGroupAccountRoutingRulePayload | null {
  const base = options.baseRule;
  if (!base) return payload;
  const changedPayload: UpdateGroupAccountRoutingRulePayload = {};
  if (draft.allowCutOut !== (base.allowCutOut ?? true))
    changedPayload.allowCutOut = payload.allowCutOut;
  if (draft.allowCutIn !== (base.allowCutIn ?? true))
    changedPayload.allowCutIn = payload.allowCutIn;
  if (draft.priorityTier !== (base.priorityTier ?? "normal")) {
    changedPayload.priorityTier = payload.priorityTier;
  }
  if (draft.fastModeRewriteMode !== (base.fastModeRewriteMode ?? "keep_original")) {
    changedPayload.fastModeRewriteMode = payload.fastModeRewriteMode;
  }
  if (draft.imageToolRewriteMode !== (base.imageToolRewriteMode ?? "keep_original")) {
    changedPayload.imageToolRewriteMode = payload.imageToolRewriteMode;
  }
  if (
    draft.codexImagegenRewriteMode !==
    (base.codexImagegenRewriteMode ?? CODEX_IMAGEGEN_INHERIT_VALUE)
  ) {
    changedPayload.codexImagegenRewriteMode = payload.codexImagegenRewriteMode;
  }
  if (
    draft.requestCompressionAlgorithm !==
    (base.requestCompressionAlgorithm ?? REQUEST_COMPRESSION_INHERIT_VALUE)
  ) {
    changedPayload.requestCompressionAlgorithm =
      draft.requestCompressionAlgorithm === REQUEST_COMPRESSION_INHERIT_VALUE
        ? null
        : draft.requestCompressionAlgorithm;
  }
  if (draft.concurrencyLimit !== apiConcurrencyLimitToSliderValue(base.concurrencyLimit ?? 0)) {
    changedPayload.concurrencyLimit = payload.concurrencyLimit;
  }
  if (
    draft.upstream429RetryEnabled !== (base.upstream429RetryEnabled ?? false) ||
    draft.upstream429MaxRetries !== normalizeRetryCount(base.upstream429MaxRetries)
  ) {
    changedPayload.upstream429RetryEnabled = payload.upstream429RetryEnabled;
    changedPayload.upstream429MaxRetries = payload.upstream429MaxRetries;
  }
  if (draft.availableModelsMode !== (base.availableModelsMode ?? AVAILABLE_MODE_INHERIT_VALUE)) {
    changedPayload.availableModelsMode = payload.availableModelsMode;
  }
  if (
    draft.availableModelsTouched ||
    JSON.stringify(payload.availableModels ?? []) !==
      JSON.stringify(normalizeModelIds(base.availableModels ?? []))
  ) {
    changedPayload.availableModels = payload.availableModels;
  }
  const statusChangeReasonDiff = buildStatusChangeReasonPayload(draft.statusChangeReasons, {
    changedFieldsOnly: true,
    baseRule: base,
  });
  if (statusChangeReasonDiff) changedPayload.statusChangeReasons = statusChangeReasonDiff;

  const baseTimeoutDraft = buildDraft(base, {
    changedFieldsOnly: true,
    effectiveTimeouts: options.effectiveTimeouts,
    timeoutFieldSources: options.timeoutFieldSources,
    timeoutOverrideSource: options.timeoutOverrideSource,
  });
  const timeoutDiff = diffRoutingTimeoutOverrideDraftWithEnabledState(
    baseTimeoutDraft.timeoutOverrides,
    baseTimeoutDraft.timeoutOverrideEnabledFields,
    draft.timeoutOverrides,
    draft.timeoutOverrideEnabledFields,
    timeoutLabels,
  );
  if (!timeoutDiff.ok) return null;
  if (timeoutDiff.changed) changedPayload.timeouts = timeoutDiff.patch;
  return changedPayload;
}
