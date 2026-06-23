import { useEffect, useMemo, useRef, useState } from "react";
import { AppIcon } from "./AppIcon";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./ui/dialog";
import { Input } from "./ui/input";
import { SelectField } from "./ui/select-field";
import { Switch } from "./ui/switch";
import { Badge } from "./ui/badge";
import { ConcurrencyLimitSlider } from "./ConcurrencyLimitSlider";
import { MultiSelectFilterCombobox, type MultiSelectFilterOption } from "./MultiSelectFilterCombobox";
import type {
  GroupAccountRoutingRule,
  ImageToolRewriteMode,
  TagFastModeRewriteMode,
  TagPriorityTier,
  UpdateGroupAccountRoutingRulePayload,
} from "../lib/api";
import { apiConcurrencyLimitToSliderValue, sliderConcurrencyLimitToApiValue } from "../lib/concurrencyLimit";

type GroupAccountRoutingRuleDraft = {
  blockNewConversations: boolean;
  allowCutOut: boolean;
  allowCutIn: boolean;
  priorityTier: TagPriorityTier;
  fastModeRewriteMode: TagFastModeRewriteMode;
  imageToolRewriteMode: ImageToolRewriteMode;
  concurrencyLimit: number;
  upstream429RetryEnabled: boolean;
  upstream429MaxRetries: number;
  availableModels: string[];
  availableModelInput: string;
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

function buildDraft(rule?: GroupAccountRoutingRule | null): GroupAccountRoutingRuleDraft {
  return {
    blockNewConversations: rule?.blockNewConversations ?? false,
    allowCutOut: rule?.allowCutOut ?? true,
    allowCutIn: rule?.allowCutIn ?? true,
    priorityTier: rule?.priorityTier ?? "normal",
    fastModeRewriteMode: rule?.fastModeRewriteMode ?? "keep_original",
    imageToolRewriteMode: rule?.imageToolRewriteMode ?? "keep_original",
    concurrencyLimit: apiConcurrencyLimitToSliderValue(rule?.concurrencyLimit),
    upstream429RetryEnabled: rule?.upstream429RetryEnabled === true,
    upstream429MaxRetries: normalizeRetryCount(rule?.upstream429MaxRetries),
    availableModels: normalizeModelIds(rule?.availableModels ?? []),
    availableModelInput: "",
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
  },
): UpdateGroupAccountRoutingRulePayload | null {
  const payload: UpdateGroupAccountRoutingRulePayload = {
    blockNewConversations: draft.blockNewConversations,
    allowCutOut: draft.allowCutOut,
    allowCutIn: draft.allowCutIn,
    priorityTier: draft.priorityTier,
    fastModeRewriteMode: draft.fastModeRewriteMode,
    imageToolRewriteMode: draft.imageToolRewriteMode,
    concurrencyLimit: sliderConcurrencyLimitToApiValue(draft.concurrencyLimit),
    upstream429RetryEnabled: draft.upstream429RetryEnabled,
    upstream429MaxRetries: draft.upstream429RetryEnabled
      ? Math.max(1, normalizeRetryCount(draft.upstream429MaxRetries) || 1)
      : 0,
    availableModels: normalizeModelIds(draft.availableModels),
  };

  if (options?.changedFieldsOnly && options.baseRule) {
    const base = options.baseRule;
    const changedPayload: UpdateGroupAccountRoutingRulePayload = {};
    if (draft.blockNewConversations !== (base.blockNewConversations ?? false)) {
      changedPayload.blockNewConversations = payload.blockNewConversations;
    }
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
      JSON.stringify(payload.availableModels ?? []) !==
      JSON.stringify(normalizeModelIds(base.availableModels ?? []))
    ) {
      changedPayload.availableModels = payload.availableModels;
    }
    return changedPayload;
  }

  return payload;
}

interface GroupAccountRoutingRuleDialogProps {
  open: boolean;
  title: string;
  description: string;
  submitLabel: string;
  rule?: GroupAccountRoutingRule | null;
  busy?: boolean;
  error?: string | null;
  changedFieldsOnly?: boolean;
  onClose: () => void;
  onSubmit: (payload: UpdateGroupAccountRoutingRulePayload) => Promise<void> | void;
  labels: {
    blockNewConversations: string;
    forbidNewConversation?: string;
    allowCutOut: string;
    allowCutIn: string;
    forbidCutOut?: string;
    forbidCutIn?: string;
    priorityTier: string;
    priorityPrimary: string;
    priorityNormal: string;
    priorityFallback: string;
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
    cancel: string;
    validation: string;
  };
  availableModelOptions?: string[];
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
  onClose,
  onSubmit,
  labels,
  availableModelOptions = [],
}: GroupAccountRoutingRuleDialogProps) {
  const [draft, setDraft] = useState<GroupAccountRoutingRuleDraft>(() =>
    buildDraft(rule),
  );
  const [baseRule, setBaseRule] = useState<GroupAccountRoutingRule | null>(
    () => rule ?? null,
  );
  const previousOpenRef = useRef(open);
  const activeResetKeyRef = useRef<string | null>(
    open ? buildDraftResetKey(rule) : null,
  );
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
    setDraft(buildDraft(nextBaseRule));
  }, [open, resetKey, rule]);

  const payload = useMemo(
    () =>
      buildPayload(draft, {
        changedFieldsOnly,
        baseRule,
      }),
    [baseRule, changedFieldsOnly, draft],
  );
  const disabled = !payload || busy;
  const availableModelComboboxOptions = useMemo<MultiSelectFilterOption[]>(
    () => {
      const values = normalizeModelIds([
        ...availableModelOptions,
        ...draft.availableModels,
      ]);
      return values.map((value) => ({
        value,
        label: labels.availableModelsCustomLabel(value),
      }));
    },
    [availableModelOptions, draft.availableModels, labels],
  );
  const trimmedModelInput = draft.availableModelInput.trim();
  const canAddCustomModel =
    trimmedModelInput.length > 0 &&
    !draft.availableModels.includes(trimmedModelInput);
  const appendAvailableModel = (model: string) => {
    const normalizedModel = model.trim();
    if (!normalizedModel) return;
    setDraft((current) => ({
      ...current,
      availableModels: normalizeModelIds([
        ...current.availableModels,
        normalizedModel,
      ]),
      availableModelInput: "",
    }));
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => (!busy && !nextOpen ? onClose() : undefined)}
    >
      <DialogContent className="p-0">
        <div className="border-b border-base-300/80 px-6 py-5">
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
        </div>
        <div className="space-y-5 px-6 py-5">
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
            <div className="flex items-start justify-between gap-4">
              <div>
                <p className="font-medium text-base-content">
                  {labels.blockNewConversations}
                </p>
                {labels.forbidNewConversation ? (
                  <p className="text-xs leading-5 text-base-content/65">
                    {labels.forbidNewConversation}
                  </p>
                ) : null}
              </div>
              <Switch
                checked={draft.blockNewConversations}
                onCheckedChange={(checked) =>
                  setDraft((current) => ({
                    ...current,
                    blockNewConversations: checked,
                  }))
                }
              />
            </div>
          </div>

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
            onChange={(value) =>
              setDraft((current) => ({ ...current, concurrencyLimit: value }))
            }
          />

          <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
            <div className="space-y-1">
              <p className="font-medium text-base-content">
                {labels.availableModels}
              </p>
              <p className="text-xs leading-5 text-base-content/65">
                {labels.availableModelsHint}
              </p>
            </div>
            <div className="mt-4 grid gap-3">
              <MultiSelectFilterCombobox
                options={availableModelComboboxOptions}
                value={draft.availableModels}
                onValueChange={(value) =>
                  setDraft((current) => ({
                    ...current,
                    availableModels: normalizeModelIds(value),
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
                    <Badge
                      key={model}
                      variant="secondary"
                      className="gap-1 pr-1"
                    >
                      <span>{labels.availableModelsCustomLabel(model)}</span>
                      <button
                        type="button"
                        className="rounded-full p-0.5 text-base-content/55 transition hover:bg-base-300/70 hover:text-base-content"
                        aria-label={`${labels.availableModelsRemove} ${model}`}
                        onClick={() =>
                          setDraft((current) => ({
                            ...current,
                            availableModels: current.availableModels.filter(
                              (value) => value !== model,
                            ),
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
            <div className="flex items-start justify-between gap-4">
              <div className="space-y-1">
                <p className="font-medium text-base-content">
                  {labels.upstream429Retry}
                </p>
                <p className="text-xs leading-5 text-base-content/65">
                  {labels.upstream429RetryHint}
                </p>
              </div>
              <Switch
                checked={draft.upstream429RetryEnabled}
                onCheckedChange={(checked) =>
                  setDraft((current) => ({
                    ...current,
                    upstream429RetryEnabled: checked,
                    upstream429MaxRetries: checked
                      ? Math.max(1, current.upstream429MaxRetries || 1)
                      : 0,
                  }))
                }
                aria-label={labels.upstream429RetryToggle}
              />
            </div>
            <SelectField
              className="mt-4"
              label={labels.upstream429RetryCount}
              name="groupUpstream429MaxRetries"
              value={String(Math.max(1, draft.upstream429MaxRetries || 1))}
              disabled={busy || !draft.upstream429RetryEnabled}
              options={[1, 2, 3, 4, 5].map((value) => ({
                value: String(value),
                label:
                  value === 1
                    ? labels.upstream429RetryCountOnce
                    : labels.upstream429RetryCountMany(value),
              }))}
              onValueChange={(value) =>
                setDraft((current) => ({
                  ...current,
                  upstream429MaxRetries: normalizeRetryCount(Number(value)),
                }))
              }
            />
          </div>

          {error ? <p className="text-sm text-error">{error}</p> : null}
          {!payload ? <p className="text-sm text-warning">{labels.validation}</p> : null}
        </div>
        <div className="border-t border-base-300/80 px-6 py-4">
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
