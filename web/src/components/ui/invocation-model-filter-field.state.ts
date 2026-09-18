import { useMemo, useState } from "react";
import type { FilterableComboboxOption } from "./filterable-combobox";
import type { InvocationModelFilterFieldProps } from "./invocation-model-filter-field";
import type { InvocationModelFilterSummary } from "./invocation-model-filter-field.parts";

function normalizeOption(option: string | FilterableComboboxOption): FilterableComboboxOption {
  return typeof option === "string" ? { value: option, label: option } : option;
}

function normalizeKey(value: string) {
  return value.trim().toLowerCase();
}

function optionLabel(option: FilterableComboboxOption) {
  return option.label?.trim() || option.value.trim();
}

function buildOptionLabelMap(options: Array<string | FilterableComboboxOption>) {
  return new Map(
    options
      .map(normalizeOption)
      .map((option) => [normalizeKey(option.value), optionLabel(option)] as const)
      .filter(([key, label]) => key.length > 0 && label.length > 0),
  );
}

function resolveLabels(values: string[], labels: Map<string, string>) {
  return values
    .map((value) => labels.get(normalizeKey(value)) ?? value.trim())
    .filter((value) => value.length > 0);
}

function summarize(labels: string[], limit = 2): InvocationModelFilterSummary {
  return { visible: labels.slice(0, limit), hiddenCount: Math.max(0, labels.length - limit) };
}

export function useInvocationModelFilterFieldState({
  value,
  modelOptions,
  reasoningEffortOptions,
  disabled,
  modelTargetLabel,
  requestTargetLabel,
  responseTargetLabel,
  reroutedLabel,
  reroutedAllLabel,
  reroutedOnlyLabel,
  notReroutedLabel,
  modelPlaceholder,
  modelLabel,
  reasoningEffortLabel,
  onModelOpenChange,
  onReasoningEffortOpenChange,
}: Pick<
  InvocationModelFilterFieldProps,
  | "value"
  | "modelOptions"
  | "reasoningEffortOptions"
  | "disabled"
  | "modelTargetLabel"
  | "requestTargetLabel"
  | "responseTargetLabel"
  | "reroutedLabel"
  | "reroutedAllLabel"
  | "reroutedOnlyLabel"
  | "notReroutedLabel"
  | "modelPlaceholder"
  | "modelLabel"
  | "reasoningEffortLabel"
  | "onModelOpenChange"
  | "onReasoningEffortOpenChange"
>) {
  const [open, setOpen] = useState(false);
  const modelLabels = useMemo(
    () => resolveLabels(value.models, buildOptionLabelMap(modelOptions)),
    [modelOptions, value.models],
  );
  const reasoningLabels = useMemo(
    () => resolveLabels(value.reasoningEfforts, buildOptionLabelMap(reasoningEffortOptions)),
    [reasoningEffortOptions, value.reasoningEfforts],
  );
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
    reasoningLabels.length > 0 ? `${reasoningEffortLabel}: ${reasoningLabels.join(", ")}` : null,
  ]
    .filter((entry): entry is string => Boolean(entry))
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
  return {
    open,
    commitOpenState,
    reasoningEffortDisabled: disabled || value.models.length === 0,
    selectedTargetLabel,
    selectedReroutedLabel,
    triggerSummaryText,
    visibleModels: summarize(modelLabels),
    visibleReasoningEfforts: summarize(reasoningLabels),
  };
}
