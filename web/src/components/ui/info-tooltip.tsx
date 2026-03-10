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
  const rootRef = useRef<HTMLSpanElement | null>(null)
  const tooltipId = useId()

  useEffect(() => {
    if (!open) return
    const handlePointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpen(false)
      }
    }
    document.addEventListener('pointerdown', handlePointerDown)
    return () => document.removeEventListener('pointerdown', handlePointerDown)
  }, [open])

  return (
    <span
      ref={rootRef}
      className={cn('relative inline-flex items-center', className)}
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
    >
      <button
        type="button"
        aria-label={label}
        aria-describedby={open ? tooltipId : undefined}
        className="inline-flex h-5 w-5 items-center justify-center rounded-full text-[inherit] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        onClick={() => setOpen((current) => !current)}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
      >
        <Icon icon="mdi:help-circle-outline" className="h-4.5 w-4.5 text-[inherit]" aria-hidden />
      </button>
      <span
        id={tooltipId}
        role="tooltip"
        aria-hidden={open ? 'false' : 'true'}
        className={cn(
          'pointer-events-none absolute right-0 top-[calc(100%+0.45rem)] z-20 w-64 rounded-xl border border-base-300/80 bg-base-100/95 px-3 py-2 text-left text-xs leading-5 text-base-content shadow-lg backdrop-blur',
          open ? 'opacity-100' : 'opacity-0',
        )}
      >
        {content}
      </span>
    </span>
  )
}
