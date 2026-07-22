import { type ReactNode, useId } from "react";
import { AppIcon } from "../../features/shared/AppIcon";
import { textInputAutocompleteOffProps } from "../../lib/form-autocomplete";
import { cn } from "../../lib/utils";
import { Button } from "./button";
import { FormFieldFeedback } from "./form-field-feedback";
import { Input } from "./input";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";

export interface DateTimeRangeFieldOption<T extends string = string> {
  value: T;
  label: string;
}

export interface DateTimeRangeFieldValue<T extends string = string> {
  preset: T;
  from: string;
  to: string;
}

interface DateTimeRangeFieldProps<T extends string = string> {
  label: ReactNode;
  value: DateTimeRangeFieldValue<T>;
  options: Array<DateTimeRangeFieldOption<T>>;
  customPresetValue: T;
  onChange: (next: DateTimeRangeFieldValue<T>) => void;
  summary?: string;
  fromLabel?: ReactNode;
  toLabel?: ReactNode;
  disabled?: boolean;
  error?: string | null;
  className?: string;
  triggerClassName?: string;
  contentClassName?: string;
  fromName?: string;
  toName?: string;
  testId?: string;
}

function fallbackSummary<T extends string>(
  value: DateTimeRangeFieldValue<T>,
  options: Array<DateTimeRangeFieldOption<T>>,
  customPresetValue: T,
) {
  if (value.preset !== customPresetValue) {
    return options.find((option) => option.value === value.preset)?.label ?? String(value.preset);
  }
  const values = [value.from, value.to].filter(Boolean).map((item) => item.replace("T", " "));
  return (
    values.join(" - ") ||
    (options.find((option) => option.value === customPresetValue)?.label ?? "Custom")
  );
}

export function DateTimeRangeField<T extends string = string>({
  label,
  value,
  options,
  customPresetValue,
  onChange,
  summary,
  fromLabel = "From",
  toLabel = "To",
  disabled,
  error,
  className,
  triggerClassName,
  contentClassName,
  fromName,
  toName,
  testId,
}: DateTimeRangeFieldProps<T>) {
  const labelId = useId();
  const summaryId = useId();
  const feedbackId = useId();
  const resolvedSummary = summary ?? fallbackSummary(value, options, customPresetValue);

  return (
    <div className={cn("field", className)} data-testid={testId}>
      <FormFieldFeedback
        label={label}
        labelId={labelId}
        message={error}
        messageId={error ? feedbackId : undefined}
      />
      <Popover>
        <PopoverTrigger asChild>
          <Button
            type="button"
            variant="outline"
            disabled={disabled}
            aria-labelledby={`${labelId} ${summaryId}`}
            aria-describedby={error ? feedbackId : undefined}
            aria-invalid={error ? true : undefined}
            className={cn(
              "h-auto w-full items-start justify-between rounded-lg px-3 py-2 text-left",
              error && "border-error/70 text-error",
              triggerClassName,
            )}
          >
            <span className="flex min-w-0 flex-col gap-1">
              <span className="text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/45">
                {options.find((option) => option.value === value.preset)?.label ??
                  String(value.preset)}
              </span>
              <span id={summaryId} className="truncate text-sm text-base-content">
                {resolvedSummary}
              </span>
            </span>
            <AppIcon name="chevron-down" className="mt-0.5 h-4 w-4 shrink-0 text-base-content/55" />
          </Button>
        </PopoverTrigger>
        <PopoverContent
          aria-labelledby={labelId}
          align="start"
          sideOffset={8}
          className={cn("w-[min(24rem,calc(100vw-2rem))] rounded-2xl p-3", contentClassName)}
        >
          <div className="space-y-3">
            <div className="grid grid-cols-2 gap-2">
              {options.map((option) => {
                const active = option.value === value.preset;
                return (
                  <Button
                    key={option.value}
                    type="button"
                    variant={active ? "default" : "ghost"}
                    className="justify-start rounded-xl"
                    onClick={() => onChange({ ...value, preset: option.value })}
                  >
                    {option.label}
                  </Button>
                );
              })}
            </div>

            <div className="rounded-2xl border border-base-300/75 bg-base-200/35 p-3">
              <div className="grid gap-3">
                <label className="field gap-1">
                  <span className="field-label">{fromLabel}</span>
                  <Input
                    {...textInputAutocompleteOffProps}
                    type="datetime-local"
                    name={fromName}
                    aria-describedby={error ? feedbackId : undefined}
                    aria-invalid={error ? true : undefined}
                    disabled={disabled}
                    value={value.from}
                    onChange={(event) =>
                      onChange({
                        ...value,
                        preset: customPresetValue,
                        from: event.target.value,
                      })
                    }
                  />
                </label>
                <label className="field gap-1">
                  <span className="field-label">{toLabel}</span>
                  <Input
                    {...textInputAutocompleteOffProps}
                    type="datetime-local"
                    name={toName}
                    aria-describedby={error ? feedbackId : undefined}
                    aria-invalid={error ? true : undefined}
                    disabled={disabled}
                    value={value.to}
                    onChange={(event) =>
                      onChange({
                        ...value,
                        preset: customPresetValue,
                        to: event.target.value,
                      })
                    }
                  />
                </label>
              </div>
            </div>
          </div>
        </PopoverContent>
      </Popover>
    </div>
  );
}
