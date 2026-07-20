import { type KeyboardEvent, type ReactNode, useId, useMemo, useRef, useState } from "react";
import { AppIcon } from "../../features/shared/AppIcon";
import {
  resolveTextInputAutocompleteProps,
  type TextInputAutocompleteOffProps,
} from "../../lib/form-autocomplete";
import { cn } from "../../lib/utils";
import { Badge } from "./badge";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "./command";
import type { FilterableComboboxOption } from "./filterable-combobox";
import { FormFieldFeedback } from "./form-field-feedback";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";

interface MultiValueSuggestionFieldProps {
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

function isSearchableCandidate(candidate: string | undefined): candidate is string {
  return typeof candidate === "string" && candidate.trim().length > 0;
}

function focusWithoutScroll(element: HTMLInputElement | null) {
  element?.focus({ preventScroll: true });
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
      snapshots.push({
        element,
        left: element.scrollLeft,
        top: element.scrollTop,
      });
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

function hasValue(values: string[], candidate: string) {
  const normalizedCandidate = normalizeKey(candidate);
  if (!normalizedCandidate) return false;
  return values.some((value) => normalizeKey(value) === normalizedCandidate);
}

function resolveCommittedValue(rawValue: string, options: FilterableComboboxOption[]) {
  const normalizedRawValue = normalizeKey(rawValue);
  const matchingOption = options.find(
    (option) =>
      normalizeKey(option.value) === normalizedRawValue ||
      normalizeKey(getOptionDisplayValue(option)) === normalizedRawValue,
  );
  return matchingOption?.value.trim() || normalizeValue(rawValue);
}

export function MultiValueSuggestionField({
  label,
  inputLabel,
  values,
  onValuesChange,
  inputValue,
  onInputValueChange,
  options,
  placeholder,
  emptyText,
  loading,
  loadingText,
  disabled,
  error,
  surface = "default",
  className,
  inputClassName,
  listClassName,
  name,
  id,
  onOpenChange,
  addLabel = "Add",
  inputAutocompleteProps,
  testId,
}: MultiValueSuggestionFieldProps) {
  const feedbackId = useId();
  const fallbackInputId = useId();
  const isEmbedded = surface === "embedded";
  const [open, setOpen] = useState(false);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const resolvedInputAutocompleteProps = resolveTextInputAutocompleteProps(inputAutocompleteProps);
  const normalizedOptions = useMemo(
    () =>
      options.map(normalizeOption).filter((option, index, allOptions) => {
        const key = normalizeKey(option.value);
        return (
          key.length > 0 &&
          allOptions.findIndex((candidate) => normalizeKey(candidate.value) === key) === index
        );
      }),
    [options],
  );
  const optionLabelMap = useMemo(
    () =>
      new Map(
        normalizedOptions.map((option) => [
          normalizeKey(option.value),
          getOptionDisplayValue(option),
        ]),
      ),
    [normalizedOptions],
  );
  const trimmedInputValue = normalizeValue(inputValue);
  const committedInputValue = resolveCommittedValue(trimmedInputValue, normalizedOptions);
  const canAddInput = committedInputValue.length > 0 && !hasValue(values, committedInputValue);
  const selectedValueSet = useMemo(() => new Set(values.map(normalizeKey)), [values]);
  const filteredOptions = useMemo(() => {
    const query = normalizeKey(inputValue);
    if (!query) return normalizedOptions;
    return normalizedOptions.filter((option) =>
      [getOptionDisplayValue(option), option.value, option.searchText]
        .filter(isSearchableCandidate)
        .some((candidate) => candidate.toLowerCase().includes(query)),
    );
  }, [inputValue, normalizedOptions]);

  const addValue = (rawValue: string) => {
    const committedValue = resolveCommittedValue(rawValue, normalizedOptions);
    if (!committedValue || hasValue(values, committedValue)) {
      onInputValueChange("");
      return;
    }
    onValuesChange([...values, committedValue]);
    onInputValueChange("");
  };

  const removeValue = (targetValue: string) => {
    onValuesChange(values.filter((value) => normalizeKey(value) !== normalizeKey(targetValue)));
  };

  const toggleOption = (option: FilterableComboboxOption) => {
    if (selectedValueSet.has(normalizeKey(option.value))) {
      removeValue(option.value);
      return;
    }
    addValue(option.value);
  };

  const inputId = id ?? fallbackInputId;
  const scheduleScrollRestore = () => {
    const snapshots = captureScrollSnapshots();
    if (snapshots.length === 0) return;
    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => {
        restoreScrollSnapshots(snapshots);
      });
    });
  };
  const commitOpenState = (nextOpen: boolean) => {
    if (disabled) {
      setOpen(false);
      onOpenChange?.(false);
      return;
    }
    setOpen(nextOpen);
    onOpenChange?.(nextOpen);
    if (!nextOpen && inputValue) {
      onInputValueChange("");
    }
    if (isEmbedded && nextOpen) {
      scheduleScrollRestore();
    }
  };
  const focusSearchInput = () => {
    window.requestAnimationFrame(() => focusWithoutScroll(searchInputRef.current));
    if (isEmbedded) {
      scheduleScrollRestore();
    }
  };
  const triggerTitle =
    values.length > 0
      ? values
          .map((value) => optionLabelMap.get(normalizeKey(value)) ?? normalizeValue(value))
          .join(", ")
      : placeholder || inputLabel;
  const optionContent = (
    <CommandList id={`${inputId}-list`} className={cn(isEmbedded && "max-h-40", listClassName)}>
      {canAddInput ? (
        <>
          <CommandGroup>
            <CommandItem
              value={`${addLabel} ${committedInputValue}`}
              onSelect={() => {
                addValue(committedInputValue);
                focusSearchInput();
              }}
            >
              <AppIcon
                name="plus-circle-outline"
                className="mr-2 h-4 w-4 text-primary"
                aria-hidden
              />
              <span className="truncate">
                {addLabel} “{trimmedInputValue}”
              </span>
            </CommandItem>
          </CommandGroup>
          <CommandSeparator />
        </>
      ) : null}
      {loading ? (
        <div className="px-3 py-2 text-sm text-base-content/60">{loadingText ?? "Loading…"}</div>
      ) : filteredOptions.length === 0 ? (
        <CommandEmpty>{emptyText ?? "No matches"}</CommandEmpty>
      ) : (
        <CommandGroup>
          {filteredOptions.map((option) => {
            const active = selectedValueSet.has(normalizeKey(option.value));
            return (
              <CommandItem
                key={`${option.value}:${getOptionDisplayValue(option)}`}
                value={`${option.value} ${getOptionDisplayValue(option)} ${option.searchText ?? ""}`}
                disabled={disabled}
                onSelect={() => {
                  toggleOption(option);
                  focusSearchInput();
                }}
              >
                <AppIcon
                  name="check"
                  className={cn(
                    "mr-2 h-4 w-4 text-primary transition-opacity",
                    active ? "opacity-100" : "opacity-0",
                  )}
                  aria-hidden
                />
                <span className="truncate">{getOptionDisplayValue(option)}</span>
              </CommandItem>
            );
          })}
        </CommandGroup>
      )}
    </CommandList>
  );

  const handleEmbeddedInputKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      commitOpenState(false);
      return;
    }
    if (event.key === "Backspace" && inputValue.length === 0 && values.length > 0) {
      event.preventDefault();
      removeValue(values[values.length - 1]);
      return;
    }
    if (event.key === "ArrowDown" && !open) {
      event.preventDefault();
      commitOpenState(true);
    }
  };

  if (isEmbedded) {
    return (
      <div className={cn("field space-y-3", className)} data-testid={testId}>
        <FormFieldFeedback
          label={label}
          message={error}
          messageId={error ? feedbackId : undefined}
        />
        <Popover open={disabled ? false : open} onOpenChange={commitOpenState}>
          <PopoverTrigger asChild>
            <button
              type="button"
              role="combobox"
              aria-expanded={open}
              aria-label={inputLabel}
              aria-describedby={error ? feedbackId : undefined}
              aria-invalid={error ? true : undefined}
              aria-controls={open ? `${inputId}-list` : undefined}
              disabled={disabled}
              title={triggerTitle}
              onClick={(event) => {
                if (!open) return;
                event.preventDefault();
                commitOpenState(false);
              }}
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
              <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
                {values.length > 0 ? (
                  values.map((value) => {
                    const displayValue =
                      optionLabelMap.get(normalizeKey(value)) ?? normalizeValue(value);
                    return (
                      <Badge
                        key={normalizeKey(value)}
                        variant="secondary"
                        className="max-w-full rounded-full border border-base-300/80 bg-base-200/65 px-2.5 py-1 text-base-content"
                      >
                        <span className="truncate">{displayValue}</span>
                      </Badge>
                    );
                  })
                ) : (
                  <span className="text-sm text-base-content/55">{placeholder || inputLabel}</span>
                )}
              </span>
              <AppIcon
                name="chevron-down"
                className={cn(
                  "h-4 w-4 shrink-0 text-base-content/45 transition-transform",
                  open && "rotate-180",
                )}
                aria-hidden
              />
            </button>
          </PopoverTrigger>
          <PopoverContent
            align="start"
            sideOffset={6}
            collisionPadding={12}
            style={{ zIndex: 90 }}
            className="z-[60] w-[var(--radix-popover-trigger-width)] overflow-hidden rounded-xl border border-base-300/80 bg-base-100 p-0 shadow-lg"
            onOpenAutoFocus={(event) => {
              event.preventDefault();
              focusSearchInput();
            }}
          >
            <Command shouldFilter={false}>
              <CommandInput
                {...resolvedInputAutocompleteProps}
                ref={searchInputRef}
                id={inputId}
                name={name}
                aria-label={inputLabel}
                value={inputValue}
                placeholder={placeholder}
                onValueChange={onInputValueChange}
                onKeyDown={handleEmbeddedInputKeyDown}
                className={cn("w-full", inputClassName)}
              />
              {optionContent}
            </Command>
          </PopoverContent>
        </Popover>
      </div>
    );
  }

  return (
    <div className={cn("field", className)} data-testid={testId}>
      <FormFieldFeedback label={label} message={error} messageId={error ? feedbackId : undefined} />
      <div
        className={cn(
          isEmbedded
            ? "space-y-3"
            : "space-y-3 rounded-lg border border-base-300/80 bg-base-100 p-3",
          error && !isEmbedded && "border-error/70",
          disabled && "opacity-60",
        )}
      >
        <Popover open={disabled ? false : open} onOpenChange={commitOpenState}>
          <PopoverTrigger asChild>
            <button
              type="button"
              role="combobox"
              aria-expanded={open}
              aria-label={inputLabel}
              aria-describedby={error ? feedbackId : undefined}
              aria-invalid={error ? true : undefined}
              aria-controls={open ? `${inputId}-list` : undefined}
              disabled={disabled}
              title={triggerTitle}
              onClick={(event) => {
                if (!open) return;
                event.preventDefault();
                commitOpenState(false);
              }}
              className={cn(
                "flex min-h-11 w-full items-center gap-3 rounded-xl border border-base-300/80 bg-base-100 px-3 py-2.5 text-left shadow-sm transition-colors",
                "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100",
                "hover:border-primary/35",
                error && "border-error/70",
                disabled && "cursor-not-allowed opacity-60",
              )}
            >
              <AppIcon
                name="tag-outline"
                className="mt-0.5 h-4 w-4 shrink-0 text-base-content/55"
                aria-hidden
              />
              <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
                {values.length > 0 ? (
                  values.map((value) => {
                    const displayValue =
                      optionLabelMap.get(normalizeKey(value)) ?? normalizeValue(value);
                    return (
                      <Badge
                        key={normalizeKey(value)}
                        variant="secondary"
                        className="max-w-full rounded-full border border-base-300/80 bg-base-200/65 px-2.5 py-1 text-base-content"
                      >
                        <span className="truncate">{displayValue}</span>
                      </Badge>
                    );
                  })
                ) : (
                  <span className="text-sm text-base-content/55">{placeholder || inputLabel}</span>
                )}
              </span>
              <AppIcon
                name="chevron-down"
                className={cn(
                  "h-4 w-4 shrink-0 text-base-content/45 transition-transform",
                  open && "rotate-180",
                )}
                aria-hidden
              />
            </button>
          </PopoverTrigger>
          <PopoverContent
            align="start"
            className="w-[var(--radix-popover-trigger-width)] p-0"
            onOpenAutoFocus={(event) => {
              event.preventDefault();
              window.requestAnimationFrame(() => focusWithoutScroll(searchInputRef.current));
            }}
          >
            <Command shouldFilter={false}>
              <CommandInput
                {...resolvedInputAutocompleteProps}
                ref={searchInputRef}
                id={inputId}
                name={name}
                aria-label={inputLabel}
                value={inputValue}
                placeholder={placeholder}
                onValueChange={onInputValueChange}
                className={cn("w-full", inputClassName)}
              />
              {optionContent}
            </Command>
          </PopoverContent>
        </Popover>
      </div>
    </div>
  );
}
