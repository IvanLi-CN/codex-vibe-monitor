import { Icon } from '@iconify/react'
import { useEffect, useId, useRef, useState } from 'react'
import { cn } from '../../lib/utils'

interface InfoTooltipProps {
  content: string
  label: string
  className?: string
}

export function InfoTooltip({ content, label, className }: InfoTooltipProps) {
  const [open, setOpen] = useState(false)
  const [pinned, setPinned] = useState(false)
  const [shiftX, setShiftX] = useState(0)
  const [placement, setPlacement] = useState<'top' | 'bottom'>('bottom')
  const rootRef = useRef<HTMLSpanElement | null>(null)
  const tooltipRef = useRef<HTMLSpanElement | null>(null)
  const tooltipId = useId()

  useEffect(() => {
    if (!open) return
    const handlePointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setPinned(false)
        setOpen(false)
      }
    }
    document.addEventListener('pointerdown', handlePointerDown)
    return () => document.removeEventListener('pointerdown', handlePointerDown)
  }, [open])

  useEffect(() => {
    if (!open) {
      setShiftX(0)
      setPlacement('bottom')
      return
    }

    const update = () => {
      const tooltipEl = tooltipRef.current
      const rootEl = rootRef.current
      if (!tooltipEl || !rootEl) return
      const rect = tooltipEl.getBoundingClientRect()
      const rootRect = rootEl.getBoundingClientRect()
      const margin = 8
      const gap = 8
      let nextShift = 0
      const overflowLeft = margin - rect.left
      if (overflowLeft > 0) nextShift += overflowLeft
      const overflowRight = rect.right - (window.innerWidth - margin)
      if (overflowRight > 0) nextShift -= overflowRight
      setShiftX(nextShift)

      const fitsBelow = rootRect.bottom + gap + rect.height <= window.innerHeight - margin
      const fitsAbove = rootRect.top - gap - rect.height >= margin
      setPlacement(!fitsBelow && fitsAbove ? 'top' : 'bottom')
    }

    update()
    window.addEventListener('resize', update)
    // Capture scroll events from any scroll container so edge tooltips remain readable.
    window.addEventListener('scroll', update, true)
    return () => {
      window.removeEventListener('resize', update)
      window.removeEventListener('scroll', update, true)
    }
  }, [open])

  return (
    <span
      ref={rootRef}
      className={cn('relative inline-flex items-center', className)}
      onMouseEnter={() => {
        if (!pinned) setOpen(true)
      }}
      onMouseLeave={() => {
        if (!pinned) setOpen(false)
      }}
    >
      <button
        type="button"
        aria-label={label}
        aria-describedby={open ? tooltipId : undefined}
        className="inline-flex h-5 w-5 items-center justify-center rounded-full text-[inherit] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        onClick={() => {
          setPinned((current) => {
            const nextPinned = !current
            setOpen(nextPinned)
            return nextPinned
          })
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => {
          if (!pinned) setOpen(false)
        }}
      >
        <span className="inline-flex h-[18px] w-[18px] items-center justify-center text-[inherit]" aria-hidden>
          <Icon icon="mdi:help-circle-outline" className="h-full w-full" />
        </span>
      </button>
      <span
        ref={tooltipRef}
        id={tooltipId}
        role="tooltip"
        aria-hidden={open ? 'false' : 'true'}
        style={{ transform: `translateX(-50%) translateX(${shiftX}px)` }}
        className={cn(
          'absolute left-1/2 z-20 w-64 max-w-[calc(100vw-2rem)] rounded-xl border border-base-300/80 bg-base-100/95 px-3 py-2 text-left text-xs leading-5 text-base-content shadow-lg backdrop-blur transition-opacity',
          placement === 'top' ? 'bottom-[calc(100%+0.45rem)]' : 'top-[calc(100%+0.45rem)]',
          open ? 'pointer-events-auto opacity-100' : 'pointer-events-none opacity-0',
        )}
      >
        {content}
      </span>
    </span>
  )
}
