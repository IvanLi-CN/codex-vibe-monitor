import { useEffect, useRef, useState } from 'react'
import { AppIcon } from './AppIcon'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import { cn } from '../lib/utils'

interface AccountTagContextChipLabels {
  selectedFromCurrentPage: string
  remove: string
  deleteAndRemove: string
  edit: string
}

interface AccountTagContextChipProps {
  name: string
  currentPageCreated?: boolean
  disabled?: boolean
  labels: AccountTagContextChipLabels
  onRemove: () => void | Promise<void>
  onEdit: () => void
  defaultOpen?: boolean
  defaultShowActionButton?: boolean
}

export function AccountTagContextChip({
  name,
  currentPageCreated = false,
  disabled = false,
  labels,
  onRemove,
  onEdit,
  defaultOpen = false,
  defaultShowActionButton = false,
}: AccountTagContextChipProps) {
  const wrapperRef = useRef<HTMLDivElement | null>(null)
  const longPressTimer = useRef<number | null>(null)
  const [showActionButton, setShowActionButton] = useState(!disabled && (defaultShowActionButton || defaultOpen))
  const [menuOpen, setMenuOpen] = useState(!disabled && defaultOpen)
  const [busyAction, setBusyAction] = useState<'remove' | null>(null)

  useEffect(() => {
    if (!disabled) return
    setShowActionButton(false)
    setMenuOpen(false)
  }, [disabled])

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
      onMouseEnter={() => {
        if (disabled) return
        setShowActionButton(true)
      }}
      onMouseLeave={() => {
        if (disabled) return
        setShowActionButton(menuOpen)
      }}
    >
      <div
        className="inline-flex"
        onPointerDown={(event) => {
          if (disabled) return
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
        <Badge variant="secondary" className="gap-2 px-3 py-1.5">
          <AppIcon name="tag-outline" className="h-3.5 w-3.5" aria-hidden />
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
        disabled={disabled}
        className={cn(
          'absolute right-0.5 top-1/2 hidden h-6 w-6 -translate-y-1/2 items-center justify-center rounded-full border border-base-300 bg-base-100/95 text-base-content shadow-sm transition-all sm:inline-flex',
          showActionButton || menuOpen
            ? 'pointer-events-auto opacity-100 scale-100'
            : 'pointer-events-none opacity-0 scale-95',
        )}
        onFocus={() => {
          if (disabled) return
          setShowActionButton(true)
        }}
        onBlur={() => {
          if (disabled) return
          setShowActionButton(menuOpen)
        }}
        onClick={() => {
          if (disabled) return
          setShowActionButton(true)
          setMenuOpen((current) => !current)
        }}
      >
        <AppIcon name="dots-horizontal" className="h-3.5 w-3.5" aria-hidden />
      </button>

      {menuOpen ? (
        <div
          role="menu"
          className={cn(
            'absolute right-0 top-full z-30 mt-1 inline-flex min-w-[9.5rem] flex-col rounded-[0.85rem] border border-base-300/90 bg-base-100/97 p-1 shadow-lg backdrop-blur',
            'animate-in fade-in-0 zoom-in-95',
          )}
        >
          <div className="space-y-0.5">
            <Button
              type="button"
              variant="ghost"
              className="h-7.5 w-full justify-start rounded-[0.7rem] px-2 text-[0.82rem] whitespace-nowrap"
              disabled={busyAction === 'remove'}
              onClick={() => void handleRemove()}
            >
              <AppIcon
                name={currentPageCreated ? 'delete-outline' : 'link-variant-off'}
                className="mr-1.5 h-3.5 w-3.5"
                aria-hidden
              />
              {currentPageCreated ? labels.deleteAndRemove : labels.remove}
            </Button>
            <Button
              type="button"
              variant="ghost"
              className="h-7.5 w-full justify-start rounded-[0.7rem] px-2 text-[0.82rem] whitespace-nowrap"
              onClick={() => {
                setMenuOpen(false)
                onEdit()
              }}
            >
              <AppIcon name="pencil-outline" className="mr-1.5 h-3.5 w-3.5" aria-hidden />
              {labels.edit}
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  )
}
