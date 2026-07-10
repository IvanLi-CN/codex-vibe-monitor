import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { AppIcon } from '../shared/AppIcon'
import { Button } from '../../components/ui/button'
import { OverlayHostProvider } from '../../components/ui/overlay-host'
import { cn } from '../../lib/utils'

interface AccountDetailDrawerShellProps {
  open: boolean
  labelledBy: string
  closeLabel: string
  onClose: () => void
  header: ReactNode
  children: ReactNode
  presentation?: 'overlay' | 'page'
  closeDisabled?: boolean
  autoFocusCloseButton?: boolean
  onPortalContainerChange?: (node: HTMLElement | null) => void
  onBodyElementChange?: (node: HTMLDivElement | null) => void
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
  presentation = 'overlay',
  closeDisabled = false,
  autoFocusCloseButton = true,
  onPortalContainerChange,
  onBodyElementChange,
  shellClassName,
  bodyClassName,
}: AccountDetailDrawerShellProps) {
  const closeButtonRef = useRef<HTMLButtonElement | null>(null)
  const [sectionElement, setSectionElement] = useState<HTMLElement | null>(null)
  const onCloseRef = useRef(onClose)
  const closeDisabledRef = useRef(closeDisabled)
  const hasAutofocusedForOpenRef = useRef(false)

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

  const handleBodyRef = useCallback(
    (node: HTMLDivElement | null) => {
      onBodyElementChange?.(node)
    },
    [onBodyElementChange],
  )

  useEffect(() => {
    if (!open || presentation === 'page' || typeof document === 'undefined') return undefined

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
  }, [open, presentation])

  useEffect(() => {
    if (!open) {
      hasAutofocusedForOpenRef.current = false
      return undefined
    }

    if (!autoFocusCloseButton || hasAutofocusedForOpenRef.current || typeof window === 'undefined') {
      return undefined
    }

    const focusTimer = window.setTimeout(() => {
      closeButtonRef.current?.focus()
      hasAutofocusedForOpenRef.current = true
    }, 0)

    return () => {
      window.clearTimeout(focusTimer)
    }
  }, [autoFocusCloseButton, open])

  if (!open) return null

  const shell = (
    <section
      ref={handleSectionRef}
      role={presentation === 'page' ? 'region' : 'dialog'}
      aria-modal={presentation === 'overlay' ? 'true' : undefined}
      aria-labelledby={labelledBy}
      className={cn(
        'drawer-shell flex w-full flex-col overflow-hidden',
        presentation === 'page'
          ? 'min-h-[calc(100dvh-8.5rem)] bg-base-100'
          : 'h-[min(100dvh-0.5rem,100dvh)]',
        'min-[1024px]:h-full min-[1024px]:rounded-none min-[1024px]:border-0',
        shellClassName,
      )}
      onClick={(event) => event.stopPropagation()}
    >
      <OverlayHostProvider value={sectionElement ?? undefined}>
        <div className="drawer-header px-4 py-4 sm:px-5 min-[1024px]:px-5 min-[1024px]:py-4 sm:min-[1024px]:px-6">
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
          ref={handleBodyRef}
          className={cn(
            'drawer-body min-h-0 flex-1 overflow-y-auto px-4 py-4 sm:px-5 sm:py-5 min-[1024px]:px-5 min-[1024px]:py-5 sm:min-[1024px]:px-6 sm:min-[1024px]:py-6',
            bodyClassName,
          )}
        >
          {children}
        </div>
      </OverlayHostProvider>
    </section>
  )

  if (presentation === 'page') {
    return shell
  }

  if (typeof document === 'undefined') return null

  return createPortal(
    <div className="fixed inset-0 z-[70]">
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-neutral/50 backdrop-blur-sm"
        onClick={closeDisabled ? undefined : onClose}
      />
      <div
        className="absolute inset-x-0 bottom-0 top-0 flex items-end justify-end min-[1024px]:inset-y-0 min-[1024px]:left-auto min-[1024px]:right-0 min-[1024px]:items-stretch min-[1024px]:pl-4 sm:min-[1024px]:pl-8"
        onClick={closeDisabled ? undefined : onClose}
      >
        {shell}
      </div>
    </div>,
    document.body,
  )
}
