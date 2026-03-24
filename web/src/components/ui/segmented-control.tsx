import * as React from 'react'
import { Slot } from '@radix-ui/react-slot'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '../../lib/utils'

const segmentedControlItemVariants = cva('segmented-control-item', {
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

type SegmentedControlItemSize = NonNullable<VariantProps<typeof segmentedControlItemVariants>['size']>

const SegmentedControlSizeContext = React.createContext<SegmentedControlItemSize>('default')

export interface SegmentedControlProps extends React.HTMLAttributes<HTMLDivElement> {
  size?: SegmentedControlItemSize
}

const SegmentedControl = React.forwardRef<HTMLDivElement, SegmentedControlProps>(
  ({ className, size = 'default', ...props }, ref) => (
    <SegmentedControlSizeContext.Provider value={size}>
      <div ref={ref} className={cn('segmented-control', className)} {...props} />
    </SegmentedControlSizeContext.Provider>
  ),
)
SegmentedControl.displayName = 'SegmentedControl'

export interface SegmentedControlItemProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    Omit<VariantProps<typeof segmentedControlItemVariants>, 'active'> {
  active?: boolean
  asChild?: boolean
}

const SegmentedControlItem = React.forwardRef<HTMLButtonElement, SegmentedControlItemProps>(
  ({ active = false, asChild = false, className, size, type, ...props }, ref) => {
    const Comp = asChild ? Slot : 'button'
    const inheritedSize = React.useContext(SegmentedControlSizeContext)
    const resolvedSize = size ?? inheritedSize
    const nextProps = asChild ? props : { type: type ?? 'button', ...props }

    return (
      <Comp
        ref={ref}
        data-active={active}
        className={cn(segmentedControlItemVariants({ size: resolvedSize, active }), className)}
        {...nextProps}
      />
    )
  },
)
SegmentedControlItem.displayName = 'SegmentedControlItem'

export { SegmentedControl, SegmentedControlItem, segmentedControlItemVariants }
