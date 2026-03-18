import { useState, type ReactNode } from 'react'
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
const INLINE_ARROW_PADDING = 16
const INLINE_ARROW_WIDTH = 8
const INLINE_ARROW_HEIGHT = 6
const CORNER_ARROW_PADDING = 18
const CORNER_ARROW_WIDTH = 14
const CORNER_ARROW_HEIGHT = 8

interface FloatingFieldBubbleProps {
  message: string
  variant?: BubbleVariant
  className?: string
  placement?: FloatingFieldBubblePlacement
  anchor?: ReactNode
  anchorClassName?: string
}

function statusRole(variant: BubbleVariant) {
  return variant === 'error' ? 'alert' : 'status'
}

export function FloatingFieldBubble({
  message,
  variant = 'error',
  className,
  placement = 'input-corner',
  anchor,
  anchorClassName,
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
            aria-hidden={anchor ? undefined : true}
            className={cn(
              anchor
                ? 'inline-flex shrink-0 items-center'
                : 'inline-flex h-4 w-4 shrink-0 translate-y-0.5',
              anchorClassName,
            )}
          >
            {anchor}
          </span>
        </PopoverAnchor>
        <PopoverContent
          data-theme={portalTheme}
          style={bubbleSurfaceStyle(variant, portalTheme)}
          role={role}
          aria-live="polite"
          onOpenAutoFocus={(event) => event.preventDefault()}
          onCloseAutoFocus={(event) => event.preventDefault()}
          side="left"
          align="center"
          sideOffset={2}
          arrowPadding={INLINE_ARROW_PADDING}
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
            width={INLINE_ARROW_WIDTH}
            height={INLINE_ARROW_HEIGHT}
            className={bubbleArrowClassName()}
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
        onOpenAutoFocus={(event) => event.preventDefault()}
        onCloseAutoFocus={(event) => event.preventDefault()}
        side="bottom"
        align="end"
        sideOffset={4}
        avoidCollisions
        collisionPadding={12}
        sticky="partial"
        arrowPadding={CORNER_ARROW_PADDING}
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
          width={CORNER_ARROW_WIDTH}
          height={CORNER_ARROW_HEIGHT}
          className={bubbleArrowClassName()}
          style={bubbleArrowStyle(variant, portalTheme)}
        />
      </PopoverContent>
    </Popover>
  )
}
