import type { CSSProperties } from 'react'
import { cva, type VariantProps } from 'class-variance-authority'

const bubbleVariants = cva(
  [
    'z-[92] max-w-[min(22rem,calc(100vw-1rem))] rounded-2xl border-0 px-3.5 py-2 !shadow-none',
    'text-left text-xs font-medium leading-5 outline-none',
    'data-[state=open]:animate-in data-[state=closed]:animate-out',
    'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
    'data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95',
    'data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2',
    'data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2',
  ].join(' '),
  {
    variants: {
      variant: {
        neutral: 'text-base-content',
        info: 'bg-info/40 text-base-content',
        success: 'bg-success/40 text-base-content',
        warning: 'text-base-content',
        error: 'bg-error/40 text-base-content',
      },
    },
    defaultVariants: {
      variant: 'neutral',
    },
  },
)

export type BubbleVariant = NonNullable<VariantProps<typeof bubbleVariants>['variant']>
type BubbleTheme = 'vibe-light' | 'vibe-dark' | undefined
const bubbleShadowFilter =
  'drop-shadow(0 24px 32px rgba(15, 23, 42, 0.14)) drop-shadow(0 12px 18px rgba(15, 23, 42, 0.10))'

function bubbleSurfaceColor(variant: BubbleVariant, theme: BubbleTheme) {
  const isDark = theme === 'vibe-dark'

  switch (variant) {
    case 'neutral':
      return isDark
        ? 'color-mix(in oklab, oklch(var(--color-base-200)) 86%, oklch(var(--color-primary)) 14%)'
        : 'color-mix(in oklab, oklch(var(--color-base-100)) 88%, oklch(var(--color-primary)) 12%)'
    case 'info':
      return 'oklch(var(--color-info) / 0.40)'
    case 'success':
      return 'oklch(var(--color-success) / 0.40)'
    case 'warning':
      return isDark
        ? 'color-mix(in oklab, oklch(var(--color-base-200)) 62%, oklch(var(--color-warning)) 38%)'
        : 'color-mix(in oklab, oklch(var(--color-warning)) 72%, oklch(var(--color-warning-content)))'
    case 'error':
      return 'oklch(var(--color-error) / 0.40)'
    default:
      return 'oklch(var(--color-error) / 0.40)'
  }
}

export function bubbleContentClassName(variant?: BubbleVariant) {
  return bubbleVariants({ variant })
}

export function bubbleArrowClassName(variant?: BubbleVariant) {
  return ''
}

export function bubbleSurfaceStyle(
  variant: BubbleVariant = 'neutral',
  theme?: BubbleTheme,
): CSSProperties {
  return {
    backgroundColor: bubbleSurfaceColor(variant, theme),
    filter: bubbleShadowFilter,
  }
}

export function bubbleArrowStyle(
  variant: BubbleVariant = 'neutral',
  theme?: BubbleTheme,
): CSSProperties {
  return {
    fill: bubbleSurfaceColor(variant, theme),
  }
}
