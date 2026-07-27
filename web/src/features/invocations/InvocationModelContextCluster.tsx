import type { ReactNode } from "react";
import type { useTranslation } from "../../i18n";
import type { FastIndicatorState } from "../../lib/invocation";
import { cn } from "../../lib/utils";
import {
  FALLBACK_CELL,
  renderFastIndicator,
  renderInvocationModelBadge,
} from "./invocation-details-shared";
import {
  getReasoningEffortTone,
  REASONING_EFFORT_TONE_CLASSNAMES,
} from "./invocation-table-reasoning";

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
  if (value === FALLBACK_CELL) {
    return (
      <span
        data-testid={testId}
        className="inline-flex shrink-0 items-center font-mono text-[0.625rem] font-semibold text-base-content/48"
        title={value}
      >
        {value}
      </span>
    );
  }

  const tone = getReasoningEffortTone(value);
  return (
    <span
      data-testid={testId}
      data-reasoning-effort-tone={tone}
      className={cn(
        "inline-flex min-h-5 max-w-[5rem] shrink-0 items-center rounded-full border px-2 py-0.5 text-[0.625rem] font-semibold leading-none tracking-[0.01em]",
        REASONING_EFFORT_TONE_CLASSNAMES[tone],
      )}
      title={value}
    >
      <span className="truncate whitespace-nowrap">{value}</span>
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
  const fastIndicator = renderFastIndicator(fastIndicatorState, t);
  const fastLabel =
    fastIndicatorState === "effective"
      ? t("table.model.fastPriorityAria")
      : fastIndicatorState === "requested_only"
        ? t("table.model.fastRequestedOnlyAria")
        : null;
  const modelLabel = [
    modelValue,
    reasoningEffortValue === FALLBACK_CELL ? null : reasoningEffortValue,
    fastLabel,
  ]
    .filter(Boolean)
    .join(" · ");
  const separator = <span className="shrink-0 text-base-content/28">·</span>;
  const content: ReactNode = (
    <>
      <span data-testid={modelTestId} className="min-w-0">
        {renderInvocationModelBadge(modelValue, {
          t,
          hasMismatch: modelHasMismatch,
          className: "max-w-full",
          textClassName: "font-mono",
          iconClassName: "h-3 w-3",
          testId: testId ? `${testId}-model` : undefined,
        })}
      </span>
      {separator}
      <InvocationReasoningEffortBadge
        value={reasoningEffortValue}
        testId={testId ? `${testId}-reasoning-effort` : undefined}
      />
      {fastIndicator ? (
        <>
          {separator}
          {fastIndicator}
        </>
      ) : null}
    </>
  );

  return (
    <div
      data-testid={testId}
      data-model-context-grouped={grouped ? "true" : "false"}
      className={cn(
        grouped
          ? "inline-flex min-w-0 max-w-full items-center gap-1 rounded-full border border-base-300/65 bg-base-100/65 px-1.5 py-0.5 shadow-sm"
          : "flex min-w-0 items-center gap-1",
        className,
      )}
      title={modelLabel}
      aria-label={modelLabel}
      role="group"
    >
      {content}
    </div>
  );
}
