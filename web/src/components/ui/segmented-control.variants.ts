import { cva, type VariantProps } from 'class-variance-authority'

export const segmentedControlItemVariants = cva('segmented-control-item', {
  variants: {
    size: {
      compact: 'h-8 px-2 text-sm sm:px-3',
      default: 'h-8 px-3 text-sm',
      nav: 'h-8 px-3.5 text-sm',
    },
    active: {
      true: 'segmented-control-item--active font-semibold',
      false: '',
    },
  },
  defaultVariants: {
    size: 'default',
    active: false,
  },
})

export type SegmentedControlItemSize = NonNullable<VariantProps<typeof segmentedControlItemVariants>['size']>
