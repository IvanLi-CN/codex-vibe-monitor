import type { Dispatch, SetStateAction } from "react";
import { useMemo } from "react";
import { Button } from "../../components/ui/button";
import { Chip } from "../../components/ui/chip";
import { InfoTooltip } from "../../components/ui/info-tooltip";
import { Input } from "../../components/ui/input";
import { SelectField } from "../../components/ui/select-field";
import { Switch } from "../../components/ui/switch";
import { useCompactViewport } from "../../hooks/useCompactViewport";
import type {
  EffectiveRoutingTimeoutFieldSources,
  PoolRoutingTimeoutSettings,
  UpdateGroupAccountRoutingRulePayload,
} from "../../lib/api";
import {
  REQUEST_COMPRESSION_INHERIT_VALUE,
  requestCompressionAlgorithmLabel,
} from "../../lib/requestCompression";
import { STATUS_CHANGE_REASON_CODES } from "../../lib/upstreamAccountStatusChangeReasons";
import { AppIcon } from "../shared/AppIcon";
import { ConcurrencyLimitSlider } from "./ConcurrencyLimitSlider";
import {
  AVAILABLE_MODE_INHERIT_VALUE,
  CODEX_IMAGEGEN_INHERIT_VALUE,
  type GroupAccountRoutingRuleDraft,
  type GroupAccountRoutingRuleLabels,
  normalizeModelIds,
  normalizeRetryCount,
} from "./GroupAccountRoutingRuleDialog.model";
import {
  MultiSelectFilterCombobox,
  type MultiSelectFilterOption,
} from "./MultiSelectFilterCombobox";
import { PolicyInlineOptionGroup } from "./PolicyInlineOptionGroup";
import {
  type RoutingTimeoutEditorFieldConfig,
  RoutingTimeoutOverridesEditor,
} from "./RoutingTimeoutOverridesEditor";
import { StatusChangeToggleButton } from "./StatusChangeToggleButton";
import { statusChangeReasonIconName } from "./statusChangeReasonIcons";

type DraftSetter = Dispatch<SetStateAction<GroupAccountRoutingRuleDraft>>;

type ResponsivePolicyOption<T extends string> = {
  value: T;
  label: string;
};

type ResponsivePolicySelectProps<T extends string> = {
  compact: boolean;
  label: string;
  helpContent?: string;
  name: string;
  value: T;
  options: readonly ResponsivePolicyOption<T>[];
  disabled: boolean;
  onValueChange: (value: T) => void;
};

function ResponsivePolicySelect<T extends string>({
  compact,
  label,
  helpContent,
  name,
  value,
  options,
  disabled,
  onValueChange,
}: ResponsivePolicySelectProps<T>) {
  const fieldLabel = (
    <div className="field-label flex items-center gap-1">
      <span>{label}</span>
      {helpContent ? <InfoTooltip label={`${label} help`} content={helpContent} /> : null}
    </div>
  );
  if (compact) {
    return (
      <div className="field">
        {fieldLabel}
        <SelectField
          name={name}
          value={value}
          disabled={disabled}
          aria-label={label}
          options={options}
          onValueChange={(nextValue) => onValueChange(nextValue as T)}
        />
      </div>
    );
  }
  return (
    <div className="field">
      {fieldLabel}
      <PolicyInlineOptionGroup<T>
        ariaLabel={label}
        value={value}
        options={options}
        disabled={disabled}
        onChange={onValueChange}
      />
    </div>
  );
}

type PolicySectionProps = {
  compact: boolean;
  busy: boolean;
  draft: GroupAccountRoutingRuleDraft;
  setDraft: DraftSetter;
  labels: GroupAccountRoutingRuleLabels;
  changedFieldsOnly: boolean;
};

function PriorityAndFastModeSection({
  compact,
  busy,
  draft,
  setDraft,
  labels,
}: PolicySectionProps) {
  return (
    <>
      <ResponsivePolicySelect
        compact={compact}
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
        onValueChange={(value) => setDraft((current) => ({ ...current, priorityTier: value }))}
      />
      <ResponsivePolicySelect
        compact={compact}
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
          setDraft((current) => ({ ...current, fastModeRewriteMode: value }))
        }
      />
    </>
  );
}

function ImageRewriteSection({ compact, busy, draft, setDraft, labels }: PolicySectionProps) {
  const imageOptions = [
    { value: "keep_original", label: labels.imageToolKeepOriginal },
    { value: "fill_missing", label: labels.imageToolFillMissing },
    { value: "force_add", label: labels.imageToolForceAdd },
    { value: "force_remove", label: labels.imageToolForceRemove },
  ] as const;
  return (
    <>
      <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
        <ResponsivePolicySelect
          compact={compact}
          label={labels.imageToolRewriteMode}
          helpContent={labels.imageToolRewriteHint}
          name="groupImageToolRewriteMode"
          value={draft.imageToolRewriteMode}
          disabled={busy}
          options={imageOptions}
          onValueChange={(value) =>
            setDraft((current) => ({ ...current, imageToolRewriteMode: value }))
          }
        />
      </div>
      <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
        <ResponsivePolicySelect
          compact={compact}
          label={labels.codexImagegenRewriteMode ?? "Codex imagegen"}
          helpContent={labels.codexImagegenRewriteHint}
          name="groupCodexImagegenRewriteMode"
          value={draft.codexImagegenRewriteMode}
          disabled={busy}
          options={[
            { value: CODEX_IMAGEGEN_INHERIT_VALUE, label: labels.requestCompressionInherited },
            ...imageOptions,
          ]}
          onValueChange={(value) =>
            setDraft((current) => ({ ...current, codexImagegenRewriteMode: value }))
          }
        />
      </div>
    </>
  );
}

function RequestCompressionSection({
  compact,
  busy,
  draft,
  setDraft,
  labels,
  changedFieldsOnly,
}: PolicySectionProps) {
  return (
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
      <ResponsivePolicySelect
        compact={compact}
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
          ...(["follow", "identity", "gzip", "deflate", "zstd"] as const).map((value) => ({
            value,
            label: requestCompressionAlgorithmLabel(value, labels),
          })),
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
  );
}

function TimeoutSection({
  busy,
  draft,
  setDraft,
  labels,
  effectiveTimeouts,
  timeoutFieldSources,
}: {
  busy: boolean;
  draft: GroupAccountRoutingRuleDraft;
  setDraft: DraftSetter;
  labels: GroupAccountRoutingRuleLabels;
  effectiveTimeouts: PoolRoutingTimeoutSettings;
  timeoutFieldSources?: EffectiveRoutingTimeoutFieldSources | null;
}) {
  const fields: RoutingTimeoutEditorFieldConfig[] = [
    { key: "responsesFirstByteTimeoutSecs", label: labels.timeoutResponsesFirstByte },
    { key: "compactFirstByteTimeoutSecs", label: labels.timeoutCompactFirstByte },
    { key: "imageFirstByteTimeoutSecs", label: labels.timeoutImageFirstByte },
    { key: "responsesStreamTimeoutSecs", label: labels.timeoutResponsesStream },
    { key: "compactStreamTimeoutSecs", label: labels.timeoutCompactStream },
  ];
  return (
    <RoutingTimeoutOverridesEditor
      fields={fields}
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
          timeoutOverrides: { ...current.timeoutOverrides, [key]: value },
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
                  [key]: effectiveTimeouts[key] != null ? String(effectiveTimeouts[key]) : "",
                }
              : current.timeoutOverrides,
        }))
      }
    />
  );
}

function CutAndConcurrencySections({ draft, setDraft, busy, labels }: PolicySectionProps) {
  const cutFields: Array<{
    key: "allowCutOut" | "allowCutIn";
    label: string;
  }> = [
    { key: "allowCutOut", label: labels.forbidCutOut ?? labels.allowCutOut },
    { key: "allowCutIn", label: labels.forbidCutIn ?? labels.allowCutIn },
  ];
  return (
    <>
      <div className="grid gap-3 sm:grid-cols-2">
        {cutFields.map(({ key, label }) => (
          <div key={key} className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
            <div className="flex items-center justify-between gap-4">
              <p className="font-medium text-base-content">{label}</p>
              <Switch
                checked={!draft[key]}
                onCheckedChange={(checked) =>
                  setDraft((current) => ({ ...current, [key]: !checked }))
                }
              />
            </div>
          </div>
        ))}
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
    </>
  );
}

function AvailableModelsModeSelect({
  draft,
  setDraft,
  busy,
  labels,
  changedFieldsOnly,
}: PolicySectionProps) {
  return (
    <div className="space-y-2">
      <p className="text-xs font-medium uppercase tracking-[0.08em] text-base-content/60">
        {labels.availableModelsMode ?? labels.availableModels}
      </p>
      <PolicyInlineOptionGroup<string>
        ariaLabel={labels.availableModelsMode ?? labels.availableModels}
        value={draft.availableModelsMode}
        disabled={busy}
        options={[
          ...(changedFieldsOnly
            ? [
                {
                  value: AVAILABLE_MODE_INHERIT_VALUE,
                  label: labels.availableModelsModeInherited ?? labels.availableModelsInherited,
                },
              ]
            : []),
          { value: "allowlist", label: labels.availableModelsAllowlist ?? "Allowlist" },
          { value: "denylist", label: labels.availableModelsDenylist ?? "Denylist" },
        ]}
        onChange={(value) =>
          setDraft((current) => ({
            ...current,
            availableModelsMode: value as GroupAccountRoutingRuleDraft["availableModelsMode"],
            availableModelsTouched: true,
          }))
        }
      />
    </div>
  );
}

function AvailableModelsInput({
  draft,
  setDraft,
  busy,
  labels,
  canAddCustomModel,
  appendAvailableModel,
}: {
  draft: GroupAccountRoutingRuleDraft;
  setDraft: DraftSetter;
  busy: boolean;
  labels: GroupAccountRoutingRuleLabels;
  canAddCustomModel: boolean;
  appendAvailableModel: (model: string) => void;
}) {
  const trimmedModelInput = draft.availableModelInput.trim();
  return (
    <div className="flex gap-2">
      <Input
        name="availableModelInput"
        value={draft.availableModelInput}
        placeholder={labels.availableModelsAddCustom}
        disabled={busy}
        onChange={(event) =>
          setDraft((current) => ({ ...current, availableModelInput: event.target.value }))
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
  );
}

function AvailableModelsList({
  draft,
  setDraft,
  labels,
}: Pick<PolicySectionProps, "draft" | "setDraft" | "labels">) {
  if (draft.availableModels.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-2">
      {draft.availableModels.map((model) => (
        <Chip key={model} tone="secondary" className="gap-1 pr-1">
          <span>{labels.availableModelsCustomLabel(model)}</span>
          <button
            type="button"
            className="rounded-full p-0.5 text-current/70 transition hover:bg-base-300/70 hover:text-current"
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
        </Chip>
      ))}
    </div>
  );
}

function AvailableModelsSection({
  draft,
  setDraft,
  busy,
  labels,
  changedFieldsOnly,
  availableModelOptions,
}: PolicySectionProps & { availableModelOptions: string[] }) {
  const options = useMemo<MultiSelectFilterOption[]>(
    () =>
      normalizeModelIds([...availableModelOptions, ...draft.availableModels]).map((value) => ({
        value,
        label: labels.availableModelsCustomLabel(value),
      })),
    [availableModelOptions, draft.availableModels, labels],
  );
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
    <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
      <div className="space-y-1">
        <p className="font-medium text-base-content">{labels.availableModels}</p>
        <p className="text-xs leading-5 text-base-content/65">{labels.availableModelsHint}</p>
      </div>
      <div className="mt-4 grid gap-3">
        <AvailableModelsModeSelect
          {...{ draft, setDraft, busy, labels, changedFieldsOnly, compact: false }}
        />
        <MultiSelectFilterCombobox
          options={options}
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
        <AvailableModelsInput
          {...{ draft, setDraft, busy, labels, canAddCustomModel, appendAvailableModel }}
        />
        <AvailableModelsList {...{ draft, setDraft, labels }} />
      </div>
    </div>
  );
}

function RetrySection({ draft, setDraft, busy, labels }: PolicySectionProps) {
  return (
    <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
      <p className="font-medium text-base-content">{labels.upstream429Retry}</p>
      <p className="text-xs leading-5 text-base-content/65">{labels.upstream429RetryHint}</p>
      <div className="mt-4">
        <PolicyInlineOptionGroup<number>
          ariaLabel={labels.upstream429Retry}
          value={
            draft.upstream429RetryEnabled
              ? Math.max(1, normalizeRetryCount(draft.upstream429MaxRetries) || 1)
              : 0
          }
          disabled={busy}
          options={[0, 1, 2, 3, 4, 5].map((value) => ({ value, label: String(value) }))}
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
  );
}

function StatusReasonsSection({ draft, setDraft, busy, labels }: PolicySectionProps) {
  return (
    <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
      <p className="font-medium text-base-content">
        {labels.statusChangeReasonSectionTitle ?? "Status change trigger reasons"}
      </p>
      {labels.statusChangeReasonSectionHint ? (
        <p className="text-xs leading-5 text-base-content/65">
          {labels.statusChangeReasonSectionHint}
        </p>
      ) : null}
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
                  statusChangeReasons: { ...current.statusChangeReasons, [reason]: checked },
                }))
              }
              ariaLabel={reasonLabel}
              className="min-h-[4rem]"
            />
          );
        })}
      </div>
    </div>
  );
}

function ValidationMessages({
  error,
  timeoutValidationError,
  payload,
  labels,
}: Pick<EditorContentProps, "error" | "timeoutValidationError" | "payload" | "labels">) {
  return (
    <>
      {error ? <p className="text-sm text-error">{error}</p> : null}
      {timeoutValidationError ? (
        <p className="text-sm text-error">{timeoutValidationError}</p>
      ) : !payload ? (
        <p className="text-sm text-warning">{labels.validation}</p>
      ) : null}
    </>
  );
}

interface EditorContentProps {
  className?: string;
  draft: GroupAccountRoutingRuleDraft;
  setDraft: DraftSetter;
  busy: boolean;
  changedFieldsOnly: boolean;
  effectiveTimeouts?: PoolRoutingTimeoutSettings | null;
  timeoutFieldSources?: EffectiveRoutingTimeoutFieldSources | null;
  labels: GroupAccountRoutingRuleLabels;
  availableModelOptions: string[];
  error?: string | null;
  timeoutValidationError: string | null;
  payload: UpdateGroupAccountRoutingRulePayload | null;
}

export function GroupAccountRoutingRuleEditorContent({
  className,
  draft,
  setDraft,
  busy,
  changedFieldsOnly,
  effectiveTimeouts,
  timeoutFieldSources,
  labels,
  availableModelOptions,
  error,
  timeoutValidationError,
  payload,
}: EditorContentProps) {
  const compact = useCompactViewport();
  const policyProps = { compact, busy, draft, setDraft, labels, changedFieldsOnly };
  return (
    <div className={className ?? "space-y-5"}>
      <PriorityAndFastModeSection {...policyProps} />
      <ImageRewriteSection {...policyProps} />
      <RequestCompressionSection {...policyProps} />
      {effectiveTimeouts ? (
        <TimeoutSection
          {...{ busy, draft, setDraft, labels, effectiveTimeouts, timeoutFieldSources }}
        />
      ) : null}
      <CutAndConcurrencySections {...policyProps} />
      <AvailableModelsSection {...policyProps} availableModelOptions={availableModelOptions} />
      <RetrySection {...policyProps} />
      <StatusReasonsSection {...policyProps} />
      <ValidationMessages {...{ error, timeoutValidationError, payload, labels }} />
    </div>
  );
}
