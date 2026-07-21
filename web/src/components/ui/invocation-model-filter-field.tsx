import { type ReactNode, useId, useMemo, useState } from "react";
import { AppIcon } from "../../features/shared/AppIcon";
import type { InvocationModelRerouteFilter, InvocationModelTarget } from "../../lib/api";
import type { TextInputAutocompleteOffProps } from "../../lib/form-autocomplete";
import { cn } from "../../lib/utils";
import { Badge } from "./badge";
import type { FilterableComboboxOption } from "./filterable-combobox";
import { FormFieldFeedback } from "./form-field-feedback";
import { MultiValueSuggestionField } from "./multi-value-suggestion-field";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";
import { SegmentedControl, SegmentedControlItem } from "./segmented-control";

export interface InvocationModelFilterFieldValue {
  modelTarget: InvocationModelTarget;
  modelRerouted: InvocationModelRerouteFilter;
  models: string[];
  reasoningEfforts: string[];
}

interface InvocationModelFilterFieldProps {
  label: ReactNode;
  hint?: ReactNode;
  value: InvocationModelFilterFieldValue;
  onChange: (value: InvocationModelFilterFieldValue) => void;
  modelLabel: string;
  reasoningEffortLabel: string;
  modelTargetLabel: string;
  requestTargetLabel: string;
  responseTargetLabel: string;
  reroutedLabel: string;
  reroutedAllLabel: string;
  reroutedOnlyLabel: string;
  notReroutedLabel: string;
  modelInputValue: string;
  onModelInputValueChange: (value: string) => void;
  modelOptions: Array<string | FilterableComboboxOption>;
  modelPlaceholder?: string;
  reasoningEffortInputValue: string;
  onReasoningEffortInputValueChange: (value: string) => void;
  reasoningEffortOptions: Array<string | FilterableComboboxOption>;
  reasoningEffortPlaceholder?: string;
  emptyText?: string;
  loadingText?: string;
  addLabel?: string;
  modelLoading?: boolean;
  reasoningEffortLoading?: boolean;
  disabled?: boolean;
  error?: string | null;
  className?: string;
  modelInputId?: string;
  reasoningEffortInputId?: string;
  onModelOpenChange?: (open: boolean) => void;
  onReasoningEffortOpenChange?: (open: boolean) => void;
  inputAutocompleteProps?: Partial<TextInputAutocompleteOffProps>;
  testId?: string;
}

function normalizeOption(option: string | FilterableComboboxOption): FilterableComboboxOption {
  return typeof option === "string" ? { value: option, label: option } : option;
}

function normalizeValue(value: string) {
  return value.trim();
}

function normalizeKey(value: string) {
  return normalizeValue(value).toLowerCase();
}

function getOptionDisplayValue(option: FilterableComboboxOption) {
  return option.label?.trim() || option.value.trim();
}

function buildOptionLabelMap(options: Array<string | FilterableComboboxOption>) {
  return new Map(
    options
      .map(normalizeOption)
      .map((option) => [normalizeKey(option.value), getOptionDisplayValue(option)] as const)
      .filter(([key, label]) => key.length > 0 && label.length > 0),
  );
}

function resolveDisplayLabels(values: string[], optionLabelMap: Map<string, string>) {
  return values
    .map((value) => optionLabelMap.get(normalizeKey(value)) ?? normalizeValue(value))
    .filter((value) => value.length > 0);
}

function summarizeLabels(labels: string[], limit = 2) {
  return {
    visible: labels.slice(0, limit),
    hiddenCount: Math.max(0, labels.length - limit),
  };
}

export function InvocationModelFilterField({
  label,
  hint,
  value,
  onChange,
  modelLabel,
  reasoningEffortLabel,
  modelTargetLabel,
  requestTargetLabel,
  responseTargetLabel,
  reroutedLabel,
  reroutedAllLabel,
  reroutedOnlyLabel,
  notReroutedLabel,
  modelInputValue,
  onModelInputValueChange,
  modelOptions,
  modelPlaceholder,
  reasoningEffortInputValue,
  onReasoningEffortInputValueChange,
  reasoningEffortOptions,
  reasoningEffortPlaceholder,
  emptyText,
  loadingText,
  addLabel = "Add",
  modelLoading,
  reasoningEffortLoading,
  disabled,
  error,
  className,
  modelInputId,
  reasoningEffortInputId,
  onModelOpenChange,
  onReasoningEffortOpenChange,
  inputAutocompleteProps,
  testId,
}: InvocationModelFilterFieldProps) {
  const [open, setOpen] = useState(false);
  const labelId = useId();
  const summaryId = useId();
  const feedbackId = useId();
  const panelId = useId();
  const reasoningEffortDisabled = disabled || value.models.length === 0;
  const modelOptionLabelMap = useMemo(() => buildOptionLabelMap(modelOptions), [modelOptions]);
  const reasoningEffortOptionLabelMap = useMemo(
    () => buildOptionLabelMap(reasoningEffortOptions),
    [reasoningEffortOptions],
  );
  const modelLabels = useMemo(
    () => resolveDisplayLabels(value.models, modelOptionLabelMap),
    [modelOptionLabelMap, value.models],
  );
  const reasoningEffortLabels = useMemo(
    () => resolveDisplayLabels(value.reasoningEfforts, reasoningEffortOptionLabelMap),
    [reasoningEffortOptionLabelMap, value.reasoningEfforts],
  );
  const visibleModels = summarizeLabels(modelLabels);
  const visibleReasoningEfforts = summarizeLabels(reasoningEffortLabels);
  const selectedTargetLabel =
    value.modelTarget === "response" ? responseTargetLabel : requestTargetLabel;
  const selectedReroutedLabel =
    value.modelRerouted === "rerouted"
      ? reroutedOnlyLabel
      : value.modelRerouted === "notRerouted"
        ? notReroutedLabel
        : reroutedAllLabel;
  const triggerSummaryText = [
    `${modelTargetLabel}: ${selectedTargetLabel}`,
    `${reroutedLabel}: ${selectedReroutedLabel}`,
    modelLabels.length > 0 ? modelLabels.join(", ") : modelPlaceholder || modelLabel,
    reasoningEffortLabels.length > 0
      ? `${reasoningEffortLabel}: ${reasoningEffortLabels.join(", ")}`
      : null,
  ]
    .filter((value): value is string => Boolean(value))
    .join(" · ");
  const commitOpenState = (nextOpen: boolean) => {
    if (disabled) {
      setOpen(false);
      onModelOpenChange?.(false);
      onReasoningEffortOpenChange?.(false);
      return;
    }
    setOpen(nextOpen);
    if (!nextOpen) {
      onModelOpenChange?.(false);
      onReasoningEffortOpenChange?.(false);
    }
  };

  return (
    <div className={cn("field", className)} data-testid={testId}>
      <FormFieldFeedback
        label={label}
        labelId={labelId}
        message={error}
        messageId={error ? feedbackId : undefined}
      />
      <Popover open={disabled ? false : open} onOpenChange={commitOpenState}>
        <PopoverTrigger asChild>
          <button
            type="button"
            data-testid={testId ? `${testId}-trigger` : undefined}
            aria-haspopup="dialog"
            aria-expanded={open}
            aria-controls={open ? panelId : undefined}
            aria-labelledby={`${labelId} ${summaryId}`}
            aria-describedby={error ? feedbackId : undefined}
            aria-invalid={error ? true : undefined}
            disabled={disabled}
            className={cn(
              "flex min-h-11 w-full items-center gap-3 rounded-xl border border-base-300/80 bg-base-100 px-3 py-2.5 text-left shadow-sm transition-colors",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100",
              "hover:border-primary/35",
              open && "border-primary/45 ring-2 ring-primary/15",
              error && "border-error/70",
              disabled && "cursor-not-allowed opacity-60",
            )}
          >
            <AppIcon
              name="tag-outline"
              className="mt-0.5 h-4 w-4 shrink-0 text-base-content/55"
              aria-hidden
            />
            <span className="flex min-w-0 flex-1 flex-col gap-2">
              <span className="text-[11px] font-semibold uppercase tracking-[0.14em] text-base-content/45">
                {`${selectedTargetLabel} · ${selectedReroutedLabel}`}
              </span>
              <span
                id={summaryId}
                className="flex min-w-0 flex-wrap items-center gap-2"
                title={triggerSummaryText}
              >
                {visibleModels.visible.length > 0 ? (
                  <>
                    {visibleModels.visible.map((item) => (
                      <Badge key={`model-${item}`} variant="secondary" className="max-w-full">
                        <span className="truncate">{item}</span>
                      </Badge>
                    ))}
                    {visibleModels.hiddenCount > 0 ? (
                      <Badge variant="secondary">+{visibleModels.hiddenCount}</Badge>
                    ) : null}
                  </>
                ) : (
                  <span className="text-sm text-base-content/45">
                    {modelPlaceholder || modelLabel}
                  </span>
                )}
                {visibleReasoningEfforts.visible.map((item) => (
                  <Badge key={`reasoning-${item}`} variant="info" className="max-w-full">
                    <span className="truncate">
                      {reasoningEffortLabel} {item}
                    </span>
                  </Badge>
                ))}
                {visibleReasoningEfforts.hiddenCount > 0 ? (
                  <Badge variant="info">
                    {reasoningEffortLabel} +{visibleReasoningEfforts.hiddenCount}
                  </Badge>
                ) : null}
              </span>
            </span>
            <AppIcon
              name="chevron-down"
              className={cn(
                "h-4 w-4 shrink-0 text-base-content/55 transition-transform",
                open && "rotate-180",
              )}
              aria-hidden
            />
          </button>
        </PopoverTrigger>
        <PopoverContent
          id={panelId}
          data-testid={testId ? `${testId}-panel` : undefined}
          aria-labelledby={labelId}
          align="end"
          sideOffset={8}
          collisionPadding={16}
          className="w-[min(max(var(--radix-popover-trigger-width),40rem),var(--radix-popover-content-available-width))] max-h-[min(36rem,var(--radix-popover-content-available-height))] overflow-visible rounded-2xl p-4"
        >
          <div className="space-y-4">
            {hint ? <p className="min-w-0 text-xs text-base-content/60">{hint}</p> : null}

            <div className="grid gap-4 [grid-template-columns:minmax(0,0.85fr)_minmax(0,1.35fr)]">
              <div className="space-y-2.5 min-w-0">
                <span className="field-label whitespace-nowrap">{modelTargetLabel}</span>
                <SegmentedControl size="compact" className="w-full">
                  <SegmentedControlItem
                    active={value.modelTarget === "request"}
                    className="flex-1"
                    onClick={() => onChange({ ...value, modelTarget: "request" })}
                    disabled={disabled}
                    data-testid={testId ? `${testId}-target-request` : undefined}
                  >
                    {requestTargetLabel}
                  </SegmentedControlItem>
                  <SegmentedControlItem
                    active={value.modelTarget === "response"}
                    className="flex-1"
                    onClick={() => onChange({ ...value, modelTarget: "response" })}
                    disabled={disabled}
                    data-testid={testId ? `${testId}-target-response` : undefined}
                  >
                    {responseTargetLabel}
                  </SegmentedControlItem>
                </SegmentedControl>
              </div>

              <div className="space-y-2.5 min-w-0">
                <span className="field-label whitespace-nowrap">{reroutedLabel}</span>
                <SegmentedControl size="compact" className="w-full">
                  <SegmentedControlItem
                    active={value.modelRerouted === "all"}
                    className="flex-1"
                    onClick={() => onChange({ ...value, modelRerouted: "all" })}
                    aria-label={`${reroutedLabel}: ${reroutedAllLabel}`}
                    aria-describedby={error ? feedbackId : undefined}
                    aria-invalid={error ? true : undefined}
                    disabled={disabled}
                    data-testid={testId ? `${testId}-rerouted-all` : undefined}
                  >
                    {reroutedAllLabel}
                  </SegmentedControlItem>
                  <SegmentedControlItem
                    active={value.modelRerouted === "rerouted"}
                    className="flex-1"
                    onClick={() => onChange({ ...value, modelRerouted: "rerouted" })}
                    aria-label={`${reroutedLabel}: ${reroutedOnlyLabel}`}
                    aria-describedby={error ? feedbackId : undefined}
                    aria-invalid={error ? true : undefined}
                    disabled={disabled}
                    data-testid={testId ? `${testId}-rerouted-only` : undefined}
                  >
                    {reroutedOnlyLabel}
                  </SegmentedControlItem>
                  <SegmentedControlItem
                    active={value.modelRerouted === "notRerouted"}
                    className="flex-1"
                    onClick={() => onChange({ ...value, modelRerouted: "notRerouted" })}
                    aria-label={`${reroutedLabel}: ${notReroutedLabel}`}
                    aria-describedby={error ? feedbackId : undefined}
                    aria-invalid={error ? true : undefined}
                    disabled={disabled}
                    data-testid={testId ? `${testId}-rerouted-not` : undefined}
                  >
                    {notReroutedLabel}
                  </SegmentedControlItem>
                </SegmentedControl>
              </div>
            </div>

            <div className="grid gap-4 [grid-template-columns:minmax(0,1.25fr)_minmax(0,1fr)]">
              <MultiValueSuggestionField
                surface="embedded"
                label={modelLabel}
                inputLabel={modelLabel}
                id={modelInputId}
                values={value.models}
                onValuesChange={(models) => onChange({ ...value, models })}
                inputValue={modelInputValue}
                onInputValueChange={onModelInputValueChange}
                options={modelOptions}
                placeholder={modelPlaceholder}
                emptyText={emptyText}
                loading={modelLoading}
                loadingText={loadingText}
                disabled={disabled}
                onOpenChange={onModelOpenChange}
                addLabel={addLabel}
                inputAutocompleteProps={inputAutocompleteProps}
              />
              <MultiValueSuggestionField
                surface="embedded"
                label={reasoningEffortLabel}
                inputLabel={reasoningEffortLabel}
                id={reasoningEffortInputId}
                values={value.reasoningEfforts}
                onValuesChange={(reasoningEfforts) => onChange({ ...value, reasoningEfforts })}
                inputValue={reasoningEffortInputValue}
                onInputValueChange={onReasoningEffortInputValueChange}
                options={reasoningEffortOptions}
                placeholder={reasoningEffortPlaceholder}
                emptyText={emptyText}
                loading={reasoningEffortLoading}
                loadingText={loadingText}
                disabled={reasoningEffortDisabled}
                onOpenChange={onReasoningEffortOpenChange}
                addLabel={addLabel}
                inputAutocompleteProps={inputAutocompleteProps}
              />
            </div>
          </div>
        </PopoverContent>
      </Popover>
    </div>
  );
}
