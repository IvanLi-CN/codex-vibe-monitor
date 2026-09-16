import {
  type KeyboardEvent,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
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

interface FilterableComboboxListProps {
  id: string;
  options: FilterableComboboxOption[];
  value: string;
  activeIndex: number;
  loading?: boolean;
  loadingText?: string;
  emptyText?: string;
  listClassName?: string;
  onSelect: (option: FilterableComboboxOption) => void;
  onActiveIndexChange: (index: number) => void;
}

function FilterableComboboxList({
  id,
  options,
  value,
  activeIndex,
  loading,
  loadingText,
  emptyText,
  listClassName,
  onSelect,
  onActiveIndexChange,
}: FilterableComboboxListProps) {
  return (
    <div
      id={id}
      role="listbox"
      className={cn(
        "absolute z-20 mt-1 max-h-56 w-full overflow-auto rounded-xl border border-base-300/80 bg-base-100/95 py-1 shadow-lg backdrop-blur",
        listClassName,
      )}
    >
      {loading ? (
        <div className="px-3 py-2 text-sm text-base-content/60">{loadingText ?? "Loading…"}</div>
      ) : options.length === 0 ? (
        <div className="px-3 py-2 text-sm text-base-content/60">{emptyText ?? "No matches"}</div>
      ) : (
        options.map((option, idx) => (
          <button
            key={`${option.value}:${getOptionDisplayValue(option)}`}
            type="button"
            role="option"
            aria-selected={value === getOptionDisplayValue(option)}
            className={cn(
              "flex w-full items-center justify-between px-3 py-2 text-left text-sm text-base-content",
              idx === activeIndex ? "bg-base-200/70" : "hover:bg-base-200/50",
            )}
            onPointerDown={(event) => {
              event.preventDefault();
              onSelect(option);
            }}
            onMouseEnter={() => onActiveIndexChange(idx)}
          >
            <span
              className={cn("truncate", value === getOptionDisplayValue(option) && "font-semibold")}
            >
              {getOptionDisplayValue(option)}
            </span>
          </button>
        ))
      )}
    </div>
  );
}

interface FilterableComboboxInputProps {
  rootRef: React.RefObject<HTMLDivElement | null>;
  inputId: string;
  listId: string;
  value: string;
  label: string;
  name?: string;
  placeholder?: string;
  disabled?: boolean;
  open: boolean;
  inputClassName?: string;
  resolvedInputAutocompleteProps: React.InputHTMLAttributes<HTMLInputElement>;
  onValueChange: (value: string) => void;
  onOpen: () => void;
  onToggle: () => void;
  onBlur: () => void;
  onKeyDown: (event: KeyboardEvent<HTMLInputElement>) => void;
}

function useFilterableComboboxState({
  value,
  options,
  open,
  setOpen,
  activeIndex,
  setActiveIndex,
  onValueChange,
  onOptionSelect,
}: {
  value: string;
  options: Array<string | FilterableComboboxOption>;
  open: boolean;
  setOpen: (open: boolean) => void;
  activeIndex: number;
  setActiveIndex: (value: number | ((current: number) => number)) => void;
  onValueChange: (value: string) => void;
  onOptionSelect?: (option: FilterableComboboxOption) => void;
}) {
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
  }, [filteredOptions, open, setActiveIndex]);

  const selectOption = useCallback(
    (option: FilterableComboboxOption) => {
      onValueChange(getOptionDisplayValue(option));
      onOptionSelect?.(option);
      setOpen(false);
    },
    [onOptionSelect, onValueChange, setOpen],
  );

  const handleKeyDown = useCallback(
    (event: KeyboardEvent<HTMLInputElement>) => {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setOpen(true);
        setActiveIndex((current: number) =>
          Math.min(filteredOptions.length - 1, Math.max(0, current + 1)),
        );
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setOpen(true);
        setActiveIndex((current: number) => Math.max(0, current - 1));
        return;
      }
      if (event.key === "Enter") {
        if (!open) return;
        event.preventDefault();
        const next = filteredOptions[activeIndex];
        if (next) selectOption(next);
        return;
      }
      if (event.key === "Escape") setOpen(false);
    },
    [activeIndex, filteredOptions, open, selectOption, setActiveIndex, setOpen],
  );

  return { filteredOptions, selectOption, handleKeyDown };
}

function FilterableComboboxInput({
  rootRef,
  inputId,
  listId,
  value,
  label,
  name,
  placeholder,
  disabled,
  open,
  inputClassName,
  resolvedInputAutocompleteProps,
  onValueChange,
  onOpen,
  onToggle,
  onBlur,
  onKeyDown,
}: FilterableComboboxInputProps) {
  return (
    <div ref={rootRef} className="relative">
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
        onFocus={onOpen}
        onClick={onOpen}
        onBlur={onBlur}
        onKeyDown={onKeyDown}
      />
      <button
        type="button"
        aria-label={label}
        disabled={disabled}
        className="absolute right-2 top-1/2 -translate-y-1/2 rounded-md p-1 text-base-content/55 transition hover:bg-base-200/70 hover:text-base-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary disabled:opacity-50"
        onClick={onToggle}
      >
        <AppIcon
          name="chevron-down"
          className={cn("h-4 w-4 transition-transform", open && "rotate-180")}
          aria-hidden
        />
      </button>
    </div>
  );
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
  const setOpenState = useCallback(
    (nextOpen: boolean) => {
      setOpen(nextOpen);
      onOpenChange?.(nextOpen);
    },
    [onOpenChange],
  );

  useEffect(() => {
    if (!open) return;
    const handlePointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpenState(false);
      }
    };
    document.addEventListener("pointerdown", handlePointerDown);
    return () => document.removeEventListener("pointerdown", handlePointerDown);
  }, [open, setOpenState]);

  const { filteredOptions, selectOption, handleKeyDown } = useFilterableComboboxState({
    value,
    options,
    open,
    setOpen: setOpenState,
    activeIndex,
    setActiveIndex,
    onValueChange,
    onOptionSelect,
  });

  return (
    <div className={cn("relative", className)}>
      <FilterableComboboxInput
        rootRef={rootRef}
        inputId={inputId}
        listId={listId}
        value={value}
        label={label}
        name={name}
        placeholder={placeholder}
        disabled={disabled}
        open={open}
        inputClassName={inputClassName}
        resolvedInputAutocompleteProps={resolvedInputAutocompleteProps}
        onValueChange={onValueChange}
        onOpen={() => setOpenState(true)}
        onToggle={() => setOpenState(!open)}
        onBlur={() => {
          window.setTimeout(() => {
            if (!rootRef.current?.contains(document.activeElement)) setOpenState(false);
          }, 0);
        }}
        onKeyDown={handleKeyDown}
      />
      {open ? (
        <FilterableComboboxList
          id={listId}
          options={filteredOptions}
          value={value}
          activeIndex={activeIndex}
          loading={loading}
          loadingText={loadingText}
          emptyText={emptyText}
          listClassName={listClassName}
          onSelect={selectOption}
          onActiveIndexChange={setActiveIndex}
        />
      ) : null}
    </div>
  );
}
