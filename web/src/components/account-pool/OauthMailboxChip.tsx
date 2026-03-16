import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { AppIcon } from '../AppIcon'
import { Tooltip } from '../ui/tooltip'
import { selectAllReadonlyText } from '../../lib/clipboard'
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

function buildMailboxCopiedTooltip(copiedLabel: string) {
  return (
    <div className="inline-flex items-center gap-1.5 text-success">
      <AppIcon name="check-bold" className="h-3.5 w-3.5" aria-hidden />
      <span className="text-xs font-semibold uppercase tracking-[0.08em]">{copiedLabel}</span>
    </div>
  )
}

function buildMailboxManualTooltip(manualCopyLabel: string, emailAddress: string, inputRef: React.RefObject<HTMLInputElement | null>) {
  return (
    <div className="space-y-2">
      <p className="text-xs font-medium leading-5 text-base-content/78">{manualCopyLabel}</p>
      <input
        ref={inputRef}
        readOnly
        value={emailAddress}
        className="h-9 w-full rounded-lg border border-warning/35 bg-base-100 px-3 font-mono text-xs text-base-content shadow-sm outline-none"
        onFocus={(event) => selectAllReadonlyText(event.currentTarget)}
        onClick={(event) => selectAllReadonlyText(event.currentTarget)}
      />
    </div>
  )
}

interface OauthMailboxChipProps {
  emailAddress: string | null | undefined
  emptyLabel: string
  copyAriaLabel: string
  copyHintLabel: string
  copiedLabel: string
  manualCopyLabel: string
  manualBadgeLabel: string
  tone?: 'idle' | 'copied' | 'manual'
  onCopy: () => void
  className?: string
}

export function OauthMailboxChip({
  emailAddress,
  emptyLabel,
  copyAriaLabel,
  copyHintLabel,
  copiedLabel,
  manualCopyLabel,
  manualBadgeLabel,
  tone = 'idle',
  onCopy,
  className,
}: OauthMailboxChipProps) {
  const longPressTimerRef = useRef<number | null>(null)
  const manualCopyInputRef = useRef<HTMLInputElement | null>(null)
  const [longPressOpen, setLongPressOpen] = useState(false)
  const [hoverOpen, setHoverOpen] = useState(false)

  useEffect(() => {
    return () => {
      if (longPressTimerRef.current != null) {
        window.clearTimeout(longPressTimerRef.current)
      }
    }
  }, [])

  useEffect(() => {
    if (tone !== 'manual') return
    const timerId = window.setTimeout(() => {
      selectAllReadonlyText(manualCopyInputRef.current)
    }, 0)
    return () => {
      window.clearTimeout(timerId)
    }
  }, [tone])

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
      content={
        tone === 'manual'
          ? buildMailboxManualTooltip(manualCopyLabel, emailAddress, manualCopyInputRef)
          : tone === 'copied'
            ? buildMailboxCopiedTooltip(copiedLabel)
            : buildMailboxTooltip(copyHintLabel, emailAddress)
      }
      contentClassName={cn(
        'max-w-[min(42rem,calc(100vw-1rem))]',
        tone === 'copied' && 'border-success/35 bg-success/10 text-success',
        tone === 'manual' && 'border-warning/35 bg-warning/8',
      )}
      open={tone === 'copied' || tone === 'manual' || hoverOpen || longPressOpen}
    >
      <button
        type="button"
        className={cn(
          'inline-flex h-7 min-w-0 max-w-full cursor-copy items-center justify-start rounded-full px-2.5 font-mono text-xs',
          'border border-base-300/80 bg-base-100 text-base-content/80 shadow-sm transition-[border-color,background-color,color,box-shadow,transform]',
          'hover:-translate-y-px hover:border-primary/70 hover:bg-primary/6 hover:text-primary hover:shadow-md',
          'focus-visible:-translate-y-px focus-visible:border-primary/70 focus-visible:bg-primary/6 focus-visible:text-primary focus-visible:shadow-md focus-visible:outline-none',
          tone === 'copied' && 'border-success/55 bg-success/10 text-success shadow-md',
          tone === 'manual' && 'border-warning/45 bg-warning/10 text-warning shadow-md',
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
        {tone === 'manual' ? (
          <span className="ml-2 inline-flex shrink-0 items-center gap-1 text-[10px] font-semibold uppercase tracking-[0.08em] text-warning">
            <AppIcon name="alert-circle-outline" className="h-3.5 w-3.5" aria-hidden />
            {manualBadgeLabel}
          </span>
        ) : null}
      </button>
    </Tooltip>
  )
}
