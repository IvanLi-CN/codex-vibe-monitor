import { cva, type VariantProps } from 'class-variance-authority'

const bubbleVariants = cva(
  [
    'z-[92] max-w-[min(22rem,calc(100vw-1rem))] rounded-2xl border-0 px-3.5 py-2',
    'text-left text-xs font-medium leading-5 outline-none',
    'shadow-[0_24px_48px_-30px_rgba(15,23,42,0.42),0_14px_30px_-24px_rgba(15,23,42,0.25)]',
    'data-[state=open]:animate-in data-[state=closed]:animate-out',
    'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
    'data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95',
    'data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2',
    'data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2',
  ].join(' '),
  {
    variants: {
      variant: {
        neutral:
          'bg-[color-mix(in_oklab,oklch(var(--color-base-100))_88%,oklch(var(--color-primary))_12%)] text-base-content data-[theme=vibe-dark]:bg-[color-mix(in_oklab,oklch(var(--color-base-200))_86%,oklch(var(--color-primary))_14%)]',
        info: 'bg-info/40 text-base-content',
        success: 'bg-success/40 text-base-content',
        warning:
          'bg-[color-mix(in_oklab,oklch(var(--color-warning))_72%,oklch(var(--color-warning-content)))] text-base-content data-[theme=vibe-dark]:bg-[color-mix(in_oklab,oklch(var(--color-base-200))_62%,oklch(var(--color-warning))_38%)]',
        error: 'bg-error/40 text-base-content',
      },
    },
    defaultVariants: {
      variant: 'neutral',
    },
  },
)

const bubbleArrowVariants = cva(
  '',
  {
    variants: {
      variant: {
        neutral:
          '[&_path]:fill-[color-mix(in_oklab,oklch(var(--color-base-100))_88%,oklch(var(--color-primary))_12%)] [&_polygon]:fill-[color-mix(in_oklab,oklch(var(--color-base-100))_88%,oklch(var(--color-primary))_12%)] data-[theme=vibe-dark]:[&_path]:fill-[color-mix(in_oklab,oklch(var(--color-base-200))_86%,oklch(var(--color-primary))_14%)] data-[theme=vibe-dark]:[&_polygon]:fill-[color-mix(in_oklab,oklch(var(--color-base-200))_86%,oklch(var(--color-primary))_14%)]',
        info:
          '[&_path]:fill-[oklch(var(--color-info)/0.40)] [&_polygon]:fill-[oklch(var(--color-info)/0.40)]',
        success:
          '[&_path]:fill-[oklch(var(--color-success)/0.40)] [&_polygon]:fill-[oklch(var(--color-success)/0.40)]',
        warning:
          '[&_path]:fill-[color-mix(in_oklab,oklch(var(--color-warning))_72%,oklch(var(--color-warning-content)))] [&_polygon]:fill-[color-mix(in_oklab,oklch(var(--color-warning))_72%,oklch(var(--color-warning-content)))] data-[theme=vibe-dark]:[&_path]:fill-[color-mix(in_oklab,oklch(var(--color-base-200))_62%,oklch(var(--color-warning))_38%)] data-[theme=vibe-dark]:[&_polygon]:fill-[color-mix(in_oklab,oklch(var(--color-base-200))_62%,oklch(var(--color-warning))_38%)]',
        error:
          '[&_path]:fill-[oklch(var(--color-error)/0.40)] [&_polygon]:fill-[oklch(var(--color-error)/0.40)]',
      },
    },
    defaultVariants: {
      variant: 'neutral',
    },
  },
)

export type BubbleVariant = NonNullable<VariantProps<typeof bubbleVariants>['variant']>

export function bubbleContentClassName(variant?: BubbleVariant) {
  return bubbleVariants({ variant })
}

export function bubbleArrowClassName(variant?: BubbleVariant) {
  return bubbleArrowVariants({ variant })
}
