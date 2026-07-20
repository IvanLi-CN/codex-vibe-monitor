import { type ReactNode, useId } from "react";
import { cn } from "../../lib/utils";
import { FormFieldFeedback } from "./form-field-feedback";
import { Slider, SliderRange, SliderThumb, SliderTrack } from "./slider";

interface NumericRangeFieldProps {
  label: ReactNode;
  minValue: string;
  maxValue: string;
  onChange: (next: { minValue: string; maxValue: string }) => void;
  sliderMin: number;
  sliderMax: number;
  minAriaLabel?: string;
  maxAriaLabel?: string;
  unitLabel?: string;
  surface?: "default" | "embedded";
  step?: number;
  disabled?: boolean;
  error?: string | null;
  className?: string;
  testId?: string;
}

function resolveStepPrecision(step?: number) {
  if (!step || Number.isInteger(step)) return 0;
  const raw = step.toString().toLowerCase();
  if (raw.includes("e-")) {
    const exponent = Number(raw.split("e-")[1]);
    return Number.isFinite(exponent) ? exponent : 0;
  }
  return raw.split(".")[1]?.length ?? 0;
}

function parseFiniteNumber(value: string) {
  const normalized = value.trim();
  if (!normalized) return null;
  const parsed = Number(normalized);
  return Number.isFinite(parsed) ? parsed : null;
}

function formatSliderValue(value: number, step?: number) {
  const precision = resolveStepPrecision(step);
  if (precision <= 0) return String(Math.round(value));
  return String(Number(value.toFixed(precision)));
}

function snapToStep(value: number, min: number, step: number) {
  const offset = (value - min) / step;
  const snapped = min + Math.round(offset) * step;
  return Number(snapped.toFixed(resolveStepPrecision(step)));
}

function formatDisplayValue(value: number, step?: number) {
  const precision = resolveStepPrecision(step);
  return value.toLocaleString(undefined, {
    minimumFractionDigits: precision,
    maximumFractionDigits: precision,
  });
}

function formatValueText(value: number, step?: number, unitLabel?: string) {
  const formatted = formatDisplayValue(value, step);
  return unitLabel ? `${formatted} ${unitLabel}` : formatted;
}

function formatRangeSummary(
  startValue: number,
  endValue: number,
  step?: number,
  unitLabel?: string,
) {
  const startLabel = formatDisplayValue(startValue, step);
  const endLabel = formatDisplayValue(endValue, step);
  return unitLabel ? `${startLabel} - ${endLabel} ${unitLabel}` : `${startLabel} - ${endLabel}`;
}

export function NumericRangeField({
  label,
  minValue,
  maxValue,
  onChange,
  sliderMin,
  sliderMax,
  minAriaLabel,
  maxAriaLabel,
  unitLabel,
  surface = "default",
  step,
  disabled,
  error,
  className,
  testId,
}: NumericRangeFieldProps) {
  const feedbackId = useId();
  const isEmbedded = surface === "embedded";
  const effectiveStep = step && step > 0 ? step : 1;
  const effectiveSliderMin = Number.isFinite(sliderMin) ? sliderMin : 0;
  const effectiveSliderMax =
    Number.isFinite(sliderMax) && sliderMax > effectiveSliderMin
      ? sliderMax
      : effectiveSliderMin + effectiveStep;
  const parsedMinValue = parseFiniteNumber(minValue);
  const parsedMaxValue = parseFiniteNumber(maxValue);
  const resolvedMinThumb = Math.min(
    effectiveSliderMax,
    Math.max(effectiveSliderMin, parsedMinValue ?? effectiveSliderMin),
  );
  const resolvedMaxThumb = Math.min(
    effectiveSliderMax,
    Math.max(effectiveSliderMin, parsedMaxValue ?? effectiveSliderMax),
  );
  const trackStartValue = Math.min(resolvedMinThumb, resolvedMaxThumb);
  const trackEndValue = Math.max(resolvedMinThumb, resolvedMaxThumb);
  const currentRangeSummary = formatRangeSummary(
    trackStartValue,
    trackEndValue,
    effectiveStep,
    unitLabel,
  );

  const applyThumbValues = (nextValues: number[]) => {
    const [nextMinRaw = effectiveSliderMin, nextMaxRaw = effectiveSliderMax] = nextValues;
    const nextMin = Math.min(nextMinRaw, nextMaxRaw);
    const nextMax = Math.max(nextMinRaw, nextMaxRaw);
    const normalizedMin = snapToStep(nextMin, effectiveSliderMin, effectiveStep);
    const normalizedMax = snapToStep(nextMax, effectiveSliderMin, effectiveStep);
    onChange({
      minValue:
        normalizedMin <= effectiveSliderMin ? "" : formatSliderValue(normalizedMin, effectiveStep),
      maxValue:
        normalizedMax >= effectiveSliderMax ? "" : formatSliderValue(normalizedMax, effectiveStep),
    });
  };

  return (
    <div className={cn("field", className)} data-testid={testId}>
      <FormFieldFeedback
        label={
          <span className="flex w-full min-w-0 items-center justify-between gap-3">
            <span className="min-w-0 truncate">{label}</span>
            <span
              aria-hidden="true"
              className="min-w-0 max-w-[72%] truncate text-right text-[11px] font-medium tabular-nums normal-case tracking-normal text-base-content/60"
            >
              {currentRangeSummary}
            </span>
          </span>
        }
        labelClassName="flex min-w-0 flex-1"
        message={error}
        messageId={error ? feedbackId : undefined}
      />
      <div
        className={cn(
          isEmbedded ? "px-0 py-1" : "rounded-lg border border-base-300/80 bg-base-100 px-3 py-3",
          error && !isEmbedded && "border-error/70",
          disabled && "opacity-60",
        )}
      >
        <div className="space-y-1">
          <Slider
            className="h-8"
            data-testid={testId ? `${testId}-slider` : undefined}
            value={[trackStartValue, trackEndValue]}
            min={effectiveSliderMin}
            max={effectiveSliderMax}
            step={effectiveStep}
            minStepsBetweenThumbs={0}
            disabled={disabled}
            onValueChange={applyThumbValues}
          >
            <SliderTrack>
              <SliderRange />
            </SliderTrack>
            <SliderThumb
              aria-describedby={error ? feedbackId : undefined}
              aria-invalid={error ? true : undefined}
              aria-label={minAriaLabel ? `${minAriaLabel} slider` : "Minimum value slider"}
              aria-valuetext={formatValueText(trackStartValue, effectiveStep, unitLabel)}
            />
            <SliderThumb
              aria-describedby={error ? feedbackId : undefined}
              aria-invalid={error ? true : undefined}
              aria-label={maxAriaLabel ? `${maxAriaLabel} slider` : "Maximum value slider"}
              aria-valuetext={formatValueText(trackEndValue, effectiveStep, unitLabel)}
            />
          </Slider>

          <div className="flex items-center justify-between text-[11px] font-semibold uppercase tracking-[0.12em] text-base-content/45">
            <span>{formatDisplayValue(effectiveSliderMin, effectiveStep)}</span>
            <span>{formatDisplayValue(effectiveSliderMax, effectiveStep)}</span>
          </div>
        </div>
      </div>
    </div>
  );
}
