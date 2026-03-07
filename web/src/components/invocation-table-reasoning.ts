export const REASONING_EFFORT_TONE_CLASSNAMES = {
  none: 'border-base-300/90 bg-base-200/80 text-base-content/70',
  minimal: 'border-info/25 bg-info/8 text-info/85',
  low: 'border-info/38 bg-info/12 text-info',
  medium: 'border-primary/40 bg-primary/12 text-primary',
  high: 'border-warning/42 bg-warning/18 text-warning',
  xhigh: 'border-error/42 bg-error/15 text-error',
  unknown: 'border-dashed border-base-content/20 bg-base-200/55 text-base-content/75',
} as const

export type ReasoningEffortTone = keyof typeof REASONING_EFFORT_TONE_CLASSNAMES

export function getReasoningEffortTone(value: string): ReasoningEffortTone {
  const normalized = value.trim().toLowerCase()
  if (normalized in REASONING_EFFORT_TONE_CLASSNAMES && normalized !== 'unknown') {
    return normalized as Exclude<ReasoningEffortTone, 'unknown'>
  }
  return 'unknown'
}
