import { type KeyboardEvent, useId, useMemo, useRef, useState } from "react";
import {
  resolveTextInputAutocompleteProps,
  type TextInputAutocompleteOffProps,
} from "../../lib/form-autocomplete";
import type { FilterableComboboxOption } from "./filterable-combobox";
import type { MultiValueSuggestionFieldProps } from "./multi-value-suggestion-field";

export function normalizeMultiValueKey(value: string) {
  return value.trim().toLowerCase();
}

export function getMultiValueOptionDisplayValue(option: FilterableComboboxOption) {
  return option.label?.trim() || option.value.trim();
}

function normalizeOption(option: string | FilterableComboboxOption): FilterableComboboxOption {
  return typeof option === "string" ? { value: option, label: option } : option;
}

function normalizeValue(value: string) {
  return value.trim();
}

function isSearchableCandidate(candidate: string | undefined): candidate is string {
  return typeof candidate === "string" && candidate.trim().length > 0;
}

function resolveCommittedValue(rawValue: string, options: FilterableComboboxOption[]) {
  const normalizedRawValue = normalizeMultiValueKey(rawValue);
  const matchingOption = options.find(
    (option) =>
      normalizeMultiValueKey(option.value) === normalizedRawValue ||
      normalizeMultiValueKey(getMultiValueOptionDisplayValue(option)) === normalizedRawValue,
  );
  return matchingOption?.value.trim() || normalizeValue(rawValue);
}

interface ScrollSnapshot {
  element: HTMLElement;
  left: number;
  top: number;
}

function captureScrollSnapshots(): ScrollSnapshot[] {
  if (typeof document === "undefined") return [];

  const snapshots: ScrollSnapshot[] = [];
  const seen = new Set<HTMLElement>();
  document.querySelectorAll<HTMLElement>("*").forEach((element) => {
    if (
      (element.scrollHeight > element.clientHeight || element.scrollWidth > element.clientWidth) &&
      !seen.has(element)
    ) {
      snapshots.push({ element, left: element.scrollLeft, top: element.scrollTop });
      seen.add(element);
    }
  });

  const scrollingElement = document.scrollingElement;
  if (scrollingElement instanceof HTMLElement && !seen.has(scrollingElement)) {
    snapshots.push({
      element: scrollingElement,
      left: scrollingElement.scrollLeft,
      top: scrollingElement.scrollTop,
    });
  }

  return snapshots;
}

function restoreScrollSnapshots(snapshots: ScrollSnapshot[]) {
  snapshots.forEach(({ element, left, top }) => {
    element.scrollLeft = left;
    element.scrollTop = top;
  });
}

function scheduleScrollRestore() {
  const snapshots = captureScrollSnapshots();
  if (snapshots.length === 0) return;
  window.requestAnimationFrame(() => {
    window.requestAnimationFrame(() => restoreScrollSnapshots(snapshots));
  });
}

function focusWithoutScroll(element: HTMLInputElement | null) {
  element?.focus({ preventScroll: true });
}

function useFieldOpenState(
  props: MultiValueSuggestionFieldProps,
  isEmbedded: boolean,
  inputValue: string,
) {
  const [open, setOpen] = useState(false);
  const commitOpenState = (nextOpen: boolean) => {
    if (props.disabled) {
      setOpen(false);
      props.onOpenChange?.(false);
      return;
    }
    setOpen(nextOpen);
    props.onOpenChange?.(nextOpen);
    if (!nextOpen && inputValue) props.onInputValueChange("");
    if (isEmbedded && nextOpen) scheduleScrollRestore();
  };
  return { open, commitOpenState };
}

interface FieldActionInputs {
  props: MultiValueSuggestionFieldProps;
  normalizedOptions: FilterableComboboxOption[];
  selectedValueSet: Set<string>;
  searchInputRef: React.RefObject<HTMLInputElement | null>;
  open: boolean;
  commitOpenState: (nextOpen: boolean) => void;
  isEmbedded: boolean;
}

function useFieldActions({
  props,
  normalizedOptions,
  selectedValueSet,
  searchInputRef,
  open,
  commitOpenState,
  isEmbedded,
}: FieldActionInputs) {
  const addValue = (rawValue: string) => {
    const committedValue = resolveCommittedValue(rawValue, normalizedOptions);
    if (!committedValue || selectedValueSet.has(normalizeMultiValueKey(committedValue))) {
      props.onInputValueChange("");
      return;
    }
    props.onValuesChange([...props.values, committedValue]);
    props.onInputValueChange("");
  };
  const removeValue = (targetValue: string) => {
    props.onValuesChange(
      props.values.filter(
        (value) => normalizeMultiValueKey(value) !== normalizeMultiValueKey(targetValue),
      ),
    );
  };
  const toggleOption = (option: FilterableComboboxOption) => {
    if (selectedValueSet.has(normalizeMultiValueKey(option.value))) {
      removeValue(option.value);
      return;
    }
    addValue(option.value);
  };
  const focusSearchInput = () => {
    window.requestAnimationFrame(() => focusWithoutScroll(searchInputRef.current));
    if (isEmbedded) scheduleScrollRestore();
  };
  const handleEmbeddedInputKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      commitOpenState(false);
      return;
    }
    if (event.key === "Backspace" && props.inputValue.length === 0 && props.values.length > 0) {
      event.preventDefault();
      removeValue(props.values[props.values.length - 1]);
      return;
    }
    if (event.key === "ArrowDown" && !open) {
      event.preventDefault();
      commitOpenState(true);
    }
  };
  return { addValue, removeValue, toggleOption, focusSearchInput, handleEmbeddedInputKeyDown };
}

export interface MultiValueSuggestionFieldState {
  feedbackId: string;
  inputId: string;
  isEmbedded: boolean;
  open: boolean;
  searchInputRef: React.RefObject<HTMLInputElement | null>;
  resolvedInputAutocompleteProps: Partial<TextInputAutocompleteOffProps>;
  normalizedOptions: FilterableComboboxOption[];
  optionLabelMap: Map<string, string>;
  trimmedInputValue: string;
  committedInputValue: string;
  canAddInput: boolean;
  selectedValueSet: Set<string>;
  filteredOptions: FilterableComboboxOption[];
  triggerTitle: string;
  commitOpenState: (nextOpen: boolean) => void;
  addValue: (rawValue: string) => void;
  removeValue: (targetValue: string) => void;
  toggleOption: (option: FilterableComboboxOption) => void;
  focusSearchInput: () => void;
  handleEmbeddedInputKeyDown: (event: KeyboardEvent<HTMLInputElement>) => void;
}

export function useMultiValueSuggestionFieldState(
  props: MultiValueSuggestionFieldProps,
): MultiValueSuggestionFieldState {
  const feedbackId = useId();
  const fallbackInputId = useId();
  const isEmbedded = props.surface === "embedded";
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const resolvedInputAutocompleteProps = resolveTextInputAutocompleteProps(
    props.inputAutocompleteProps,
  );
  const normalizedOptions = useMemo(
    () =>
      props.options.map(normalizeOption).filter((option, index, allOptions) => {
        const key = normalizeMultiValueKey(option.value);
        return (
          key.length > 0 &&
          allOptions.findIndex((candidate) => normalizeMultiValueKey(candidate.value) === key) ===
            index
        );
      }),
    [props.options],
  );
  const optionLabelMap = useMemo(
    () =>
      new Map(
        normalizedOptions.map((option) => [
          normalizeMultiValueKey(option.value),
          getMultiValueOptionDisplayValue(option),
        ]),
      ),
    [normalizedOptions],
  );
  const trimmedInputValue = normalizeValue(props.inputValue);
  const committedInputValue = resolveCommittedValue(trimmedInputValue, normalizedOptions);
  const canAddInput =
    committedInputValue.length > 0 &&
    !props.values.some(
      (value) => normalizeMultiValueKey(value) === normalizeMultiValueKey(committedInputValue),
    );
  const selectedValueSet = useMemo(
    () => new Set(props.values.map(normalizeMultiValueKey)),
    [props.values],
  );
  const filteredOptions = useMemo(() => {
    const query = normalizeMultiValueKey(props.inputValue);
    if (!query) return normalizedOptions;
    return normalizedOptions.filter((option) =>
      [getMultiValueOptionDisplayValue(option), option.value, option.searchText]
        .filter(isSearchableCandidate)
        .some((candidate) => candidate.toLowerCase().includes(query)),
    );
  }, [props.inputValue, normalizedOptions]);
  const { open, commitOpenState } = useFieldOpenState(props, isEmbedded, props.inputValue);
  const actions = useFieldActions({
    props,
    normalizedOptions,
    selectedValueSet,
    searchInputRef,
    open,
    commitOpenState,
    isEmbedded,
  });
  const inputId = props.id ?? fallbackInputId;
  const triggerTitle =
    props.values.length > 0
      ? props.values
          .map(
            (value) => optionLabelMap.get(normalizeMultiValueKey(value)) ?? normalizeValue(value),
          )
          .join(", ")
      : props.placeholder || props.inputLabel;

  return {
    feedbackId,
    inputId,
    isEmbedded,
    open,
    searchInputRef,
    resolvedInputAutocompleteProps,
    normalizedOptions,
    optionLabelMap,
    trimmedInputValue,
    committedInputValue,
    canAddInput,
    selectedValueSet,
    filteredOptions,
    triggerTitle,
    commitOpenState,
    ...actions,
  };
}
