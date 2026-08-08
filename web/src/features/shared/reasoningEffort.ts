export const REASONING_EFFORT_FALLBACK = "—";

export const REASONING_EFFORT_TONE_CLASSNAMES = {
  none: "border-base-300/90 bg-base-200/80 text-base-content/70",
  minimal: "border-info/25 bg-info/10 text-info/85",
  low: "border-info/45 bg-info/10 text-info",
  medium: "border-primary/40 bg-primary/10 text-primary",
  high: "border-warning/45 bg-warning/15 text-warning",
  xhigh: "border-warning/65 bg-warning/25 text-warning",
  max: "border-error/45 bg-error/15 text-error",
  ultra: "border-error/70 bg-error/25 text-error",
  unknown: "border-dashed border-base-content/20 bg-base-200/55 text-base-content/75",
} as const;

export type ReasoningEffortTone = keyof typeof REASONING_EFFORT_TONE_CLASSNAMES;

export function formatReasoningEffort(value: string | null | undefined): string {
  const normalized = value?.trim().toLowerCase();
  return normalized || REASONING_EFFORT_FALLBACK;
}

export function getReasoningEffortTone(value: string | null | undefined): ReasoningEffortTone {
  const normalized = formatReasoningEffort(value);
  if (Object.hasOwn(REASONING_EFFORT_TONE_CLASSNAMES, normalized)) {
    return normalized as Exclude<ReasoningEffortTone, "unknown">;
  }
  return "unknown";
}
