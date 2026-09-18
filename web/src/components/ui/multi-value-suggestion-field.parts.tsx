import { AppIcon } from "../../features/shared/AppIcon";
import { cn } from "../../lib/utils";
import { Chip } from "./chip";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "./command";
import { FormFieldFeedback } from "./form-field-feedback";
import type { MultiValueSuggestionFieldProps } from "./multi-value-suggestion-field";
import {
  getMultiValueOptionDisplayValue,
  type MultiValueSuggestionFieldState,
  normalizeMultiValueKey,
} from "./multi-value-suggestion-field.state";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";

type MultiValueSuggestionFieldRenderProps = MultiValueSuggestionFieldProps &
  MultiValueSuggestionFieldState;

function MultiValueSuggestionFieldOptionList(props: MultiValueSuggestionFieldRenderProps) {
  return (
    <CommandList
      id={`${props.inputId}-list`}
      className={cn(props.isEmbedded && "max-h-40", props.listClassName)}
    >
      {props.canAddInput ? (
        <>
          <CommandGroup>
            <CommandItem
              value={`${props.addLabel ?? "Add"} ${props.committedInputValue}`}
              onSelect={() => {
                props.addValue(props.committedInputValue);
                props.focusSearchInput();
              }}
            >
              <AppIcon
                name="plus-circle-outline"
                className="mr-2 h-4 w-4 text-primary"
                aria-hidden
              />
              <span className="truncate">
                {props.addLabel ?? "Add"} “{props.trimmedInputValue}”
              </span>
            </CommandItem>
          </CommandGroup>
          <CommandSeparator />
        </>
      ) : null}
      {props.loading ? (
        <div className="px-3 py-2 text-sm text-base-content/60">
          {props.loadingText ?? "Loading…"}
        </div>
      ) : props.filteredOptions.length === 0 ? (
        <CommandEmpty>{props.emptyText ?? "No matches"}</CommandEmpty>
      ) : (
        <CommandGroup>
          {props.filteredOptions.map((option) => {
            const active = props.selectedValueSet.has(normalizeMultiValueKey(option.value));
            return (
              <CommandItem
                key={`${option.value}:${getMultiValueOptionDisplayValue(option)}`}
                value={`${option.value} ${getMultiValueOptionDisplayValue(option)} ${option.searchText ?? ""}`}
                disabled={props.disabled}
                onSelect={() => {
                  props.toggleOption(option);
                  props.focusSearchInput();
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
                <span className="truncate">{getMultiValueOptionDisplayValue(option)}</span>
              </CommandItem>
            );
          })}
        </CommandGroup>
      )}
    </CommandList>
  );
}

function MultiValueSuggestionFieldTrigger(props: MultiValueSuggestionFieldRenderProps) {
  return (
    <PopoverTrigger asChild>
      <button
        type="button"
        role="combobox"
        aria-expanded={props.open}
        aria-label={props.inputLabel}
        aria-describedby={props.error ? props.feedbackId : undefined}
        aria-invalid={props.error ? true : undefined}
        aria-controls={props.open ? `${props.inputId}-list` : undefined}
        disabled={props.disabled}
        title={props.triggerTitle}
        onClick={(event) => {
          if (!props.open) return;
          event.preventDefault();
          props.commitOpenState(false);
        }}
        className={cn(
          "flex min-h-11 w-full items-center gap-3 rounded-xl border border-base-300/80 bg-base-100 px-3 py-2.5 text-left shadow-sm transition-colors",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100",
          "hover:border-primary/35",
          props.isEmbedded && props.open && "border-primary/45 ring-2 ring-primary/15",
          !props.isEmbedded && props.error && "border-error/70",
          props.isEmbedded && props.error && "border-error/70",
          props.disabled && "cursor-not-allowed opacity-60",
        )}
      >
        <AppIcon
          name="tag-outline"
          className="mt-0.5 h-4 w-4 shrink-0 text-base-content/55"
          aria-hidden
        />
        <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          {props.values.length > 0 ? (
            props.values.map((value) => (
              <Chip
                key={normalizeMultiValueKey(value)}
                tone="secondary"
                className="max-w-full px-2.5 py-1"
              >
                <span className="truncate">
                  {props.optionLabelMap.get(normalizeMultiValueKey(value)) ?? value.trim()}
                </span>
              </Chip>
            ))
          ) : (
            <span className="text-sm text-base-content/55">
              {props.placeholder || props.inputLabel}
            </span>
          )}
        </span>
        <AppIcon
          name="chevron-down"
          className={cn(
            "h-4 w-4 shrink-0 text-base-content/45 transition-transform",
            props.open && "rotate-180",
          )}
          aria-hidden
        />
      </button>
    </PopoverTrigger>
  );
}

function MultiValueSuggestionFieldInput(
  props: MultiValueSuggestionFieldRenderProps,
  embedded: boolean,
) {
  return (
    <CommandInput
      {...props.resolvedInputAutocompleteProps}
      ref={props.searchInputRef}
      id={props.inputId}
      name={props.name}
      aria-label={props.inputLabel}
      value={props.inputValue}
      placeholder={props.placeholder}
      onValueChange={props.onInputValueChange}
      onKeyDown={embedded ? props.handleEmbeddedInputKeyDown : undefined}
      className={cn("w-full", props.inputClassName)}
    />
  );
}

function MultiValueSuggestionFieldEmbedded(props: MultiValueSuggestionFieldRenderProps) {
  return (
    <div className={cn("field space-y-3", props.className)} data-testid={props.testId}>
      <FormFieldFeedback
        label={props.label}
        message={props.error}
        messageId={props.error ? props.feedbackId : undefined}
      />
      <Popover open={props.disabled ? false : props.open} onOpenChange={props.commitOpenState}>
        <MultiValueSuggestionFieldTrigger {...props} />
        <PopoverContent
          align="start"
          sideOffset={6}
          collisionPadding={12}
          style={{ zIndex: 90 }}
          className="z-[60] w-[var(--radix-popover-trigger-width)] overflow-hidden rounded-xl border border-base-300/80 bg-base-100 p-0 shadow-lg"
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            props.focusSearchInput();
          }}
        >
          <Command shouldFilter={false}>
            {MultiValueSuggestionFieldInput(props, true)}
            <MultiValueSuggestionFieldOptionList {...props} />
          </Command>
        </PopoverContent>
      </Popover>
    </div>
  );
}

function MultiValueSuggestionFieldDefault(props: MultiValueSuggestionFieldRenderProps) {
  return (
    <div className={cn("field", props.className)} data-testid={props.testId}>
      <FormFieldFeedback
        label={props.label}
        message={props.error}
        messageId={props.error ? props.feedbackId : undefined}
      />
      <div
        className={cn(
          "space-y-3 rounded-lg border border-base-300/80 bg-base-100 p-3",
          props.error && "border-error/70",
          props.disabled && "opacity-60",
        )}
      >
        <Popover open={props.disabled ? false : props.open} onOpenChange={props.commitOpenState}>
          <MultiValueSuggestionFieldTrigger {...props} />
          <PopoverContent
            align="start"
            className="w-[var(--radix-popover-trigger-width)] p-0"
            onOpenAutoFocus={(event) => {
              event.preventDefault();
              props.focusSearchInput();
            }}
          >
            <Command shouldFilter={false}>
              {MultiValueSuggestionFieldInput(props, false)}
              <MultiValueSuggestionFieldOptionList {...props} />
            </Command>
          </PopoverContent>
        </Popover>
      </div>
    </div>
  );
}

export function MultiValueSuggestionFieldView(props: MultiValueSuggestionFieldRenderProps) {
  return props.isEmbedded ? (
    <MultiValueSuggestionFieldEmbedded {...props} />
  ) : (
    <MultiValueSuggestionFieldDefault {...props} />
  );
}
