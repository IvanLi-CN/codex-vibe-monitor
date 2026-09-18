import type { ReactNode } from "react";
import type { TextInputAutocompleteOffProps } from "../../lib/form-autocomplete";
import type { FilterableComboboxOption } from "./filterable-combobox";
import { MultiValueSuggestionFieldView } from "./multi-value-suggestion-field.parts";
import {
  type MultiValueSuggestionFieldState,
  useMultiValueSuggestionFieldState,
} from "./multi-value-suggestion-field.state";

export interface MultiValueSuggestionFieldProps {
  label: ReactNode;
  inputLabel: string;
  values: string[];
  onValuesChange: (values: string[]) => void;
  inputValue: string;
  onInputValueChange: (value: string) => void;
  options: Array<string | FilterableComboboxOption>;
  placeholder?: string;
  emptyText?: string;
  loading?: boolean;
  loadingText?: string;
  disabled?: boolean;
  error?: string | null;
  surface?: "default" | "embedded";
  className?: string;
  inputClassName?: string;
  listClassName?: string;
  name?: string;
  id?: string;
  onOpenChange?: (open: boolean) => void;
  addLabel?: string;
  inputAutocompleteProps?: Partial<TextInputAutocompleteOffProps>;
  testId?: string;
}

export function MultiValueSuggestionField(props: MultiValueSuggestionFieldProps) {
  const fieldState: MultiValueSuggestionFieldState = useMultiValueSuggestionFieldState(props);
  return <MultiValueSuggestionFieldView {...props} {...fieldState} />;
}
