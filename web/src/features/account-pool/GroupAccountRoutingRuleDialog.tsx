import { useEffect, useMemo, useRef, useState } from "react";
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
import { Input } from "../../components/ui/input";
import { SelectField } from "../../components/ui/select-field";
import { Switch } from "../../components/ui/switch";
import type {
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
import {
  REQUEST_COMPRESSION_INHERIT_VALUE,
  requestCompressionAlgorithmLabel,
} from "../../lib/requestCompression";
import {
  resolveStatusChangeReasons,
  STATUS_CHANGE_REASON_CODES,
  type StatusChangeReasonCode,
} from "../../lib/upstreamAccountStatusChangeReasons";
import { AppIcon } from "../shared/AppIcon";
import { ConcurrencyLimitSlider } from "./ConcurrencyLimitSlider";
import {
  MultiSelectFilterCombobox,
  type MultiSelectFilterOption,
} from "./MultiSelectFilterCombobox";
import { PolicyInlineOptionGroup } from "./PolicyInlineOptionGroup";
import { RoutingTimeoutOverridesEditor } from "./RoutingTimeoutOverridesEditor";
import { StatusChangeToggleButton } from "./StatusChangeToggleButton";
import { statusChangeReasonIconName } from "./statusChangeReasonIcons";

type GroupAccountRoutingRuleDraft = {
  allowCutOut: boolean;
  allowCutIn: boolean;
  priorityTier: TagPriorityTier;
  fastModeRewriteMode: TagFastModeRewriteMode;
  imageToolRewriteMode: ImageToolRewriteMode;
  requestCompressionAlgorithm:
    | RequestCompressionAlgorithm
    | typeof REQUEST_COMPRESSION_INHERIT_VALUE;
  concurrencyLimit: number;
  upstream429RetryEnabled: boolean;
  upstream429MaxRetries: number;
  availableModels: string[];
  availableModelInput: string;
  availableModelsTouched: boolean;
  statusChangeReasons: Record<StatusChangeReasonCode, boolean>;
  timeoutOverrides: RoutingTimeoutOverrideDraft;
  timeoutOverrideEnabledFields: RoutingTimeoutOverrideEnabledState;
};

function normalizeRetryCount(value?: number | null): number {
  if (!Number.isFinite(value ?? NaN)) return 0;
  return Math.max(0, Math.min(5, Math.trunc(value ?? 0)));
}

function normalizeModelIds(values: string[]) {
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
  if (!options?.changedFieldsOnly) {
    return { ...statusChangeReasons };
  }
  const baseReasons = resolveStatusChangeReasons(options.baseRule?.statusChangeReasons);
  const patch: Partial<Record<StatusChangeReasonCode, boolean | null>> = {};
  for (const reason of STATUS_CHANGE_REASON_CODES) {
    if (statusChangeReasons[reason] !== baseReasons[reason]) {
      patch[reason] = statusChangeReasons[reason];
    }
  }
  return Object.keys(patch).length > 0 ? patch : undefined;
}

function buildDraft(
  rule?: GroupAccountRoutingRule | null,
  options?: {
    changedFieldsOnly?: boolean;
    effectiveTimeouts?: PoolRoutingTimeoutSettings | null;
    timeoutFieldSources?: EffectiveRoutingTimeoutFieldSources | null;
    timeoutOverrideSource?: "group" | "account";
  },
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
    requestCompressionAlgorithm: options?.changedFieldsOnly
      ? (rule?.requestCompressionAlgorithm ?? REQUEST_COMPRESSION_INHERIT_VALUE)
      : (rule?.requestCompressionAlgorithm ?? "identity"),
    concurrencyLimit: apiConcurrencyLimitToSliderValue(rule?.concurrencyLimit),
    upstream429RetryEnabled: rule?.upstream429RetryEnabled === true,
    upstream429MaxRetries: normalizeRetryCount(rule?.upstream429MaxRetries),
    availableModels: normalizeModelIds(rule?.availableModels ?? []),
    availableModelInput: "",
    availableModelsTouched: false,
    statusChangeReasons: resolveStatusChangeReasons(rule?.statusChangeReasons),
    timeoutOverrides,
    timeoutOverrideEnabledFields,
  };
}

function buildDraftResetKey(rule?: GroupAccountRoutingRule | null): string {
  return JSON.stringify(rule ?? null);
}

function buildPayload(
  draft: GroupAccountRoutingRuleDraft,
  options?: {
    changedFieldsOnly?: boolean;
    baseRule?: GroupAccountRoutingRule | null;
    effectiveTimeouts?: PoolRoutingTimeoutSettings | null;
    timeoutFieldSources?: EffectiveRoutingTimeoutFieldSources | null;
    timeoutFieldLabels?: Record<RoutingTimeoutFieldKey, string>;
    timeoutOverrideSource?: "group" | "account";
  },
): UpdateGroupAccountRoutingRulePayload | null {
  const timeoutLabels = options?.timeoutFieldLabels;
  if (!timeoutLabels) return null;
  const parsedTimeouts = parseRoutingTimeoutOverrideDraftWithEnabledState(
    draft.timeoutOverrides,
    draft.timeoutOverrideEnabledFields,
    timeoutLabels,
  );
  if (!parsedTimeouts.ok) {
    return null;
  }
  const requestCompressionAlgorithm =
    draft.requestCompressionAlgorithm === REQUEST_COMPRESSION_INHERIT_VALUE
      ? null
      : draft.requestCompressionAlgorithm;
  const payload: UpdateGroupAccountRoutingRulePayload = {
    allowCutOut: draft.allowCutOut,
    allowCutIn: draft.allowCutIn,
    priorityTier: draft.priorityTier,
    fastModeRewriteMode: draft.fastModeRewriteMode,
    imageToolRewriteMode: draft.imageToolRewriteMode,
    ...(requestCompressionAlgorithm == null ? {} : { requestCompressionAlgorithm }),
    concurrencyLimit: sliderConcurrencyLimitToApiValue(draft.concurrencyLimit),
    upstream429RetryEnabled: draft.upstream429RetryEnabled,
    upstream429MaxRetries: draft.upstream429RetryEnabled
      ? Math.max(1, normalizeRetryCount(draft.upstream429MaxRetries) || 1)
      : 0,
    availableModels: normalizeModelIds(draft.availableModels),
    statusChangeReasons: buildStatusChangeReasonPayload(draft.statusChangeReasons),
    timeouts: parsedTimeouts.patch,
  };

  if (options?.changedFieldsOnly && options.baseRule) {
    const base = options.baseRule;
    const changedPayload: UpdateGroupAccountRoutingRulePayload = {};
    if (draft.allowCutOut !== (base.allowCutOut ?? true)) {
      changedPayload.allowCutOut = payload.allowCutOut;
    }
    if (draft.allowCutIn !== (base.allowCutIn ?? true)) {
      changedPayload.allowCutIn = payload.allowCutIn;
    }
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
      draft.requestCompressionAlgorithm !==
      (base.requestCompressionAlgorithm ?? REQUEST_COMPRESSION_INHERIT_VALUE)
    ) {
      changedPayload.requestCompressionAlgorithm = requestCompressionAlgorithm;
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
    if (statusChangeReasonDiff) {
      changedPayload.statusChangeReasons = statusChangeReasonDiff;
    }
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
    if (!timeoutDiff.ok) {
      return null;
    }
    if (timeoutDiff.changed) {
      changedPayload.timeouts = timeoutDiff.patch;
    }
    return changedPayload;
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

interface GroupAccountRoutingRuleEditorProps {
  open: boolean;
  rule?: GroupAccountRoutingRule | null;
  busy?: boolean;
  error?: string | null;
  changedFieldsOnly?: boolean;
  effectiveTimeouts?: PoolRoutingTimeoutSettings | null;
  timeoutFieldSources?: EffectiveRoutingTimeoutFieldSources | null;
  timeoutOverrideSource?: "group" | "account";
  labels: GroupAccountRoutingRuleLabels;
  availableModelOptions?: string[];
  className?: string;
  onPayloadChange?: (payload: UpdateGroupAccountRoutingRulePayload | null) => void;
}

interface GroupAccountRoutingRuleDialogProps
  extends Omit<GroupAccountRoutingRuleEditorProps, "className" | "onPayloadChange"> {
  title: string;
  description: string;
  submitLabel: string;
  onClose: () => void;
  onSubmit: (payload: UpdateGroupAccountRoutingRulePayload) => Promise<void> | void;
}

export function GroupAccountRoutingRuleEditor({
  open,
  rule,
  busy = false,
  error,
  changedFieldsOnly = false,
  effectiveTimeouts,
  timeoutFieldSources,
  timeoutOverrideSource = "group",
  labels,
  availableModelOptions = [],
  className,
  onPayloadChange,
}: GroupAccountRoutingRuleEditorProps) {
  const [draft, setDraft] = useState<GroupAccountRoutingRuleDraft>(() =>
    buildDraft(rule, {
      changedFieldsOnly,
      effectiveTimeouts,
      timeoutFieldSources,
      timeoutOverrideSource,
    }),
  );
  const [baseRule, setBaseRule] = useState<GroupAccountRoutingRule | null>(() => rule ?? null);
  const previousOpenRef = useRef(open);
  const activeResetKeyRef = useRef<string | null>(open ? buildDraftResetKey(rule) : null);
  const resetKey = useMemo(() => buildDraftResetKey(rule), [rule]);

  useEffect(() => {
    const wasOpen = previousOpenRef.current;
    previousOpenRef.current = open;

    if (!open) {
      activeResetKeyRef.current = null;
      return;
    }

    if (wasOpen && activeResetKeyRef.current === resetKey) {
      return;
    }

    const nextBaseRule = rule ?? null;
    activeResetKeyRef.current = resetKey;
    setBaseRule(nextBaseRule);
    setDraft(
      buildDraft(nextBaseRule, {
        changedFieldsOnly,
        effectiveTimeouts,
        timeoutFieldSources,
        timeoutOverrideSource,
      }),
    );
  }, [
    changedFieldsOnly,
    effectiveTimeouts,
    open,
    resetKey,
    rule,
    timeoutFieldSources,
    timeoutOverrideSource,
  ]);

  const timeoutFieldLabels = useMemo(
    () => ({
      responsesFirstByteTimeoutSecs: labels.timeoutResponsesFirstByte,
      compactFirstByteTimeoutSecs: labels.timeoutCompactFirstByte,
      imageFirstByteTimeoutSecs: labels.timeoutImageFirstByte,
      responsesStreamTimeoutSecs: labels.timeoutResponsesStream,
      compactStreamTimeoutSecs: labels.timeoutCompactStream,
    }),
    [labels],
  );

  const timeoutValidationError = useMemo(() => {
    const parsed = parseRoutingTimeoutOverrideDraftWithEnabledState(
      draft.timeoutOverrides,
      draft.timeoutOverrideEnabledFields,
      timeoutFieldLabels,
    );
    return parsed.ok ? null : parsed.error;
  }, [draft.timeoutOverrideEnabledFields, draft.timeoutOverrides, timeoutFieldLabels]);

  const payload = useMemo(
    () =>
      buildPayload(draft, {
        changedFieldsOnly,
        baseRule,
        effectiveTimeouts,
        timeoutFieldSources,
        timeoutFieldLabels,
        timeoutOverrideSource,
      }),
    [
      baseRule,
      changedFieldsOnly,
      draft,
      effectiveTimeouts,
      timeoutFieldLabels,
      timeoutFieldSources,
      timeoutOverrideSource,
    ],
  );
  useEffect(() => {
    onPayloadChange?.(payload);
  }, [onPayloadChange, payload]);
  const availableModelComboboxOptions = useMemo<MultiSelectFilterOption[]>(() => {
    const values = normalizeModelIds([...availableModelOptions, ...draft.availableModels]);
    return values.map((value) => ({
      value,
      label: labels.availableModelsCustomLabel(value),
    }));
  }, [availableModelOptions, draft.availableModels, labels]);
  const trimmedModelInput = draft.availableModelInput.trim();
  const canAddCustomModel =
    trimmedModelInput.length > 0 && !draft.availableModels.includes(trimmedModelInput);
  const appendAvailableModel = (model: string) => {
    const normalizedModel = model.trim();
    if (!normalizedModel) return;
    setDraft((current) => ({
      ...current,
      availableModels: normalizeModelIds([...current.availableModels, normalizedModel]),
      availableModelInput: "",
      availableModelsTouched: true,
    }));
  };

  return (
    <div className={className ?? "space-y-5"}>
      <SelectField
        className="field"
        label={labels.priorityTier}
        name="groupPriorityTier"
        value={draft.priorityTier}
        disabled={busy}
        options={[
          { value: "primary", label: labels.priorityPrimary },
          { value: "normal", label: labels.priorityNormal },
          { value: "fallback", label: labels.priorityFallback },
          { value: "no_new", label: labels.priorityNoNew ?? "No new" },
        ]}
        onValueChange={(value) =>
          setDraft((current) => ({
            ...current,
            priorityTier: value as TagPriorityTier,
          }))
        }
      />

      <SelectField
        className="field"
        label={labels.fastModeRewriteMode}
        name="groupFastModeRewriteMode"
        value={draft.fastModeRewriteMode}
        disabled={busy}
        options={[
          { value: "keep_original", label: labels.fastModeKeepOriginal },
          { value: "fill_missing", label: labels.fastModeFillMissing },
          { value: "force_add", label: labels.fastModeForceAdd },
          { value: "force_remove", label: labels.fastModeForceRemove },
        ]}
        onValueChange={(value) =>
          setDraft((current) => ({
            ...current,
            fastModeRewriteMode: value as TagFastModeRewriteMode,
          }))
        }
      />

      <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
        {labels.imageToolRewriteHint ? (
          <p className="mb-3 text-xs leading-5 text-base-content/65">
            {labels.imageToolRewriteHint}
          </p>
        ) : null}
        <SelectField
          label={labels.imageToolRewriteMode}
          name="groupImageToolRewriteMode"
          value={draft.imageToolRewriteMode}
          disabled={busy}
          options={[
            { value: "keep_original", label: labels.imageToolKeepOriginal },
            { value: "fill_missing", label: labels.imageToolFillMissing },
            { value: "force_add", label: labels.imageToolForceAdd },
            { value: "force_remove", label: labels.imageToolForceRemove },
          ]}
          onValueChange={(value) =>
            setDraft((current) => ({
              ...current,
              imageToolRewriteMode: value as ImageToolRewriteMode,
            }))
          }
        />
      </div>

      <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
        {labels.requestCompressionHint ? (
          <p className="mb-2 text-xs leading-5 text-base-content/65">
            {labels.requestCompressionHint}
          </p>
        ) : null}
        {labels.requestCompressionMixedGroupHint ? (
          <p className="mb-3 text-xs leading-5 text-base-content/55">
            {labels.requestCompressionMixedGroupHint}
          </p>
        ) : null}
        <SelectField
          label={labels.requestCompressionAlgorithm}
          name="groupRequestCompressionAlgorithm"
          value={draft.requestCompressionAlgorithm}
          disabled={busy}
          options={[
            ...(changedFieldsOnly
              ? [
                  {
                    value: REQUEST_COMPRESSION_INHERIT_VALUE,
                    label: labels.requestCompressionInherited,
                  },
                ]
              : []),
            {
              value: "follow",
              label: requestCompressionAlgorithmLabel("follow", labels),
            },
            {
              value: "identity",
              label: requestCompressionAlgorithmLabel("identity", labels),
            },
            {
              value: "gzip",
              label: requestCompressionAlgorithmLabel("gzip", labels),
            },
            {
              value: "deflate",
              label: requestCompressionAlgorithmLabel("deflate", labels),
            },
            {
              value: "zstd",
              label: requestCompressionAlgorithmLabel("zstd", labels),
            },
          ]}
          onValueChange={(value) =>
            setDraft((current) => ({
              ...current,
              requestCompressionAlgorithm:
                value as GroupAccountRoutingRuleDraft["requestCompressionAlgorithm"],
            }))
          }
        />
      </div>

      {effectiveTimeouts ? (
        <RoutingTimeoutOverridesEditor
          fields={[
            {
              key: "responsesFirstByteTimeoutSecs",
              label: labels.timeoutResponsesFirstByte,
            },
            {
              key: "compactFirstByteTimeoutSecs",
              label: labels.timeoutCompactFirstByte,
            },
            {
              key: "imageFirstByteTimeoutSecs",
              label: labels.timeoutImageFirstByte,
            },
            {
              key: "responsesStreamTimeoutSecs",
              label: labels.timeoutResponsesStream,
            },
            {
              key: "compactStreamTimeoutSecs",
              label: labels.timeoutCompactStream,
            },
          ]}
          effective={effectiveTimeouts}
          draft={draft.timeoutOverrides}
          enabledFields={draft.timeoutOverrideEnabledFields}
          sources={timeoutFieldSources}
          busy={busy}
          disabled={busy}
          labels={{
            sectionTitle: labels.timeoutSectionTitle,
            sectionHint: labels.timeoutSectionHint,
            inheritedValue: labels.timeoutInheritedValue,
            overrideValue: labels.timeoutOverrideValue,
            sourceRoot: labels.timeoutSourceGlobal,
            sourceGroup: labels.timeoutSourceGroup,
            sourceAccount: labels.timeoutSourceAccount,
            sourceConversation: labels.timeoutSourceConversation,
            clearField: labels.timeoutClearField,
            inheritField: labels.timeoutInheritField,
            savingField: labels.validation,
          }}
          onDraftChange={(key, value) =>
            setDraft((current) => ({
              ...current,
              timeoutOverrides: {
                ...current.timeoutOverrides,
                [key]: value,
              },
            }))
          }
          onFieldEnabledChange={(key, enabled) =>
            setDraft((current) => ({
              ...current,
              timeoutOverrideEnabledFields: {
                ...current.timeoutOverrideEnabledFields,
                [key]: enabled,
              },
              timeoutOverrides:
                enabled && (current.timeoutOverrides[key] ?? "").trim() === ""
                  ? {
                      ...current.timeoutOverrides,
                      [key]: effectiveTimeouts?.[key] != null ? String(effectiveTimeouts[key]) : "",
                    }
                  : current.timeoutOverrides,
            }))
          }
        />
      ) : null}

      <div className="grid gap-3 sm:grid-cols-2">
        <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
          <div className="flex items-center justify-between gap-4">
            <p className="font-medium text-base-content">
              {labels.forbidCutOut ?? labels.allowCutOut}
            </p>
            <Switch
              checked={!draft.allowCutOut}
              onCheckedChange={(checked) =>
                setDraft((current) => ({
                  ...current,
                  allowCutOut: !checked,
                }))
              }
            />
          </div>
        </div>
        <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
          <div className="flex items-center justify-between gap-4">
            <p className="font-medium text-base-content">
              {labels.forbidCutIn ?? labels.allowCutIn}
            </p>
            <Switch
              checked={!draft.allowCutIn}
              onCheckedChange={(checked) =>
                setDraft((current) => ({
                  ...current,
                  allowCutIn: !checked,
                }))
              }
            />
          </div>
        </div>
      </div>

      <ConcurrencyLimitSlider
        value={draft.concurrencyLimit}
        disabled={busy}
        title={labels.concurrencyLimit}
        description={labels.concurrencyHint}
        currentLabel={labels.currentValue}
        unlimitedLabel={labels.unlimited}
        onChange={(value) => setDraft((current) => ({ ...current, concurrencyLimit: value }))}
      />

      <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
        <div className="space-y-1">
          <p className="font-medium text-base-content">{labels.availableModels}</p>
          <p className="text-xs leading-5 text-base-content/65">{labels.availableModelsHint}</p>
        </div>
        <div className="mt-4 grid gap-3">
          <MultiSelectFilterCombobox
            options={availableModelComboboxOptions}
            value={draft.availableModels}
            onValueChange={(value) =>
              setDraft((current) => ({
                ...current,
                availableModels: normalizeModelIds(value),
                availableModelsTouched: true,
              }))
            }
            disabled={busy}
            placeholder={labels.availableModelsAll}
            searchPlaceholder={labels.availableModelsSearchPlaceholder}
            emptyLabel={labels.availableModelsEmpty}
            clearLabel={labels.availableModelsInherited}
            ariaLabel={labels.availableModels}
          />
          <div className="flex gap-2">
            <Input
              name="availableModelInput"
              value={draft.availableModelInput}
              placeholder={labels.availableModelsAddCustom}
              disabled={busy}
              onChange={(event) =>
                setDraft((current) => ({
                  ...current,
                  availableModelInput: event.target.value,
                }))
              }
              onKeyDown={(event) => {
                if (event.key !== "Enter" || !canAddCustomModel) return;
                event.preventDefault();
                appendAvailableModel(trimmedModelInput);
              }}
            />
            <Button
              type="button"
              variant="outline"
              disabled={busy || !canAddCustomModel}
              onClick={() => appendAvailableModel(trimmedModelInput)}
            >
              <AppIcon name="plus" className="mr-2 h-4 w-4" aria-hidden />
              {labels.availableModelsAddCustom}
            </Button>
          </div>
          {draft.availableModels.length > 0 ? (
            <div className="flex flex-wrap gap-2">
              {draft.availableModels.map((model) => (
                <Badge key={model} variant="secondary" className="gap-1 pr-1">
                  <span>{labels.availableModelsCustomLabel(model)}</span>
                  <button
                    type="button"
                    className="rounded-full p-0.5 text-base-content/55 transition hover:bg-base-300/70 hover:text-base-content"
                    aria-label={`${labels.availableModelsRemove} ${model}`}
                    onClick={() =>
                      setDraft((current) => ({
                        ...current,
                        availableModels: current.availableModels.filter((value) => value !== model),
                        availableModelsTouched: true,
                      }))
                    }
                  >
                    <AppIcon name="close" className="h-3 w-3" aria-hidden />
                  </button>
                </Badge>
              ))}
            </div>
          ) : null}
        </div>
      </div>

      <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
        <div className="space-y-1">
          <div className="space-y-1">
            <p className="font-medium text-base-content">{labels.upstream429Retry}</p>
            <p className="text-xs leading-5 text-base-content/65">{labels.upstream429RetryHint}</p>
          </div>
        </div>
        <div className="mt-4">
          <PolicyInlineOptionGroup<number>
            ariaLabel={labels.upstream429Retry}
            value={
              draft.upstream429RetryEnabled
                ? Math.max(1, normalizeRetryCount(draft.upstream429MaxRetries) || 1)
                : 0
            }
            disabled={busy}
            options={[0, 1, 2, 3, 4, 5].map((value) => ({
              value,
              label: String(value),
            }))}
            onChange={(value) =>
              setDraft((current) => ({
                ...current,
                upstream429RetryEnabled: value > 0,
                upstream429MaxRetries: normalizeRetryCount(value),
              }))
            }
          />
        </div>
      </div>

      <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
        <div className="space-y-1">
          <p className="font-medium text-base-content">
            {labels.statusChangeReasonSectionTitle ?? "Status change trigger reasons"}
          </p>
          {labels.statusChangeReasonSectionHint ? (
            <p className="text-xs leading-5 text-base-content/65">
              {labels.statusChangeReasonSectionHint}
            </p>
          ) : null}
        </div>
        <div className="mt-4 grid gap-2 sm:grid-cols-2 lg:auto-rows-fr lg:grid-cols-4">
          {STATUS_CHANGE_REASON_CODES.map((reason) => {
            const reasonLabel = labels.statusChangeReasonLabel?.(reason) ?? reason;
            return (
              <StatusChangeToggleButton
                key={reason}
                title={reasonLabel}
                iconName={statusChangeReasonIconName(reason)}
                pressed={draft.statusChangeReasons[reason]}
                disabled={busy}
                activeLabel={labels.statusChangeReasonToggleEnabled}
                inactiveLabel={labels.statusChangeReasonToggleDisabled}
                onPressedChange={(checked) =>
                  setDraft((current) => ({
                    ...current,
                    statusChangeReasons: {
                      ...current.statusChangeReasons,
                      [reason]: checked,
                    },
                  }))
                }
                ariaLabel={reasonLabel}
                className="min-h-[4rem]"
              />
            );
          })}
        </div>
      </div>

      {error ? <p className="text-sm text-error">{error}</p> : null}
      {timeoutValidationError ? (
        <p className="text-sm text-error">{timeoutValidationError}</p>
      ) : !payload ? (
        <p className="text-sm text-warning">{labels.validation}</p>
      ) : null}
    </div>
  );
}

export function GroupAccountRoutingRuleDialog({
  open,
  title,
  description,
  submitLabel,
  rule,
  busy = false,
  error,
  changedFieldsOnly = false,
  effectiveTimeouts,
  timeoutFieldSources,
  timeoutOverrideSource = "group",
  onClose,
  onSubmit,
  labels,
  availableModelOptions = [],
}: GroupAccountRoutingRuleDialogProps) {
  const [payload, setPayload] = useState<UpdateGroupAccountRoutingRulePayload | null>(null);
  const disabled = !payload || busy;

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => (!busy && !nextOpen ? onClose() : undefined)}>
      <DialogContent className="flex max-h-[calc(100dvh-0.75rem)] max-w-none flex-col overflow-hidden p-0 desktop:max-h-[min(90vh,calc(100vh-2rem))] desktop:w-[min(48rem,calc(100vw-2rem))]">
        <div className="shrink-0 border-b border-base-300/80 px-5 py-4 desktop:px-6 desktop:py-5">
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5 desktop:px-6">
          <GroupAccountRoutingRuleEditor
            open={open}
            rule={rule}
            busy={busy}
            error={error}
            changedFieldsOnly={changedFieldsOnly}
            effectiveTimeouts={effectiveTimeouts}
            timeoutFieldSources={timeoutFieldSources}
            timeoutOverrideSource={timeoutOverrideSource}
            labels={labels}
            availableModelOptions={availableModelOptions}
            onPayloadChange={setPayload}
          />
        </div>
        <div className="shrink-0 border-t border-base-300/80 bg-base-100/94 px-5 pb-[max(env(safe-area-inset-bottom),1rem)] pt-4 backdrop-blur desktop:px-6 desktop:py-4">
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
              {labels.cancel}
            </Button>
            <Button
              type="button"
              disabled={disabled}
              onClick={() => {
                if (payload) void onSubmit(payload);
              }}
            >
              {submitLabel}
            </Button>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
}
