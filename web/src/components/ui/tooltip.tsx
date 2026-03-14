import * as React from 'react'
import * as TooltipPrimitive from '@radix-ui/react-tooltip'
import { cn } from '../../lib/utils'

interface TooltipProps {
  content: React.ReactNode
  children: React.ReactNode
  className?: string
  contentClassName?: string
  side?: 'top' | 'right' | 'bottom' | 'left'
  sideOffset?: number
}

export function Tooltip({
  content,
  children,
  className,
  contentClassName,
  side = 'top',
  sideOffset = 10,
}: TooltipProps) {
  return (
    <TooltipPrimitive.Provider delayDuration={120}>
      <TooltipPrimitive.Root>
        <TooltipPrimitive.Trigger asChild>
          <span className={cn('inline-flex', className)}>{children}</span>
        </TooltipPrimitive.Trigger>
        <TooltipPrimitive.Portal>
          <TooltipPrimitive.Content
            side={side}
            sideOffset={sideOffset}
            className={cn(
              'z-50 max-w-[min(20rem,calc(100vw-1rem))] rounded-xl border border-base-300/80 bg-base-100/96 px-3 py-2',
              'text-left text-xs text-base-content shadow-lg backdrop-blur-sm outline-none',
              'data-[state=delayed-open]:animate-in data-[state=closed]:animate-out',
              'data-[state=closed]:fade-out-0 data-[state=delayed-open]:fade-in-0',
              'data-[state=closed]:zoom-out-95 data-[state=delayed-open]:zoom-in-95',
              'data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2',
              'data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2',
              contentClassName,
            )}
          >
            {content}
            <TooltipPrimitive.Arrow className="fill-base-100/96 stroke-base-300/80 stroke-[0.6]" width={14} height={8} />
          </TooltipPrimitive.Content>
        </TooltipPrimitive.Portal>
      </TooltipPrimitive.Root>
    </TooltipPrimitive.Provider>
  )
}
