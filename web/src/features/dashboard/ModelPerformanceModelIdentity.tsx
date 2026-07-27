import { cn } from "../../lib/utils";
import {
  getReasoningEffortTone,
  type ReasoningEffortTone,
} from "../invocations/invocation-table-reasoning";
import { ModelIdentity, resolveModelIdentityIcon } from "../shared/ModelIdentity";

const EFFORT_TEXT_CLASSNAMES: Record<ReasoningEffortTone, string> = {
  none: "text-base-content/68",
  minimal: "tone-ink-info",
  low: "tone-ink-info",
  medium: "tone-ink-primary",
  high: "tone-ink-warning",
  xhigh: "tone-ink-error",
  unknown: "text-base-content/62",
};

const EFFORT_MARKER_CLASSNAMES: Record<ReasoningEffortTone, string> = {
  none: "bg-base-content/34",
  minimal: "bg-info/65",
  low: "bg-info/80",
  medium: "bg-primary/80",
  high: "bg-warning/85",
  xhigh: "bg-error/85",
  unknown: "bg-base-content/38",
};

export function ModelPerformanceModelIdentity({
  model,
  effort,
  className,
  modelClassName,
  testId,
}: {
  model: string;
  effort: string;
  className?: string;
  modelClassName?: string;
  testId?: string;
}) {
  const tone = getReasoningEffortTone(effort);
  const hasModelIcon = resolveModelIdentityIcon(model) !== null;
  const accessibleLabel = `${model} · ${effort}`;

  return (
    <span
      data-testid={testId}
      data-model-context-display="name-and-badge"
      className={cn("flex min-w-0 max-w-full items-center gap-1.5 whitespace-nowrap", className)}
      title={accessibleLabel}
    >
      <span className="sr-only">{accessibleLabel}</span>
      <span className="contents" aria-hidden>
        <span
          data-testid={testId ? `${testId}-name` : undefined}
          className={cn("min-w-0 truncate font-mono", modelClassName)}
          title={model}
        >
          {model}
        </span>
        <span
          data-testid={testId ? `${testId}-badge` : undefined}
          className="inline-flex h-6 shrink-0 items-stretch overflow-hidden rounded-md border border-base-300/75 bg-base-200/58 leading-none"
        >
          {hasModelIcon ? (
            <>
              <span className="flex w-6 shrink-0 items-center justify-center text-base-content/72">
                <ModelIdentity model={model} iconClassName="h-3.5 w-3.5" />
              </span>
              <span className="w-px shrink-0 bg-base-300/75" />
            </>
          ) : null}
          <span
            data-testid={testId ? `${testId}-effort` : undefined}
            data-reasoning-effort-tone={tone}
            className={cn(
              "flex items-center gap-1 px-1.5 text-xs font-semibold",
              EFFORT_TEXT_CLASSNAMES[tone],
            )}
          >
            <span className={cn("h-1 w-1 shrink-0 rounded-full", EFFORT_MARKER_CLASSNAMES[tone])} />
            <span className="max-w-20 truncate">{effort}</span>
          </span>
        </span>
      </span>
    </span>
  );
}
