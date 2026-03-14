import { useEffect, useRef, useState } from 'react'
import { Icon } from '@iconify/react'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import { cn } from '../lib/utils'

interface AccountTagContextChipLabels {
  selectedFromCurrentPage: string
  remove: string
  deleteAndRemove: string
  edit: string
  hoverHint: string
}

interface AccountTagContextChipProps {
  name: string
  currentPageCreated?: boolean
  labels: AccountTagContextChipLabels
  onRemove: () => void | Promise<void>
  onEdit: () => void
  defaultOpen?: boolean
  defaultShowActionButton?: boolean
}

export function AccountTagContextChip({
  name,
  currentPageCreated = false,
  labels,
  onRemove,
  onEdit,
  defaultOpen = false,
  defaultShowActionButton = false,
}: AccountTagContextChipProps) {
  const wrapperRef = useRef<HTMLDivElement | null>(null)
  const longPressTimer = useRef<number | null>(null)
  const [showActionButton, setShowActionButton] = useState(defaultShowActionButton || defaultOpen)
  const [menuOpen, setMenuOpen] = useState(defaultOpen)
  const [busyAction, setBusyAction] = useState<'remove' | null>(null)

  useEffect(() => {
    const clearLongPress = () => {
      if (longPressTimer.current != null) {
        window.clearTimeout(longPressTimer.current)
        longPressTimer.current = null
      }
    }

    const handlePointerDown = (event: PointerEvent) => {
      if (!wrapperRef.current?.contains(event.target as Node)) {
        setMenuOpen(false)
      }
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setMenuOpen(false)
      }
    }

    document.addEventListener('pointerdown', handlePointerDown)
    document.addEventListener('keydown', handleKeyDown)

    return () => {
      clearLongPress()
      document.removeEventListener('pointerdown', handlePointerDown)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [])

  const clearLongPress = () => {
    if (longPressTimer.current != null) {
      window.clearTimeout(longPressTimer.current)
      longPressTimer.current = null
    }
  }

  const handleRemove = async () => {
    setBusyAction('remove')
    try {
      await onRemove()
      setMenuOpen(false)
    } finally {
      setBusyAction(null)
    }
  }

  return (
    <div
      ref={wrapperRef}
      className="relative inline-flex"
      onMouseEnter={() => setShowActionButton(true)}
      onMouseLeave={() => setShowActionButton(menuOpen)}
    >
      <div
        className="inline-flex"
        onPointerDown={(event) => {
          if (event.pointerType !== 'touch') return
          clearLongPress()
          longPressTimer.current = window.setTimeout(() => {
            setShowActionButton(true)
            setMenuOpen(true)
          }, 450)
        }}
        onPointerUp={clearLongPress}
        onPointerCancel={clearLongPress}
      >
        <Badge variant="secondary" className="gap-2 px-3 py-1.5 pr-8">
          <Icon icon="mdi:tag-outline" className="h-3.5 w-3.5" aria-hidden />
          <span>{name}</span>
          {currentPageCreated ? (
            <span className="rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-[0.14em] text-primary">
              {labels.selectedFromCurrentPage}
            </span>
          ) : null}
        </Badge>
      </div>

      <button
        type="button"
        aria-label={`${name} more actions`}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        className={cn(
          'absolute right-0.5 top-1/2 inline-flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded-full border border-base-300 bg-base-100/95 text-base-content shadow-sm transition-all',
          showActionButton || menuOpen
            ? 'pointer-events-auto opacity-100 scale-100'
            : 'pointer-events-none opacity-0 scale-95',
        )}
        onFocus={() => setShowActionButton(true)}
        onBlur={() => setShowActionButton(menuOpen)}
        onClick={() => {
          setShowActionButton(true)
          setMenuOpen((current) => !current)
        }}
      >
        <Icon icon="mdi:dots-horizontal" className="h-3.5 w-3.5" aria-hidden />
      </button>

      {menuOpen ? (
        <div
          role="menu"
          className={cn('absolute left-0 top-full z-30 mt-2 w-56 rounded-2xl border border-base-300 bg-base-100/95 p-2 shadow-xl backdrop-blur', 'animate-in fade-in-0 zoom-in-95')}
        >
          <div className="mb-2 px-2 text-xs text-base-content/60">{labels.hoverHint}</div>
          <div className="space-y-1">
            <Button
              type="button"
              variant="ghost"
              className="w-full justify-start"
              disabled={busyAction === 'remove'}
              onClick={() => void handleRemove()}
            >
              <Icon
                icon={currentPageCreated ? 'mdi:delete-outline' : 'mdi:link-variant-off'}
                className="mr-2 h-4 w-4"
                aria-hidden
              />
              {currentPageCreated ? labels.deleteAndRemove : labels.remove}
            </Button>
            <Button
              type="button"
              variant="ghost"
              className="w-full justify-start"
              onClick={() => {
                setMenuOpen(false)
                onEdit()
              }}
            >
              <Icon icon="mdi:pencil-outline" className="mr-2 h-4 w-4" aria-hidden />
              {labels.edit}
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  )
}
