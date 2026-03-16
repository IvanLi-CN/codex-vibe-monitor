import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { AppIcon } from '../AppIcon'
import { Tooltip } from '../ui/tooltip'
import { cn } from '../../lib/utils'

const LONG_PRESS_DELAY_MS = 360

function buildMailboxTooltip(copyLabel: string, emailAddress: string) {
  return (
    <div className="flex max-w-full items-center gap-1.5">
      <span className="text-base-content/78">{copyLabel}</span>
      <code className="min-w-0 rounded-md bg-base-200/80 px-1.5 py-0.5 font-mono text-[11px] text-base-content">
        {emailAddress}
      </code>
    </div>
  )
}

interface OauthMailboxChipProps {
  emailAddress: string | null | undefined
  emptyLabel: string
  copyAriaLabel: string
  copyHintLabel: string
  copiedLabel: string
  tone?: 'idle' | 'copied'
  onCopy: () => void
  className?: string
}

export function OauthMailboxChip({
  emailAddress,
  emptyLabel,
  copyAriaLabel,
  copyHintLabel,
  copiedLabel,
  tone = 'idle',
  onCopy,
  className,
}: OauthMailboxChipProps) {
  const longPressTimerRef = useRef<number | null>(null)
  const [longPressOpen, setLongPressOpen] = useState(false)
  const [hoverOpen, setHoverOpen] = useState(false)

  useEffect(() => {
    return () => {
      if (longPressTimerRef.current != null) {
        window.clearTimeout(longPressTimerRef.current)
      }
    }
  }, [])

  const clearLongPressTimer = () => {
    if (longPressTimerRef.current != null) {
      window.clearTimeout(longPressTimerRef.current)
      longPressTimerRef.current = null
    }
  }

  const handlePointerDown = (event: ReactPointerEvent<HTMLButtonElement>) => {
    if (event.button !== 0) return
    clearLongPressTimer()
    longPressTimerRef.current = window.setTimeout(() => {
      setLongPressOpen(true)
      longPressTimerRef.current = null
    }, LONG_PRESS_DELAY_MS)
  }

  const handlePointerRelease = () => {
    clearLongPressTimer()
    setLongPressOpen(false)
  }

  if (!emailAddress) {
    return <span className={cn('min-w-0 flex-1 truncate text-right text-xs text-base-content/50', className)}>{emptyLabel}</span>
  }

  return (
    <Tooltip
      className={cn('min-w-0 max-w-full shrink', className)}
      content={buildMailboxTooltip(copyHintLabel, emailAddress)}
      contentClassName="max-w-[min(42rem,calc(100vw-1rem))]"
      open={hoverOpen || longPressOpen}
    >
      <button
        type="button"
        className={cn(
          'inline-flex h-7 min-w-0 max-w-full cursor-copy items-center justify-start rounded-full px-2.5 font-mono text-xs',
          'border border-base-300/80 bg-base-100 text-base-content/80 shadow-sm transition-[border-color,background-color,color,box-shadow,transform]',
          'hover:-translate-y-px hover:border-primary/70 hover:bg-primary/6 hover:text-primary hover:shadow-md',
          'focus-visible:-translate-y-px focus-visible:border-primary/70 focus-visible:bg-primary/6 focus-visible:text-primary focus-visible:shadow-md focus-visible:outline-none',
          tone === 'copied' && 'border-success/55 bg-success/10 text-success shadow-md',
        )}
        aria-label={copyAriaLabel}
        onBlur={() => setHoverOpen(false)}
        onFocus={() => setHoverOpen(true)}
        onMouseEnter={() => setHoverOpen(true)}
        onMouseLeave={() => setHoverOpen(false)}
        onPointerDown={handlePointerDown}
        onPointerUp={handlePointerRelease}
        onPointerCancel={handlePointerRelease}
        onPointerLeave={handlePointerRelease}
        onClick={onCopy}
      >
        <span className="truncate text-left">{emailAddress}</span>
        {tone === 'copied' ? (
          <span className="ml-2 inline-flex shrink-0 items-center gap-1 text-[10px] font-semibold uppercase tracking-[0.08em] text-success">
            <AppIcon name="check-bold" className="h-3.5 w-3.5" aria-hidden />
            {copiedLabel}
          </span>
        ) : null}
      </button>
    </Tooltip>
  )
}
