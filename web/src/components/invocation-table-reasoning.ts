export const REASONING_EFFORT_TONE_CLASSNAMES = {
  none: 'border-base-300/90 bg-base-200/80 text-base-content/70',
  minimal: 'border-info/25 bg-info/10 text-info/85',
  low: 'border-info/45 bg-info/10 text-info',
  medium: 'border-primary/40 bg-primary/10 text-primary',
  high: 'border-warning/45 bg-warning/15 text-warning',
  xhigh: 'border-error/45 bg-error/15 text-error',
  unknown: 'border-dashed border-base-content/20 bg-base-200/55 text-base-content/75',
} as const

export type ReasoningEffortTone = keyof typeof REASONING_EFFORT_TONE_CLASSNAMES

export function getReasoningEffortTone(value: string): ReasoningEffortTone {
  const normalized = value.trim().toLowerCase()
  if (Object.hasOwn(REASONING_EFFORT_TONE_CLASSNAMES, normalized) && normalized !== 'unknown') {
    return normalized as Exclude<ReasoningEffortTone, 'unknown'>
  }
  return 'unknown'
}
