import { type ReactNode, type PointerEvent as ReactPointerEvent, useId } from "react";
import { cn } from "../../lib/utils";
import { FormFieldFeedback } from "./form-field-feedback";

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

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function snapToStep(value: number, min: number, step: number) {
  const offset = (value - min) / step;
  const snapped = min + Math.round(offset) * step;
  return Number(snapped.toFixed(resolveStepPrecision(step)));
}

function resolvePercent(value: number, min: number, max: number) {
  if (max <= min) return 0;
  return ((value - min) / (max - min)) * 100;
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
  const resolvedMinThumb = clamp(
    parsedMinValue ?? effectiveSliderMin,
    effectiveSliderMin,
    effectiveSliderMax,
  );
  const resolvedMaxThumb = clamp(
    parsedMaxValue ?? effectiveSliderMax,
    effectiveSliderMin,
    effectiveSliderMax,
  );
  const trackStartValue = Math.min(resolvedMinThumb, resolvedMaxThumb);
  const trackEndValue = Math.max(resolvedMinThumb, resolvedMaxThumb);
  const startPercent = resolvePercent(trackStartValue, effectiveSliderMin, effectiveSliderMax);
  const endPercent = resolvePercent(trackEndValue, effectiveSliderMin, effectiveSliderMax);
  const currentRangeSummary = formatRangeSummary(
    trackStartValue,
    trackEndValue,
    effectiveStep,
    unitLabel,
  );

  const applyThumbValue = (thumb: "min" | "max", nextThumb: number) => {
    if (thumb === "min") {
      const resolved = clamp(nextThumb, effectiveSliderMin, resolvedMaxThumb);
      onChange({
        minValue: resolved <= effectiveSliderMin ? "" : formatSliderValue(resolved, effectiveStep),
        maxValue,
      });
      return;
    }

    const resolved = clamp(nextThumb, resolvedMinThumb, effectiveSliderMax);
    onChange({
      minValue,
      maxValue: resolved >= effectiveSliderMax ? "" : formatSliderValue(resolved, effectiveStep),
    });
  };

  const handleTrackPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    if (event.target instanceof HTMLInputElement) return;
    event.preventDefault();
    const { left, width } = event.currentTarget.getBoundingClientRect();
    if (width <= 0) return;
    const resolveThumbValue = (clientX: number) => {
      const percent = clamp((clientX - left) / width, 0, 1);
      const raw = effectiveSliderMin + percent * (effectiveSliderMax - effectiveSliderMin);
      return snapToStep(raw, effectiveSliderMin, effectiveStep);
    };
    const initialValue = resolveThumbValue(event.clientX);
    const targetThumb =
      Math.abs(initialValue - resolvedMinThumb) <= Math.abs(initialValue - resolvedMaxThumb)
        ? "min"
        : "max";
    const handleClientX = (clientX: number) => {
      applyThumbValue(targetThumb, resolveThumbValue(clientX));
    };
    handleClientX(event.clientX);
    const handlePointerMove = (pointerEvent: PointerEvent) => {
      handleClientX(pointerEvent.clientX);
    };
    const handlePointerUp = () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
      window.removeEventListener("pointercancel", handlePointerUp);
    };
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    window.addEventListener("pointercancel", handlePointerUp);
  };

  return (
    <div className={cn("field", className)} data-testid={testId}>
      <FormFieldFeedback label={label} message={error} messageId={error ? feedbackId : undefined} />
      <div
        className={cn(
          isEmbedded ? "px-0 py-1" : "rounded-lg border border-base-300/80 bg-base-100 px-3 py-3",
          error && !isEmbedded && "border-error/70",
          disabled && "opacity-60",
        )}
      >
        <div className="space-y-2.5">
          <div className="flex justify-end text-[11px] font-medium tabular-nums text-base-content/60">
            <span>{currentRangeSummary}</span>
          </div>
          <div
            className="dual-range-slider"
            data-disabled={disabled ? "true" : "false"}
            onPointerDown={handleTrackPointerDown}
          >
            <div className="dual-range-slider__track" aria-hidden />
            <div
              className="dual-range-slider__range"
              aria-hidden
              style={{
                left: `${startPercent}%`,
                width: `${Math.max(0, endPercent - startPercent)}%`,
              }}
            />
            <input
              type="range"
              min={effectiveSliderMin}
              max={effectiveSliderMax}
              step={effectiveStep}
              disabled={disabled}
              aria-describedby={error ? feedbackId : undefined}
              aria-invalid={error ? true : undefined}
              aria-label={minAriaLabel ? `${minAriaLabel} slider` : "Minimum value slider"}
              aria-valuetext={formatValueText(resolvedMinThumb, effectiveStep, unitLabel)}
              value={resolvedMinThumb}
              onChange={(event) => {
                applyThumbValue("min", Number(event.target.value));
              }}
              className="dual-range-slider__input dual-range-slider__input--min"
            />
            <input
              type="range"
              min={effectiveSliderMin}
              max={effectiveSliderMax}
              step={effectiveStep}
              disabled={disabled}
              aria-describedby={error ? feedbackId : undefined}
              aria-invalid={error ? true : undefined}
              aria-label={maxAriaLabel ? `${maxAriaLabel} slider` : "Maximum value slider"}
              aria-valuetext={formatValueText(resolvedMaxThumb, effectiveStep, unitLabel)}
              value={resolvedMaxThumb}
              onChange={(event) => {
                applyThumbValue("max", Number(event.target.value));
              }}
              className="dual-range-slider__input dual-range-slider__input--max"
            />
          </div>

          <div className="flex items-center justify-between text-[11px] font-semibold uppercase tracking-[0.12em] text-base-content/45">
            <span>{formatDisplayValue(effectiveSliderMin, effectiveStep)}</span>
            <span>{formatDisplayValue(effectiveSliderMax, effectiveStep)}</span>
          </div>
        </div>
      </div>
    </div>
  );
}
