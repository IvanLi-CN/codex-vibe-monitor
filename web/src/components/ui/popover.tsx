import * as React from 'react'
import * as PopoverPrimitive from '@radix-ui/react-popover'
import { cn } from '../../lib/utils'
import {
  useOverlayHostElement,
  useResolvedOverlayContainer,
} from './use-overlay-host'
import { OverlayHostProvider } from './overlay-host'

const Popover = PopoverPrimitive.Root

const PopoverTrigger = PopoverPrimitive.Trigger

const PopoverAnchor = PopoverPrimitive.Anchor
const PopoverArrow = PopoverPrimitive.Arrow

const PopoverContent = React.forwardRef<
  React.ElementRef<typeof PopoverPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof PopoverPrimitive.Content> & {
    container?: HTMLElement | null
  }
>(({ className, align = 'center', sideOffset = 4, container, ...props }, ref) => {
  const resolvedContainer = useResolvedOverlayContainer(container)
  const { hostElement, ref: contentRef } = useOverlayHostElement(ref)
  const hostValue = hostElement ?? (container === undefined ? resolvedContainer : container)

  return (
    <PopoverPrimitive.Portal container={resolvedContainer ?? undefined}>
      <OverlayHostProvider value={hostValue}>
        <PopoverPrimitive.Content
          ref={contentRef}
          align={align}
          sideOffset={sideOffset}
          className={cn(
            'z-50 w-72 rounded-md border border-base-300 bg-base-100 p-1 text-base-content shadow-md outline-none',
            'data-[state=open]:animate-in data-[state=closed]:animate-out',
            'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
            'data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95',
            'data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2',
            'data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2',
            className,
          )}
          {...props}
        />
      </OverlayHostProvider>
    </PopoverPrimitive.Portal>
  )
})
PopoverContent.displayName = PopoverPrimitive.Content.displayName

export { Popover, PopoverTrigger, PopoverContent, PopoverAnchor, PopoverArrow }
