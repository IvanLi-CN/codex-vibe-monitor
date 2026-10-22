import { Chip } from "../../components/ui/chip";
import type { useTranslation } from "../../i18n";
import type { FastIndicatorState } from "../../lib/invocation";
import { cn } from "../../lib/utils";
import { AppIcon } from "../shared/AppIcon";
import { ModelIdentity, resolveModelIdentityGenerationLabel } from "../shared/ModelIdentity";
import {
  formatReasoningEffort,
  getReasoningEffortTone,
  type ReasoningEffortTone,
} from "../shared/reasoningEffort";
import {
  FALLBACK_CELL,
  renderFastIndicator,
  renderInvocationModelChip,
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

export interface InvocationModelContextClusterProps {
  modelValue: string;
  requestModelValue?: string;
  modelHasMismatch?: boolean;
  reasoningEffortValue: string;
  fastIndicatorState: FastIndicatorState;
  grouped: boolean;
  t: ReturnType<typeof useTranslation>["t"];
  className?: string;
  testId?: string;
  modelTestId?: string;
}

function CompactModelIdentity({
  model,
  side,
  testId,
}: {
  model: string;
  side: "request" | "response" | "single";
  testId?: string;
}) {
  const generationLabel = resolveModelIdentityGenerationLabel(model);
  const identityTestId = testId ? `${testId}-${side}-identity` : undefined;

  return (
    <span
      data-model-context-identity={side}
      className={cn(
        "inline-flex min-w-0 max-w-full items-center gap-0.5",
        generationLabel ? "shrink-0" : "flex-1",
      )}
      title={model || undefined}
    >
      {generationLabel ? (
        <span
          data-model-generation-segment={generationLabel}
          data-model-context-side={side}
          data-testid={testId ? `${testId}-${side}-generation` : undefined}
          className="flex h-4 shrink-0 translate-y-px items-center font-mono text-[10px] font-semibold leading-none text-base-content/78"
        >
          {generationLabel}
        </span>
      ) : null}
      <ModelIdentity
        model={model}
        className={cn("min-w-0 max-w-full", generationLabel ? "h-4 w-4 shrink-0" : "flex-1")}
        textClassName="truncate font-mono text-[10px] leading-none text-base-content/84"
        iconClassName="h-3.5 w-3.5"
        title={model}
        testId={identityTestId}
      />
    </span>
  );
}

export function InvocationReasoningEffortChip({
  value,
  testId,
  className,
}: {
  value: string;
  testId?: string;
  className?: string;
}) {
  const displayValue = formatReasoningEffort(value);
  if (displayValue === FALLBACK_CELL) {
    return (
      <span
        data-testid={testId}
        className={cn(
          "inline-flex shrink-0 items-center font-mono text-xs font-semibold text-base-content/48",
          className,
        )}
        title={displayValue}
      >
        {displayValue}
      </span>
    );
  }

  const tone = getReasoningEffortTone(displayValue);
  const chipTone =
    tone === "minimal" || tone === "low"
      ? "info"
      : tone === "medium"
        ? "primary"
        : tone === "high" || tone === "xhigh"
          ? "warning"
          : tone === "max" || tone === "ultra"
            ? "error"
            : "neutral";
  return (
    <Chip
      size="compact"
      tone={chipTone}
      data-testid={testId}
      data-reasoning-effort-tone={tone}
      className={cn("max-w-[5rem] px-2 text-xs font-semibold tracking-[0.01em]", className)}
      title={displayValue}
    >
      <span className="truncate whitespace-nowrap">{displayValue}</span>
    </Chip>
  );
}

export function InvocationModelContextCluster({
  modelValue,
  requestModelValue,
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
  const hasReasoningEffort = displayReasoningEffort !== FALLBACK_CELL;
  const fastIndicator = renderFastIndicator(fastIndicatorState, t, {
    compact: grouped,
    verticalAlign: grouped ? "center" : "baseline",
  });
  const fastLabel =
    fastIndicatorState === "effective"
      ? t("table.model.fastPriorityAria")
      : fastIndicatorState === "requested_only"
        ? t("table.model.fastRequestedOnlyAria")
        : null;
  const hasModelMismatch =
    modelHasMismatch &&
    requestModelValue != null &&
    requestModelValue !== FALLBACK_CELL &&
    modelValue !== FALLBACK_CELL;
  const compactRequestModel = requestModelValue ?? FALLBACK_CELL;
  const modelRouteLabel = hasModelMismatch ? `${requestModelValue} -> ${modelValue}` : modelValue;
  const modelLabel = [
    modelRouteLabel,
    displayReasoningEffort === FALLBACK_CELL ? null : displayReasoningEffort,
    fastLabel,
  ]
    .filter(Boolean)
    .join(" · ");
  const modelAriaLabel = [
    hasModelMismatch
      ? `${t("table.model.routingMismatchAria")}: ${requestModelValue} -> ${modelValue}`
      : modelValue,
    displayReasoningEffort === FALLBACK_CELL ? null : displayReasoningEffort,
    fastLabel,
  ]
    .filter(Boolean)
    .join(" · ");
  const reasoningTone = hasReasoningEffort
    ? getReasoningEffortTone(displayReasoningEffort)
    : "none";
  const model = renderInvocationModelChip(modelValue, {
    t,
    hasMismatch: modelHasMismatch,
    className: "max-w-full",
    textClassName: "font-mono",
    iconClassName: "h-3.5 w-3.5",
    presentation: grouped ? "embedded" : "standalone",
    testId: testId ? `${testId}-model` : undefined,
  });

  if (grouped) {
    return (
      <div
        data-testid={testId}
        data-model-context-grouped="true"
        className={cn(
          "inline-flex h-5 min-w-0 max-w-full items-center gap-[3px] overflow-hidden rounded-md border border-base-300/75 bg-base-200/58 px-1 leading-none",
          className,
        )}
        title={modelLabel}
        aria-label={modelAriaLabel}
        role="group"
      >
        <span
          data-testid={modelTestId}
          data-model-context-part="model"
          className={cn(
            "flex h-5 max-w-full items-center overflow-hidden text-base-content/72",
            hasModelMismatch || resolveModelIdentityGenerationLabel(modelValue) != null
              ? "min-w-max shrink-0"
              : "min-w-0 flex-auto",
          )}
        >
          <CompactModelIdentity
            model={hasModelMismatch ? compactRequestModel : modelValue}
            side={hasModelMismatch ? "request" : "single"}
            testId={modelTestId}
          />
          {hasModelMismatch ? (
            <span
              className="mx-0.5 inline-flex h-4 w-3 shrink-0 items-center justify-center text-base-content/55"
              title={t("table.model.routingMismatchTitle")}
              aria-label={t("table.model.routingMismatchAria")}
              data-model-context-part="routing"
              data-testid={
                testId ? `${testId.replace(/-context$/, "")}-routing-indicator` : undefined
              }
              role="img"
            >
              <AppIcon name="arrow-right-bold" className="h-3 w-3" aria-hidden />
            </span>
          ) : null}
          {hasModelMismatch ? (
            <CompactModelIdentity model={modelValue} side="response" testId={modelTestId} />
          ) : null}
        </span>
        {hasReasoningEffort ? (
          <span
            data-testid={testId ? `${testId}-reasoning-effort` : undefined}
            data-model-context-part="reasoning-effort"
            data-reasoning-effort-tone={reasoningTone}
            className={cn(
              "flex min-h-5 min-w-0 items-center text-xs font-semibold",
              REASONING_EFFORT_CONTEXT_TONE_CLASSNAMES[reasoningTone],
            )}
            title={displayReasoningEffort}
          >
            <span
              data-model-context-part="reasoning-effort-text"
              className="truncate whitespace-nowrap"
            >
              {displayReasoningEffort}
            </span>
          </span>
        ) : null}
        {fastIndicator ? (
          <span
            data-model-context-part="fast"
            className="flex w-[7px] shrink-0 items-center justify-center"
          >
            {fastIndicator}
          </span>
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
      aria-label={modelAriaLabel}
      role="group"
    >
      <span data-testid={modelTestId} className="min-w-0">
        {model}
      </span>
      <span className="shrink-0 text-base-content/28">·</span>
      <InvocationReasoningEffortChip
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
