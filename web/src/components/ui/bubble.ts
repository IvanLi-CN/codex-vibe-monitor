import type { CSSProperties } from 'react'
import { cva, type VariantProps } from 'class-variance-authority'

const bubbleVariants = cva(
  [
    'z-[92] max-w-[min(22rem,calc(100vw-1rem))] rounded-xl border-0 px-3.5 py-2 !shadow-none',
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
export type BubbleTheme = 'vibe-light' | 'vibe-dark' | undefined
const bubbleShadowFilter =
  'drop-shadow(0 20px 30px rgba(15, 23, 42, 0.12)) drop-shadow(0 10px 18px rgba(15, 23, 42, 0.08))'
const bubbleBackdropFilter = 'blur(18px) saturate(165%)'

function bubbleSurfaceColor(variant: BubbleVariant, theme: BubbleTheme) {
  const isDark = theme === 'vibe-dark'

  switch (variant) {
    case 'neutral':
      return isDark
        ? 'color-mix(in oklab, oklch(var(--color-base-200) / 0.78) 84%, oklch(var(--color-primary) / 0.22) 16%)'
        : 'color-mix(in oklab, oklch(var(--color-base-100) / 0.64) 84%, oklch(var(--color-primary) / 0.18) 16%)'
    case 'info':
      return isDark
        ? 'color-mix(in oklab, oklch(var(--color-base-200) / 0.66) 54%, oklch(var(--color-info) / 0.34) 46%)'
        : 'color-mix(in oklab, oklch(var(--color-base-100) / 0.58) 54%, oklch(var(--color-info) / 0.28) 46%)'
    case 'success':
      return isDark
        ? 'color-mix(in oklab, oklch(var(--color-base-200) / 0.66) 56%, oklch(var(--color-success) / 0.34) 44%)'
        : 'color-mix(in oklab, oklch(var(--color-base-100) / 0.58) 56%, oklch(var(--color-success) / 0.28) 44%)'
    case 'warning':
      return isDark
        ? 'color-mix(in oklab, oklch(var(--color-base-200) / 0.7) 62%, oklch(var(--color-warning) / 0.32) 38%)'
        : 'color-mix(in oklab, oklch(var(--color-base-100) / 0.62) 62%, oklch(var(--color-warning) / 0.26) 38%)'
    case 'error':
      return isDark
        ? 'color-mix(in oklab, oklch(var(--color-base-200) / 0.66) 56%, oklch(var(--color-error) / 0.34) 44%)'
        : 'color-mix(in oklab, oklch(var(--color-base-100) / 0.58) 54%, oklch(var(--color-error) / 0.28) 46%)'
    default:
      return isDark
        ? 'color-mix(in oklab, oklch(var(--color-base-200) / 0.66) 56%, oklch(var(--color-error) / 0.34) 44%)'
        : 'color-mix(in oklab, oklch(var(--color-base-100) / 0.58) 54%, oklch(var(--color-error) / 0.28) 46%)'
  }
}

export function bubbleContentClassName(variant?: BubbleVariant) {
  return bubbleVariants({ variant })
}

export function bubbleArrowClassName() {
  return ''
}

export function bubbleSurfaceStyle(
  variant: BubbleVariant = 'neutral',
  theme?: BubbleTheme,
): CSSProperties {
  return {
    backgroundColor: bubbleSurfaceColor(variant, theme),
    filter: bubbleShadowFilter,
    backdropFilter: bubbleBackdropFilter,
    WebkitBackdropFilter: bubbleBackdropFilter,
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
