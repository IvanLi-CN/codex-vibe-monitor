import type { useTranslation } from "../../i18n";
import type { FastIndicatorState } from "../../lib/invocation";
import { cn } from "../../lib/utils";
import {
  formatReasoningEffort,
  getReasoningEffortTone,
  REASONING_EFFORT_TONE_CLASSNAMES,
  type ReasoningEffortTone,
} from "../shared/reasoningEffort";
import {
  FALLBACK_CELL,
  renderFastIndicator,
  renderInvocationModelBadge,
} from "./invocation-details-shared";

const REASONING_EFFORT_CONTEXT_TONE_CLASSNAMES: Record<ReasoningEffortTone, string> = {
  none: "text-base-content/68",
  minimal: "tone-ink-info",
  low: "tone-ink-info",
  medium: "tone-ink-primary",
  high: "tone-ink-warning",
  xhigh: "tone-ink-warning",
  max: "tone-ink-error",
  ultra: "font-bold tone-ink-error",
  unknown: "text-base-content/62",
};

const REASONING_EFFORT_CONTEXT_MARKER_CLASSNAMES: Record<ReasoningEffortTone, string> = {
  none: "bg-base-content/34",
  minimal: "bg-info/65",
  low: "bg-info/80",
  medium: "bg-primary/80",
  high: "bg-warning/85",
  xhigh: "bg-warning",
  max: "bg-error/85",
  ultra: "bg-error",
  unknown: "bg-base-content/38",
};

export interface InvocationModelContextClusterProps {
  modelValue: string;
  modelHasMismatch?: boolean;
  reasoningEffortValue: string;
  fastIndicatorState: FastIndicatorState;
  grouped: boolean;
  t: ReturnType<typeof useTranslation>["t"];
  className?: string;
  testId?: string;
  modelTestId?: string;
}

export function InvocationReasoningEffortBadge({
  value,
  testId,
}: {
  value: string;
  testId?: string;
}) {
  const displayValue = formatReasoningEffort(value);
  if (displayValue === FALLBACK_CELL) {
    return (
      <span
        data-testid={testId}
        className="inline-flex shrink-0 items-center font-mono text-xs font-semibold text-base-content/48"
        title={displayValue}
      >
        {displayValue}
      </span>
    );
  }

  const tone = getReasoningEffortTone(displayValue);
  return (
    <span
      data-testid={testId}
      data-reasoning-effort-tone={tone}
      className={cn(
        "inline-flex min-h-5 max-w-[5rem] shrink-0 items-center rounded-full border px-2 py-0.5 text-xs font-semibold leading-none tracking-[0.01em]",
        REASONING_EFFORT_TONE_CLASSNAMES[tone],
      )}
      title={displayValue}
    >
      <span className="truncate whitespace-nowrap">{displayValue}</span>
    </span>
  );
}

export function InvocationModelContextCluster({
  modelValue,
  modelHasMismatch = false,
  reasoningEffortValue,
  fastIndicatorState,
  grouped,
  t,
  className,
  testId,
  modelTestId,
}: InvocationModelContextClusterProps) {
  const displayReasoningEffort = formatReasoningEffort(reasoningEffortValue);
  const fastIndicator = renderFastIndicator(fastIndicatorState, t);
  const fastLabel =
    fastIndicatorState === "effective"
      ? t("table.model.fastPriorityAria")
      : fastIndicatorState === "requested_only"
        ? t("table.model.fastRequestedOnlyAria")
        : null;
  const modelLabel = [
    modelValue,
    displayReasoningEffort === FALLBACK_CELL ? null : displayReasoningEffort,
    fastLabel,
  ]
    .filter(Boolean)
    .join(" · ");
  const reasoningTone =
    displayReasoningEffort === FALLBACK_CELL
      ? "none"
      : getReasoningEffortTone(displayReasoningEffort);
  const model = renderInvocationModelBadge(modelValue, {
    t,
    hasMismatch: modelHasMismatch,
    className: "max-w-full",
    textClassName: "font-mono",
    iconClassName: "h-3.5 w-3.5",
    testId: testId ? `${testId}-model` : undefined,
  });

  if (grouped) {
    return (
      <div
        data-testid={testId}
        data-model-context-grouped="true"
        className={cn(
          "inline-flex h-5 min-w-0 max-w-full items-stretch overflow-hidden rounded-md border border-base-300/75 bg-base-200/58 leading-none",
          className,
        )}
        title={modelLabel}
        aria-label={modelLabel}
        role="group"
      >
        <span
          data-testid={modelTestId}
          data-model-context-part="model"
          className="flex w-5 shrink-0 items-center justify-center text-base-content/72"
        >
          {model}
        </span>
        <span className="w-px shrink-0 bg-base-300/75" aria-hidden />
        <span
          data-testid={testId ? `${testId}-reasoning-effort` : undefined}
          data-model-context-part="reasoning-effort"
          data-reasoning-effort-tone={reasoningTone}
          className={cn(
            "flex min-w-0 items-center gap-1 px-1.5 text-xs font-semibold",
            REASONING_EFFORT_CONTEXT_TONE_CLASSNAMES[reasoningTone],
          )}
          title={displayReasoningEffort}
        >
          <span
            className={cn(
              "h-1 w-1 shrink-0 rounded-full",
              REASONING_EFFORT_CONTEXT_MARKER_CLASSNAMES[reasoningTone],
            )}
            aria-hidden
          />
          <span className="truncate whitespace-nowrap">{displayReasoningEffort}</span>
        </span>
        {fastIndicator ? (
          <>
            <span className="w-px shrink-0 bg-base-300/75" aria-hidden />
            <span
              data-model-context-part="fast"
              className="flex w-5 shrink-0 items-center justify-center"
            >
              {fastIndicator}
            </span>
          </>
        ) : null}
      </div>
    );
  }

  return (
    <div
      data-testid={testId}
      data-model-context-grouped="false"
      className={cn("flex min-w-0 items-center gap-1", className)}
      title={modelLabel}
      aria-label={modelLabel}
      role="group"
    >
      <span data-testid={modelTestId} className="min-w-0">
        {model}
      </span>
      <span className="shrink-0 text-base-content/28">·</span>
      <InvocationReasoningEffortBadge
        value={displayReasoningEffort}
        testId={testId ? `${testId}-reasoning-effort` : undefined}
      />
      {fastIndicator ? (
        <>
          <span className="shrink-0 text-base-content/28">·</span>
          {fastIndicator}
        </>
      ) : null}
    </div>
  );
}
