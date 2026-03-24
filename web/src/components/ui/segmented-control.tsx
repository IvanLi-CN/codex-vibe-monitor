import * as React from 'react'
import { Slot } from '@radix-ui/react-slot'
import { cn } from '../../lib/utils'
import { segmentedControlItemVariants, type SegmentedControlItemSize } from './segmented-control.variants'

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
    Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'size'> {
  active?: boolean
  size?: SegmentedControlItemSize
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

export { SegmentedControl, SegmentedControlItem }
