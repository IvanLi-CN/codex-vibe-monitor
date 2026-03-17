import { useEffect, useRef, useState, type ChangeEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { AppIcon } from '../AppIcon'
import { Button } from '../ui/button'
import { Input } from '../ui/input'
import { Popover, PopoverAnchor, PopoverArrow, PopoverContent } from '../ui/popover'
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

function buildMailboxCopiedTooltip(copiedLabel: string) {
  return (
    <div className="inline-flex items-center gap-1.5 text-success">
      <AppIcon name="check-bold" className="h-3.5 w-3.5" aria-hidden />
      <span className="text-xs font-semibold uppercase tracking-[0.08em]">{copiedLabel}</span>
    </div>
  )
}

function buildEditableMailboxHint(copyLabel: string, emailAddress: string | null | undefined, editHintLabel: string) {
  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <p className="text-xs font-semibold uppercase tracking-[0.08em] text-base-content/72">{copyLabel}</p>
        {emailAddress ? (
          <code className="block overflow-x-auto rounded-lg bg-base-200/80 px-2.5 py-2 font-mono text-[11px] text-base-content">
            {emailAddress}
          </code>
        ) : null}
      </div>
      <p className="leading-5 text-base-content/70">{editHintLabel}</p>
    </div>
  )
}

function selectManualCopyText(target: HTMLDivElement | null) {
  if (!target) return
  target.focus()
  const selection = target.ownerDocument.getSelection?.()
  if (!selection) return
  const range = target.ownerDocument.createRange()
  range.selectNodeContents(target)
  selection.removeAllRanges()
  selection.addRange(range)
}

function buildMailboxManualTooltip(
  manualCopyLabel: string,
  emailAddress: string,
  valueRef: React.RefObject<HTMLDivElement | null>,
) {
  return (
    <div className="space-y-2">
      <p className="text-xs font-medium leading-5 text-base-content/78">{manualCopyLabel}</p>
      <div
        ref={valueRef}
        role="textbox"
        aria-readonly="true"
        tabIndex={0}
        translate="no"
        spellCheck={false}
        data-lpignore="true"
        data-1p-ignore="true"
        data-form-type="other"
        className="h-9 w-full overflow-x-auto rounded-lg border border-warning/35 bg-base-100 px-3 py-2 font-mono text-xs text-base-content shadow-sm outline-none focus-visible:ring-2 focus-visible:ring-warning/40"
        onFocus={(event) => selectManualCopyText(event.currentTarget)}
        onClick={(event) => selectManualCopyText(event.currentTarget)}
      >
        <span className="whitespace-nowrap">{emailAddress}</span>
      </div>
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
  editor?: {
    draftValue: string
    inputName?: string
    inputAriaLabel: string
    inputPlaceholder: string
    editAriaLabel: string
    editHintLabel: string
    submitAriaLabel: string
    cancelAriaLabel: string
    startEditing: () => void
    onDraftValueChange: (value: string) => void
    onSubmit: () => void
    onCancel: () => void
    editing: boolean
    busy?: boolean
    disabled?: boolean
    submitDisabled?: boolean
  }
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
  editor,
}: OauthMailboxChipProps) {
  const longPressTimerRef = useRef<number | null>(null)
  const manualCopyValueRef = useRef<HTMLDivElement | null>(null)
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
      selectManualCopyText(manualCopyValueRef.current)
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

  if (editor) {
    const showCopyState = tone === 'copied' || tone === 'manual'
    const resolvedOpen = hoverOpen || longPressOpen || editor.editing || showCopyState
    const canCopy = Boolean(emailAddress)
    const handleEditorDraftChange = (event: ChangeEvent<HTMLInputElement>) => {
      editor.onDraftValueChange(event.target.value)
    }

    return (
      <Popover open={resolvedOpen}>
        <PopoverAnchor asChild>
          <button
            type="button"
            className={cn(
              'inline-flex h-7 min-w-0 max-w-full items-center justify-start rounded-full px-2.5 font-mono text-xs',
              'border border-base-300/80 bg-base-100 text-base-content/80 shadow-sm transition-[border-color,background-color,color,box-shadow,transform]',
              canCopy && 'cursor-copy hover:-translate-y-px hover:border-primary/70 hover:bg-primary/6 hover:text-primary hover:shadow-md',
              canCopy && 'focus-visible:-translate-y-px focus-visible:border-primary/70 focus-visible:bg-primary/6 focus-visible:text-primary focus-visible:shadow-md',
              !canCopy && 'cursor-default text-base-content/55',
              tone === 'copied' && 'border-success/55 bg-success/10 text-success shadow-md',
              tone === 'manual' && 'border-warning/45 bg-warning/10 text-warning shadow-md',
              className,
            )}
            aria-label={canCopy ? copyAriaLabel : editor.editAriaLabel}
            onBlur={() => setHoverOpen(false)}
            onFocus={() => setHoverOpen(true)}
            onMouseEnter={() => setHoverOpen(true)}
            onMouseLeave={() => {
              if (!editor.editing) {
                setHoverOpen(false)
              }
            }}
            onPointerDown={handlePointerDown}
            onPointerUp={handlePointerRelease}
            onPointerCancel={handlePointerRelease}
            onPointerLeave={handlePointerRelease}
            onClick={() => {
              if (canCopy) {
                onCopy()
              }
            }}
          >
            <span className="truncate text-left">{emailAddress || emptyLabel}</span>
            {tone === 'manual' ? (
              <span className="ml-2 inline-flex shrink-0 items-center gap-1 text-[10px] font-semibold uppercase tracking-[0.08em] text-warning">
                <AppIcon name="alert-circle-outline" className="h-3.5 w-3.5" aria-hidden />
                {manualBadgeLabel}
              </span>
            ) : null}
          </button>
        </PopoverAnchor>
        <PopoverContent
          align="end"
          side="top"
          sideOffset={10}
          className="w-[min(24rem,calc(100vw-1rem))] rounded-2xl border-base-300/80 bg-base-100/96 p-3 shadow-xl backdrop-blur-sm"
          onMouseEnter={() => setHoverOpen(true)}
          onMouseLeave={() => {
            if (!editor.editing) {
              setHoverOpen(false)
            }
          }}
          onFocusCapture={() => setHoverOpen(true)}
          onBlurCapture={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget as Node | null) && !editor.editing) {
              setHoverOpen(false)
            }
          }}
        >
          {editor.editing ? (
            <div className="space-y-3">
              <Input
                name={editor.inputName}
                aria-label={editor.inputAriaLabel}
                placeholder={editor.inputPlaceholder}
                value={editor.draftValue}
                onChange={handleEditorDraftChange}
                disabled={editor.busy}
              />
              <div className="flex items-center justify-end gap-2">
                <Button
                  type="button"
                  size="icon"
                  variant="ghost"
                  className="h-8 w-8 rounded-full"
                  aria-label={editor.cancelAriaLabel}
                  title={editor.cancelAriaLabel}
                  onClick={editor.onCancel}
                  disabled={editor.busy}
                >
                  <AppIcon name="close" className="h-4 w-4" aria-hidden />
                </Button>
                <Button
                  type="button"
                  size="icon"
                  className="h-8 w-8 rounded-full"
                  aria-label={editor.submitAriaLabel}
                  title={editor.submitAriaLabel}
                  onClick={editor.onSubmit}
                  disabled={editor.disabled || editor.submitDisabled}
                >
                  {editor.busy ? (
                    <AppIcon name="loading" className="h-4 w-4 animate-spin" aria-hidden />
                  ) : (
                    <AppIcon name="check-bold" className="h-4 w-4" aria-hidden />
                  )}
                </Button>
              </div>
            </div>
          ) : (
            <div className="space-y-3">
              {tone === 'manual'
                ? buildMailboxManualTooltip(manualCopyLabel, emailAddress ?? editor.draftValue, manualCopyValueRef)
                : tone === 'copied'
                  ? buildMailboxCopiedTooltip(copiedLabel)
                  : buildEditableMailboxHint(copyHintLabel, emailAddress, editor.editHintLabel)}
              <div className="flex items-center justify-end gap-2">
                <Button
                  type="button"
                  size="icon"
                  variant="secondary"
                  className="h-8 w-8 rounded-full"
                  aria-label={editor.editAriaLabel}
                  title={editor.editAriaLabel}
                  onClick={editor.startEditing}
                  disabled={editor.disabled || editor.busy}
                >
                  <AppIcon name="pencil-outline" className="h-4 w-4" aria-hidden />
                </Button>
              </div>
            </div>
          )}
          <PopoverArrow className="fill-base-100/96 stroke-base-300/80 stroke-[0.6]" width={14} height={8} />
        </PopoverContent>
      </Popover>
    )
  }

  if (!emailAddress) {
    return <span className={cn('min-w-0 flex-1 truncate text-right text-xs text-base-content/50', className)}>{emptyLabel}</span>
  }

  return (
    <Tooltip
      className={cn('min-w-0 max-w-full shrink', className)}
      content={
        tone === 'manual'
          ? buildMailboxManualTooltip(manualCopyLabel, emailAddress, manualCopyValueRef)
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
