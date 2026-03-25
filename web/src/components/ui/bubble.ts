import type { CSSProperties } from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import {
  floatingSurfaceArrowStyle,
  floatingSurfaceStyle,
  type FloatingSurfaceTheme,
  type FloatingSurfaceTone,
} from './floating-surface'

const bubbleVariants = cva(
  [
    'z-[92] max-w-[min(22rem,calc(100vw-1rem))] rounded-xl border px-3.5 py-2 !shadow-none',
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
        info: 'text-base-content',
        success: 'text-base-content',
        warning: 'text-base-content',
        error: 'text-base-content',
      },
    },
    defaultVariants: {
      variant: 'neutral',
    },
  },
)

export type BubbleVariant = NonNullable<VariantProps<typeof bubbleVariants>['variant']>
export type BubbleTheme = FloatingSurfaceTheme

function bubbleTone(variant: BubbleVariant): FloatingSurfaceTone {
  return variant
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
  return floatingSurfaceStyle(bubbleTone(variant), theme)
}

export function bubbleArrowStyle(
  variant: BubbleVariant = 'neutral',
  theme?: BubbleTheme,
): CSSProperties {
  return floatingSurfaceArrowStyle(bubbleTone(variant), theme)
}
