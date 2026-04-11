import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { AppIcon } from './AppIcon'
import { Button } from './ui/button'
import { OverlayHostProvider } from './ui/overlay-host'
import { cn } from '../lib/utils'

interface AccountDetailDrawerShellProps {
  open: boolean
  labelledBy: string
  closeLabel: string
  onClose: () => void
  header: ReactNode
  children: ReactNode
  closeDisabled?: boolean
  autoFocusCloseButton?: boolean
  onPortalContainerChange?: (node: HTMLElement | null) => void
  shellClassName?: string
  bodyClassName?: string
}

export function AccountDetailDrawerShell({
  open,
  labelledBy,
  closeLabel,
  onClose,
  header,
  children,
  closeDisabled = false,
  autoFocusCloseButton = true,
  onPortalContainerChange,
  shellClassName,
  bodyClassName,
}: AccountDetailDrawerShellProps) {
  const closeButtonRef = useRef<HTMLButtonElement | null>(null)
  const [sectionElement, setSectionElement] = useState<HTMLElement | null>(null)
  const onCloseRef = useRef(onClose)
  const closeDisabledRef = useRef(closeDisabled)
  const previousOpenRef = useRef(false)

  useEffect(() => {
    onCloseRef.current = onClose
  }, [onClose])

  useEffect(() => {
    closeDisabledRef.current = closeDisabled
  }, [closeDisabled])

  const handleSectionRef = useCallback(
    (node: HTMLElement | null) => {
      setSectionElement(node)
      onPortalContainerChange?.(node)
    },
    [onPortalContainerChange],
  )

  useEffect(() => {
    if (!open || typeof document === 'undefined') return undefined

    const previousOverflow = document.body.style.overflow
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !closeDisabledRef.current) {
        onCloseRef.current()
      }
    }

    document.body.style.overflow = 'hidden'
    document.addEventListener('keydown', handleKeyDown)

    return () => {
      document.body.style.overflow = previousOverflow
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [open])

  useEffect(() => {
    const wasOpen = previousOpenRef.current
    previousOpenRef.current = open
    if (!open || wasOpen || !autoFocusCloseButton || typeof window === 'undefined') {
      return undefined
    }

    const focusTimer = window.setTimeout(() => closeButtonRef.current?.focus(), 0)
    return () => {
      window.clearTimeout(focusTimer)
    }
  }, [autoFocusCloseButton, open])

  if (!open || typeof document === 'undefined') return null

  return createPortal(
    <div className="fixed inset-0 z-[70]">
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-neutral/50 backdrop-blur-sm"
        onClick={closeDisabled ? undefined : onClose}
      />
      <div
        className="absolute inset-y-0 right-0 flex w-full justify-end pl-4 sm:pl-8"
        onClick={closeDisabled ? undefined : onClose}
      >
        <section
          ref={handleSectionRef}
          role="dialog"
          aria-modal="true"
          aria-labelledby={labelledBy}
          className={cn('drawer-shell flex h-full w-full flex-col', shellClassName)}
          onClick={(event) => event.stopPropagation()}
        >
          <OverlayHostProvider value={sectionElement ?? undefined}>
            <div className="drawer-header px-5 py-4 sm:px-6">
              <div className="flex items-start gap-4">
                <div className="min-w-0 flex-1">{header}</div>
                <Button
                  ref={closeButtonRef}
                  type="button"
                  variant="ghost"
                  size="icon"
                  onClick={onClose}
                  disabled={closeDisabled}
                >
                  <AppIcon name="close" className="h-5 w-5" aria-hidden />
                  <span className="sr-only">{closeLabel}</span>
                </Button>
              </div>
            </div>
            <div
              className={cn(
                'drawer-body min-h-0 flex-1 overflow-y-auto px-5 py-5 sm:px-6 sm:py-6',
                bodyClassName,
              )}
            >
              {children}
            </div>
          </OverlayHostProvider>
        </section>
      </div>
    </div>,
    document.body,
  )
}
