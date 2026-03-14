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
}

type MenuMode = 'hover' | 'sticky' | null

export function AccountTagContextChip({
  name,
  currentPageCreated = false,
  labels,
  onRemove,
  onEdit,
  defaultOpen = false,
}: AccountTagContextChipProps) {
  const wrapperRef = useRef<HTMLDivElement | null>(null)
  const longPressTimer = useRef<number | null>(null)
  const suppressClickRef = useRef(false)
  const [menuMode, setMenuMode] = useState<MenuMode>(defaultOpen ? 'sticky' : null)
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
        setMenuMode(null)
      }
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setMenuMode(null)
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

  const openStickyMenu = () => {
    setMenuMode('sticky')
  }

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
      setMenuMode(null)
    } finally {
      setBusyAction(null)
    }
  }

  return (
    <div
      ref={wrapperRef}
      className="relative inline-flex"
      onMouseEnter={() => setMenuMode((current) => (current === 'sticky' ? current : 'hover'))}
      onMouseLeave={() => setMenuMode((current) => (current === 'hover' ? null : current))}
    >
      <button
        type="button"
        className="group inline-flex"
        aria-haspopup="menu"
        aria-expanded={menuMode != null}
        onClick={() => {
          if (suppressClickRef.current) {
            suppressClickRef.current = false
            return
          }
          setMenuMode((current) => (current == null ? 'sticky' : null))
        }}
        onFocus={openStickyMenu}
        onPointerDown={(event) => {
          if (event.pointerType !== 'touch') return
          clearLongPress()
          longPressTimer.current = window.setTimeout(() => {
            suppressClickRef.current = true
            openStickyMenu()
          }, 450)
        }}
        onPointerUp={clearLongPress}
        onPointerCancel={clearLongPress}
      >
        <Badge variant="secondary" className="gap-2 px-3 py-1.5">
          <Icon icon="mdi:tag-outline" className="h-3.5 w-3.5" aria-hidden />
          <span>{name}</span>
          {currentPageCreated ? (
            <span className="rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-[0.14em] text-primary">
              {labels.selectedFromCurrentPage}
            </span>
          ) : null}
        </Badge>
      </button>

      {menuMode != null ? (
        <div
          role="menu"
          className={cn(
            'absolute left-0 top-full z-30 mt-2 w-56 rounded-2xl border border-base-300 bg-base-100/95 p-2 shadow-xl backdrop-blur',
            'animate-in fade-in-0 zoom-in-95',
          )}
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
                setMenuMode(null)
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
