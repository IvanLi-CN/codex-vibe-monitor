import { cva, type VariantProps } from 'class-variance-authority'

export const segmentedControlItemVariants = cva('segmented-control-item', {
  variants: {
    size: {
      compact: 'min-h-10 px-2.5 text-sm sm:px-3.5',
      default: 'min-h-10 px-3.5 text-sm',
      nav: 'min-h-10 px-3.5 text-sm',
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
