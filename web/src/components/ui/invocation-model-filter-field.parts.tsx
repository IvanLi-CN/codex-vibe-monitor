import type { ReactNode } from "react";
import { AppIcon } from "../../features/shared/AppIcon";
import type { TextInputAutocompleteOffProps } from "../../lib/form-autocomplete";
import { cn } from "../../lib/utils";
import { Chip } from "./chip";
import type { FilterableComboboxOption } from "./filterable-combobox";
import type { InvocationModelFilterFieldValue } from "./invocation-model-filter-field";
import { MultiValueSuggestionField } from "./multi-value-suggestion-field";
import { PopoverContent, PopoverTrigger } from "./popover";
import { SegmentedControl, SegmentedControlItem } from "./segmented-control";

export type InvocationModelFilterSummary = {
  visible: string[];
  hiddenCount: number;
};

export interface InvocationModelFilterFieldTriggerProps {
  labelId: string;
  summaryId: string;
  panelId: string;
  feedbackId?: string;
  testId?: string;
  disabled?: boolean;
  error?: string | null;
  open: boolean;
  selectedTargetLabel: string;
  selectedReroutedLabel: string;
  modelPlaceholder?: string;
  modelLabel: string;
  reasoningEffortLabel: string;
  visibleModels: InvocationModelFilterSummary;
  visibleReasoningEfforts: InvocationModelFilterSummary;
  triggerSummaryText: string;
}

export function InvocationModelFilterFieldTrigger({
  labelId,
  summaryId,
  panelId,
  feedbackId,
  testId,
  disabled,
  error,
  open,
  selectedTargetLabel,
  selectedReroutedLabel,
  modelPlaceholder,
  modelLabel,
  reasoningEffortLabel,
  visibleModels,
  visibleReasoningEfforts,
  triggerSummaryText,
}: InvocationModelFilterFieldTriggerProps) {
  return (
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
                  <Chip key={`model-${item}`} tone="secondary" className="max-w-full">
                    <span className="truncate">{item}</span>
                  </Chip>
                ))}
                {visibleModels.hiddenCount > 0 ? (
                  <Chip tone="secondary">+{visibleModels.hiddenCount}</Chip>
                ) : null}
              </>
            ) : (
              <span className="text-sm text-base-content/45">{modelPlaceholder || modelLabel}</span>
            )}
            {visibleReasoningEfforts.visible.map((item) => (
              <Chip key={`reasoning-${item}`} tone="info" className="max-w-full">
                <span className="truncate">
                  {reasoningEffortLabel} {item}
                </span>
              </Chip>
            ))}
            {visibleReasoningEfforts.hiddenCount > 0 ? (
              <Chip tone="info">
                {reasoningEffortLabel} +{visibleReasoningEfforts.hiddenCount}
              </Chip>
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
  );
}

interface InvocationModelFilterTargetsProps {
  value: InvocationModelFilterFieldValue;
  onChange: (value: InvocationModelFilterFieldValue) => void;
  modelTargetLabel: string;
  requestTargetLabel: string;
  responseTargetLabel: string;
  reroutedLabel: string;
  reroutedAllLabel: string;
  reroutedOnlyLabel: string;
  notReroutedLabel: string;
  disabled?: boolean;
  error?: string | null;
  feedbackId: string;
  testId?: string;
}

export function InvocationModelFilterTargets({
  value,
  onChange,
  modelTargetLabel,
  requestTargetLabel,
  responseTargetLabel,
  reroutedLabel,
  reroutedAllLabel,
  reroutedOnlyLabel,
  notReroutedLabel,
  disabled,
  error,
  feedbackId,
  testId,
}: InvocationModelFilterTargetsProps) {
  return (
    <div className="grid gap-4 [grid-template-columns:minmax(0,0.85fr)_minmax(0,1.35fr)]">
      <div className="min-w-0 space-y-2.5">
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
      <div className="min-w-0 space-y-2.5">
        <span className="field-label whitespace-nowrap">{reroutedLabel}</span>
        <SegmentedControl size="compact" className="w-full">
          {[
            ["all", reroutedAllLabel, "rerouted-all"],
            ["rerouted", reroutedOnlyLabel, "rerouted-only"],
            ["notRerouted", notReroutedLabel, "rerouted-not"],
          ].map(([target, optionLabel, testSuffix]) => (
            <SegmentedControlItem
              key={target}
              active={value.modelRerouted === target}
              className="flex-1"
              onClick={() =>
                onChange({
                  ...value,
                  modelRerouted: target as InvocationModelFilterFieldValue["modelRerouted"],
                })
              }
              aria-label={`${reroutedLabel}: ${optionLabel}`}
              aria-describedby={error ? feedbackId : undefined}
              aria-invalid={error ? true : undefined}
              disabled={disabled}
              data-testid={testId ? `${testId}-${testSuffix}` : undefined}
            >
              {optionLabel}
            </SegmentedControlItem>
          ))}
        </SegmentedControl>
      </div>
    </div>
  );
}

interface InvocationModelFilterSuggestionsProps {
  value: InvocationModelFilterFieldValue;
  onChange: (value: InvocationModelFilterFieldValue) => void;
  modelLabel: string;
  reasoningEffortLabel: string;
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
  addLabel: string;
  modelLoading?: boolean;
  reasoningEffortLoading?: boolean;
  disabled?: boolean;
  reasoningEffortDisabled: boolean;
  modelInputId?: string;
  reasoningEffortInputId?: string;
  onModelOpenChange?: (open: boolean) => void;
  onReasoningEffortOpenChange?: (open: boolean) => void;
  inputAutocompleteProps?: Partial<TextInputAutocompleteOffProps>;
}

export function InvocationModelFilterSuggestions({
  value,
  onChange,
  modelLabel,
  reasoningEffortLabel,
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
  addLabel,
  modelLoading,
  reasoningEffortLoading,
  disabled,
  reasoningEffortDisabled,
  modelInputId,
  reasoningEffortInputId,
  onModelOpenChange,
  onReasoningEffortOpenChange,
  inputAutocompleteProps,
}: InvocationModelFilterSuggestionsProps) {
  return (
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
  );
}

export interface InvocationModelFilterFieldPanelProps
  extends InvocationModelFilterTargetsProps,
    InvocationModelFilterSuggestionsProps {
  panelId: string;
  labelId: string;
  hint?: ReactNode;
}

export function InvocationModelFilterFieldPanel({
  panelId,
  labelId,
  hint,
  ...props
}: InvocationModelFilterFieldPanelProps) {
  return (
    <PopoverContent
      id={panelId}
      data-testid={props.testId ? `${props.testId}-panel` : undefined}
      aria-labelledby={labelId}
      align="end"
      sideOffset={8}
      collisionPadding={16}
      className="w-[min(max(var(--radix-popover-trigger-width),40rem),var(--radix-popover-content-available-width))] max-h-[min(36rem,var(--radix-popover-content-available-height))] overflow-visible rounded-2xl p-4"
    >
      <div className="space-y-4">
        {hint ? <p className="min-w-0 text-xs text-base-content/60">{hint}</p> : null}
        <InvocationModelFilterTargets {...props} />
        <InvocationModelFilterSuggestions {...props} />
      </div>
    </PopoverContent>
  );
}
