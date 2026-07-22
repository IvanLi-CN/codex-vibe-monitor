import { type KeyboardEvent, useEffect, useId, useMemo, useRef, useState } from "react";
import { AppIcon } from "../../features/shared/AppIcon";
import {
  resolveTextInputAutocompleteProps,
  type TextInputAutocompleteOffProps,
} from "../../lib/form-autocomplete";
import { cn } from "../../lib/utils";

export interface FilterableComboboxOption {
  value: string;
  label?: string;
  searchText?: string;
}

interface FilterableComboboxProps {
  value: string;
  onValueChange: (value: string) => void;
  options: Array<string | FilterableComboboxOption>;
  placeholder?: string;
  emptyText?: string;
  loading?: boolean;
  loadingText?: string;
  disabled?: boolean;
  className?: string;
  inputClassName?: string;
  listClassName?: string;
  label: string;
  name?: string;
  id?: string;
  onOpenChange?: (open: boolean) => void;
  onOptionSelect?: (option: FilterableComboboxOption) => void;
  inputAutocompleteProps?: Partial<TextInputAutocompleteOffProps>;
}

function normalizeOption(option: string | FilterableComboboxOption): FilterableComboboxOption {
  return typeof option === "string" ? { value: option, label: option } : option;
}

function getOptionDisplayValue(option: FilterableComboboxOption) {
  return option.label?.trim() || option.value;
}

function isSearchableCandidate(candidate: string | undefined): candidate is string {
  return typeof candidate === "string" && candidate.trim().length > 0;
}

export function FilterableCombobox({
  value,
  onValueChange,
  options,
  placeholder,
  emptyText,
  loading,
  loadingText,
  disabled,
  className,
  inputClassName,
  listClassName,
  label,
  name,
  id,
  onOpenChange,
  onOptionSelect,
  inputAutocompleteProps,
}: FilterableComboboxProps) {
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const fallbackInputId = useId();
  const listId = useId();
  const inputId = id ?? fallbackInputId;
  const resolvedInputAutocompleteProps = resolveTextInputAutocompleteProps(inputAutocompleteProps);

  useEffect(() => {
    onOpenChange?.(open);
  }, [onOpenChange, open]);

  useEffect(() => {
    if (!open) return;
    const handlePointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("pointerdown", handlePointerDown);
    return () => document.removeEventListener("pointerdown", handlePointerDown);
  }, [open]);

  const normalizedOptions = useMemo(() => options.map(normalizeOption), [options]);

  const filteredOptions = useMemo(() => {
    const query = value.trim().toLowerCase();
    if (!query) return normalizedOptions;
    return normalizedOptions.filter((option) =>
      [getOptionDisplayValue(option), option.value, option.searchText]
        .filter(isSearchableCandidate)
        .some((candidate) => candidate.toLowerCase().includes(query)),
    );
  }, [normalizedOptions, value]);

  useEffect(() => {
    if (!open) return;
    setActiveIndex(filteredOptions.length > 0 ? 0 : -1);
  }, [filteredOptions, open]);

  const selectOption = (option: FilterableComboboxOption) => {
    onValueChange(getOptionDisplayValue(option));
    onOptionSelect?.(option);
    setOpen(false);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setOpen(true);
      setActiveIndex((current) => Math.min(filteredOptions.length - 1, Math.max(0, current + 1)));
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setOpen(true);
      setActiveIndex((current) => Math.max(0, current - 1));
      return;
    }
    if (event.key === "Enter") {
      if (!open) return;
      event.preventDefault();
      const next = filteredOptions[activeIndex];
      if (next) {
        selectOption(next);
      }
      return;
    }
    if (event.key === "Escape") {
      setOpen(false);
    }
  };

  return (
    <div ref={rootRef} className={cn("relative", className)}>
      <input
        {...resolvedInputAutocompleteProps}
        role="combobox"
        id={inputId}
        name={name}
        aria-label={label}
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        className={cn("pr-9", inputClassName)}
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        onChange={(event) => onValueChange(event.target.value)}
        onFocus={() => setOpen(true)}
        onClick={() => setOpen(true)}
        onBlur={() => {
          window.setTimeout(() => {
            if (!rootRef.current?.contains(document.activeElement)) {
              setOpen(false);
            }
          }, 0);
        }}
        onKeyDown={handleKeyDown}
      />
      <button
        type="button"
        aria-label={label}
        disabled={disabled}
        className="absolute right-2 top-1/2 -translate-y-1/2 rounded-md p-1 text-base-content/55 transition hover:bg-base-200/70 hover:text-base-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary disabled:opacity-50"
        onClick={() => setOpen((current) => !current)}
      >
        <AppIcon
          name="chevron-down"
          className={cn("h-4 w-4 transition-transform", open && "rotate-180")}
          aria-hidden
        />
      </button>

      {open ? (
        <div
          id={listId}
          role="listbox"
          className={cn(
            "absolute z-20 mt-1 max-h-56 w-full overflow-auto rounded-xl border border-base-300/80 bg-base-100/95 py-1 shadow-lg backdrop-blur",
            listClassName,
          )}
        >
          {loading ? (
            <div className="px-3 py-2 text-sm text-base-content/60">
              {loadingText ?? "Loading…"}
            </div>
          ) : filteredOptions.length === 0 ? (
            <div className="px-3 py-2 text-sm text-base-content/60">
              {emptyText ?? "No matches"}
            </div>
          ) : (
            filteredOptions.map((option, idx) => (
              <button
                key={`${option.value}:${getOptionDisplayValue(option)}`}
                type="button"
                role="option"
                aria-selected={value === getOptionDisplayValue(option)}
                className={cn(
                  "flex w-full items-center justify-between px-3 py-2 text-left text-sm text-base-content",
                  idx === activeIndex ? "bg-base-200/70" : "hover:bg-base-200/50",
                )}
                // Use pointerdown to avoid losing focus before selection on some browsers.
                onPointerDown={(event) => {
                  event.preventDefault();
                  selectOption(option);
                }}
                onMouseEnter={() => setActiveIndex(idx)}
              >
                <span
                  className={cn(
                    "truncate",
                    value === getOptionDisplayValue(option) && "font-semibold",
                  )}
                >
                  {getOptionDisplayValue(option)}
                </span>
              </button>
            ))
          )}
        </div>
      ) : null}
    </div>
  );
}
