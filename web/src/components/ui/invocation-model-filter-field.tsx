import { type ReactNode, useId } from "react";
import type { InvocationModelRerouteFilter, InvocationModelTarget } from "../../lib/api";
import type { TextInputAutocompleteOffProps } from "../../lib/form-autocomplete";
import { cn } from "../../lib/utils";
import type { FilterableComboboxOption } from "./filterable-combobox";
import { FormFieldFeedback } from "./form-field-feedback";
import {
  InvocationModelFilterFieldPanel,
  InvocationModelFilterFieldTrigger,
} from "./invocation-model-filter-field.parts";
import { useInvocationModelFilterFieldState } from "./invocation-model-filter-field.state";
import { Popover } from "./popover";

export interface InvocationModelFilterFieldValue {
  modelTarget: InvocationModelTarget;
  modelRerouted: InvocationModelRerouteFilter;
  models: string[];
  reasoningEfforts: string[];
}

export interface InvocationModelFilterFieldProps {
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

type InvocationModelFilterFieldViewProps = InvocationModelFilterFieldProps &
  ReturnType<typeof useInvocationModelFilterFieldState>;

export function InvocationModelFilterField(props: InvocationModelFilterFieldProps) {
  const fieldState = useInvocationModelFilterFieldState(props);
  return <InvocationModelFilterFieldView {...props} {...fieldState} />;
}

function InvocationModelFilterFieldView(props: InvocationModelFilterFieldViewProps) {
  const labelId = useId();
  const summaryId = useId();
  const feedbackId = useId();
  const panelId = useId();

  return (
    <div className={cn("field", props.className)} data-testid={props.testId}>
      <FormFieldFeedback
        label={props.label}
        labelId={labelId}
        message={props.error}
        messageId={props.error ? feedbackId : undefined}
      />
      <Popover open={props.disabled ? false : props.open} onOpenChange={props.commitOpenState}>
        <InvocationModelFilterFieldTrigger
          {...props}
          labelId={labelId}
          summaryId={summaryId}
          panelId={panelId}
          feedbackId={feedbackId}
        />
        <InvocationModelFilterFieldPanel
          {...props}
          panelId={panelId}
          labelId={labelId}
          hint={props.hint}
          feedbackId={feedbackId}
          addLabel={props.addLabel ?? "Add"}
        />
      </Popover>
    </div>
  );
}
