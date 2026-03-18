import { useState } from 'react'
import {
  bubbleArrowClassName,
  bubbleArrowStyle,
  bubbleContentClassName,
  bubbleSurfaceStyle,
  type BubbleVariant,
} from './bubble'
import { Popover, PopoverAnchor, PopoverArrow, PopoverContent } from './popover'
import { cn } from '../../lib/utils'
import { usePortaledTheme } from './use-portaled-theme'

export type FloatingFieldBubblePlacement = 'input-corner' | 'label-inline'

interface FloatingFieldBubbleProps {
  message: string
  variant?: BubbleVariant
  className?: string
  placement?: FloatingFieldBubblePlacement
}

function statusRole(variant: BubbleVariant) {
  return variant === 'error' ? 'alert' : 'status'
}

export function FloatingFieldBubble({
  message,
  variant = 'error',
  className,
  placement = 'input-corner',
}: FloatingFieldBubbleProps) {
  const role = statusRole(variant)
  const [anchorElement, setAnchorElement] = useState<HTMLSpanElement | null>(null)
  const portalTheme = usePortaledTheme(anchorElement)

  if (placement === 'label-inline') {
    return (
      <Popover open modal={false}>
        <PopoverAnchor asChild>
          <span
            ref={setAnchorElement}
            aria-hidden
            className="inline-flex h-4 w-4 shrink-0 translate-y-0.5"
          />
        </PopoverAnchor>
        <PopoverContent
          data-theme={portalTheme}
          style={bubbleSurfaceStyle(variant, portalTheme)}
          role={role}
          aria-live="polite"
          side="left"
          align="center"
          sideOffset={2}
          avoidCollisions
          collisionPadding={12}
          sticky="partial"
          className={cn(
            bubbleContentClassName(variant),
            'pointer-events-none w-[min(20rem,calc(100vw-1rem))]',
            className,
          )}
        >
          {message}
          <PopoverArrow
            data-theme={portalTheme}
            data-bubble-arrow="true"
            width={18}
            height={10}
            className={bubbleArrowClassName(variant)}
            style={{
              ...bubbleArrowStyle(variant, portalTheme),
              transform: 'translateX(-1px)',
            }}
          />
        </PopoverContent>
      </Popover>
    )
  }

  return (
    <Popover open modal={false}>
      <PopoverAnchor asChild>
        <span
          ref={setAnchorElement}
          aria-hidden
          className="pointer-events-none absolute right-3 top-full h-0 w-0"
        />
      </PopoverAnchor>
      <PopoverContent
        data-theme={portalTheme}
        style={bubbleSurfaceStyle(variant, portalTheme)}
        role={role}
        aria-live="polite"
        side="bottom"
        align="end"
        sideOffset={4}
        avoidCollisions
        collisionPadding={12}
        sticky="partial"
        arrowPadding={14}
        className={cn(
          bubbleContentClassName(variant),
          'pointer-events-none w-[min(20rem,calc(100vw-1rem))]',
          className,
        )}
      >
        {message}
        <PopoverArrow
          data-theme={portalTheme}
          data-bubble-arrow="true"
          width={16}
          height={9}
          className={bubbleArrowClassName(variant)}
          style={{
            ...bubbleArrowStyle(variant, portalTheme),
            transform: 'translateY(1px)',
          }}
        />
      </PopoverContent>
    </Popover>
  )
}
