import * as React from 'react'
import * as TooltipPrimitive from '@radix-ui/react-tooltip'
import { cn } from '../../lib/utils'

const LONG_PRESS_DELAY_MS = 360

interface TooltipProps {
  content: React.ReactNode
  children: React.ReactNode
  className?: string
  contentClassName?: string
  side?: 'top' | 'right' | 'bottom' | 'left'
  sideOffset?: number
  open?: boolean
}

export function Tooltip({
  content,
  children,
  className,
  contentClassName,
  side = 'top',
  sideOffset = 10,
  open,
}: TooltipProps) {
  const longPressTimerRef = React.useRef<number | null>(null)
  const [hoverOpen, setHoverOpen] = React.useState(false)
  const [longPressOpen, setLongPressOpen] = React.useState(false)

  const clearLongPressTimer = React.useCallback(() => {
    if (longPressTimerRef.current != null) {
      window.clearTimeout(longPressTimerRef.current)
      longPressTimerRef.current = null
    }
  }, [])

  React.useEffect(() => () => clearLongPressTimer(), [clearLongPressTimer])

  const handlePointerDown = React.useCallback((event: React.PointerEvent<HTMLSpanElement>) => {
    if (open !== undefined || event.button !== 0) return
    clearLongPressTimer()
    longPressTimerRef.current = window.setTimeout(() => {
      setLongPressOpen(true)
      longPressTimerRef.current = null
    }, LONG_PRESS_DELAY_MS)
  }, [clearLongPressTimer, open])

  const handlePointerRelease = React.useCallback(() => {
    if (open !== undefined) return
    clearLongPressTimer()
    setLongPressOpen(false)
  }, [clearLongPressTimer, open])

  const resolvedOpen = open ?? (hoverOpen || longPressOpen)

  return (
    <TooltipPrimitive.Provider delayDuration={120}>
      <TooltipPrimitive.Root open={resolvedOpen}>
        <TooltipPrimitive.Trigger asChild>
          <span
            className={cn('inline-flex', className)}
            onBlur={open === undefined ? () => setHoverOpen(false) : undefined}
            onFocus={open === undefined ? () => setHoverOpen(true) : undefined}
            onMouseEnter={open === undefined ? () => setHoverOpen(true) : undefined}
            onMouseLeave={open === undefined ? () => setHoverOpen(false) : undefined}
            onPointerDownCapture={handlePointerDown}
            onPointerDown={handlePointerDown}
            onPointerUpCapture={handlePointerRelease}
            onPointerUp={handlePointerRelease}
            onPointerCancelCapture={handlePointerRelease}
            onPointerCancel={handlePointerRelease}
            onPointerLeave={handlePointerRelease}
          >
            {children}
          </span>
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
