import { Tooltip } from '../ui/tooltip'
import { cn } from '../../lib/utils'

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
  onCopy: () => void
  forceHover?: boolean
  openTooltip?: boolean
  className?: string
}

export function OauthMailboxChip({
  emailAddress,
  emptyLabel,
  copyAriaLabel,
  copyHintLabel,
  onCopy,
  forceHover = false,
  openTooltip = false,
  className,
}: OauthMailboxChipProps) {
  if (!emailAddress) {
    return <span className={cn('min-w-0 flex-1 truncate text-right text-xs text-base-content/50', className)}>{emptyLabel}</span>
  }

  return (
    <Tooltip
      className={cn('min-w-0 max-w-full shrink', className)}
      content={buildMailboxTooltip(copyHintLabel, emailAddress)}
      contentClassName="max-w-[min(42rem,calc(100vw-1rem))]"
      open={openTooltip ? true : undefined}
    >
      <button
        type="button"
        className={cn(
          'inline-flex h-7 min-w-0 max-w-full cursor-copy items-center justify-start rounded-full px-2.5 font-mono text-xs',
          'border border-base-300/80 bg-base-100 text-base-content/80 shadow-sm transition-[border-color,background-color,color,box-shadow,transform]',
          'hover:-translate-y-px hover:border-primary/70 hover:bg-primary/6 hover:text-primary hover:shadow-md',
          'focus-visible:-translate-y-px focus-visible:border-primary/70 focus-visible:bg-primary/6 focus-visible:text-primary focus-visible:shadow-md focus-visible:outline-none',
          forceHover && 'border-primary/70 bg-primary/6 text-primary shadow-md -translate-y-px',
        )}
        aria-label={copyAriaLabel}
        onClick={onCopy}
      >
        <span className="truncate text-left">{emailAddress}</span>
      </button>
    </Tooltip>
  )
}
