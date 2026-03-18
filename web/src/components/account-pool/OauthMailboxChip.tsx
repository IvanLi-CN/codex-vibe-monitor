import { useEffect, useRef, useState, type ChangeEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { AppIcon } from '../AppIcon'
import { Button } from '../ui/button'
import { Input } from '../ui/input'
import { Popover, PopoverAnchor, PopoverArrow, PopoverContent } from '../ui/popover'
import { Tooltip } from '../ui/tooltip'
import { cn } from '../../lib/utils'

const LONG_PRESS_DELAY_MS = 360
const HOVER_CLOSE_DELAY_MS = 140

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
  return <span className="whitespace-nowrap text-[11px] font-medium">{copiedLabel}</span>
}

function buildEditableMailboxHint(copyLabel: string, emailAddress: string | null | undefined) {
  return (
    <div className="flex min-w-0 flex-1 items-center gap-2.5">
      <p className="shrink-0 text-[11px] font-medium leading-5 text-base-content/62">{copyLabel}</p>
      {emailAddress ? (
        <code className="block min-w-0 flex-1 overflow-x-auto rounded-xl border border-base-300/70 bg-base-200/55 px-2.5 py-1.5 font-mono text-[11px] text-base-content">
          {emailAddress}
        </code>
      ) : null}
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
    inputInvalid?: boolean
    inputError?: string | null
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
  const hoverCloseTimerRef = useRef<number | null>(null)
  const manualCopyValueRef = useRef<HTMLDivElement | null>(null)
  const [longPressOpen, setLongPressOpen] = useState(false)
  const [hoverOpen, setHoverOpen] = useState(false)

  useEffect(() => {
    return () => {
      if (longPressTimerRef.current != null) {
        window.clearTimeout(longPressTimerRef.current)
      }
      if (hoverCloseTimerRef.current != null) {
        window.clearTimeout(hoverCloseTimerRef.current)
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

  const clearHoverCloseTimer = () => {
    if (hoverCloseTimerRef.current != null) {
      window.clearTimeout(hoverCloseTimerRef.current)
      hoverCloseTimerRef.current = null
    }
  }

  const openHoverPopover = () => {
    clearHoverCloseTimer()
    setHoverOpen(true)
  }

  const scheduleHoverPopoverClose = () => {
    clearHoverCloseTimer()
    hoverCloseTimerRef.current = window.setTimeout(() => {
      setHoverOpen(false)
      hoverCloseTimerRef.current = null
    }, HOVER_CLOSE_DELAY_MS)
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
    const showCopiedTooltip = tone === 'copied'
    const showManualPanel = tone === 'manual'
    const resolvedOpen = editor.editing || showManualPanel || ((hoverOpen || longPressOpen) && !showCopiedTooltip)
    const canCopy = Boolean(emailAddress)
    const handleEditorDraftChange = (event: ChangeEvent<HTMLInputElement>) => {
      editor.onDraftValueChange(event.target.value)
    }

    const trigger = (
      <button
        type="button"
        className={cn(
          'inline-flex h-7 min-w-0 max-w-full items-center justify-start rounded-full px-2.5 font-mono text-xs',
          'border border-base-300/80 bg-base-100 text-base-content/80 shadow-sm transition-[border-color,background-color,color,box-shadow]',
          canCopy && 'cursor-copy hover:border-primary/55 hover:bg-primary/5 hover:text-primary hover:shadow-sm',
          canCopy && 'focus-visible:border-primary/55 focus-visible:bg-primary/5 focus-visible:text-primary focus-visible:shadow-sm',
          !canCopy && 'cursor-default text-base-content/55',
          tone === 'manual' && 'border-warning/35 bg-base-100 text-base-content shadow-sm',
          className,
        )}
        aria-label={canCopy ? copyAriaLabel : editor.editAriaLabel}
        onBlur={() => {
          if (!editor.editing) {
            scheduleHoverPopoverClose()
          }
        }}
        onFocus={openHoverPopover}
        onMouseEnter={openHoverPopover}
        onMouseLeave={() => {
          if (!editor.editing) {
            scheduleHoverPopoverClose()
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
    )

    const buttonWithCopiedTooltip = showCopiedTooltip ? (
      <Tooltip
        open
        side="top"
        sideOffset={8}
        content={buildMailboxCopiedTooltip(copiedLabel)}
        contentClassName="w-fit max-w-none rounded-md border-transparent bg-base-content px-2 py-1 text-base-100 shadow-lg"
        arrowClassName="fill-base-content stroke-base-content"
        className="min-w-0 max-w-full shrink"
      >
        {trigger}
      </Tooltip>
    ) : (
      trigger
    )

    if (showCopiedTooltip) {
      return buttonWithCopiedTooltip
    }

    return (
      <Popover open={resolvedOpen}>
        <PopoverAnchor asChild>{trigger}</PopoverAnchor>
        <PopoverContent
          align="end"
          side="top"
          sideOffset={8}
          className={cn(
            'rounded-2xl border border-base-300/90 bg-base-100/98 p-2 shadow-[0_12px_32px_rgba(15,23,42,0.12)] backdrop-blur-sm',
            editor.editing
              ? 'w-[min(30rem,calc(100vw-1rem))]'
              : showManualPanel
                ? 'w-[min(24rem,calc(100vw-1rem))]'
                : 'w-fit min-w-[15rem] max-w-[min(30rem,calc(100vw-1rem))]',
          )}
          onMouseEnter={openHoverPopover}
          onMouseLeave={() => {
            if (!editor.editing) {
              scheduleHoverPopoverClose()
            }
          }}
          onFocusCapture={openHoverPopover}
          onBlurCapture={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget as Node | null) && !editor.editing) {
              scheduleHoverPopoverClose()
            }
          }}
        >
          {editor.editing ? (
            <div className="space-y-2">
              <div className="flex min-w-0 items-center gap-2">
                <Input
                  type="text"
                  inputMode="email"
                  autoComplete="off"
                  autoCorrect="off"
                  autoCapitalize="none"
                  spellCheck={false}
                  translate="no"
                  data-lpignore="true"
                  data-1p-ignore="true"
                  data-bwignore="true"
                  data-form-type="other"
                  name={editor.inputName}
                  aria-label={editor.inputAriaLabel}
                  aria-invalid={editor.inputInvalid ? 'true' : 'false'}
                  placeholder={editor.inputPlaceholder}
                  value={editor.draftValue}
                  onChange={handleEditorDraftChange}
                  disabled={editor.busy}
                  className="h-8 min-w-0 flex-1 rounded-xl border-base-300/80 bg-base-100 px-3"
                />
                <Button
                  type="button"
                  size="icon"
                  variant="ghost"
                  className="h-7 w-7 shrink-0 rounded-full text-base-content/72 hover:bg-base-200"
                  aria-label={editor.cancelAriaLabel}
                  title={editor.cancelAriaLabel}
                  onClick={editor.onCancel}
                  disabled={editor.busy}
                >
                  <AppIcon name="close" className="h-3.5 w-3.5" aria-hidden />
                </Button>
                <Button
                  type="button"
                  size="icon"
                  className="h-7 w-7 shrink-0 rounded-full"
                  aria-label={editor.submitAriaLabel}
                  title={editor.submitAriaLabel}
                  onClick={editor.onSubmit}
                  disabled={editor.disabled || editor.submitDisabled}
                >
                  {editor.busy ? (
                    <AppIcon name="loading" className="h-3.5 w-3.5 animate-spin" aria-hidden />
                  ) : (
                    <AppIcon name="check-bold" className="h-3.5 w-3.5" aria-hidden />
                  )}
                </Button>
              </div>
              {editor.inputError ? <p className="text-xs leading-5 text-error">{editor.inputError}</p> : null}
            </div>
          ) : (
            <div className="flex min-w-0 items-center gap-2">
              <div className="min-w-0 flex-1">
                {showManualPanel
                  ? buildMailboxManualTooltip(manualCopyLabel, emailAddress ?? editor.draftValue, manualCopyValueRef)
                  : buildEditableMailboxHint(copyHintLabel, emailAddress)}
              </div>
              <Button
                type="button"
                size="icon"
                variant="ghost"
                className="h-7 w-7 shrink-0 rounded-full border border-transparent text-base-content/68 hover:border-base-300/80 hover:bg-base-200/80"
                aria-label={editor.editAriaLabel}
                title={editor.editAriaLabel}
                onClick={editor.startEditing}
                disabled={editor.disabled || editor.busy}
              >
                <AppIcon name="pencil-outline" className="h-3.5 w-3.5" aria-hidden />
              </Button>
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
        tone === 'copied' && 'w-fit max-w-none rounded-md border-transparent bg-base-content px-2 py-1 text-base-100 shadow-lg',
        tone === 'manual' && 'border-warning/35 bg-warning/8',
      )}
      arrowClassName={tone === 'copied' ? 'fill-base-content stroke-base-content' : undefined}
      open={tone === 'copied' || tone === 'manual' || hoverOpen || longPressOpen}
    >
      <button
        type="button"
        className={cn(
          'inline-flex h-7 min-w-0 max-w-full cursor-copy items-center justify-start rounded-full px-2.5 font-mono text-xs',
          'border border-base-300/80 bg-base-100 text-base-content/80 shadow-sm transition-[border-color,background-color,color,box-shadow]',
          'hover:border-primary/55 hover:bg-primary/5 hover:text-primary hover:shadow-sm',
          'focus-visible:border-primary/55 focus-visible:bg-primary/5 focus-visible:text-primary focus-visible:shadow-sm focus-visible:outline-none',
          tone === 'manual' && 'border-warning/35 bg-base-100 text-base-content shadow-sm',
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
