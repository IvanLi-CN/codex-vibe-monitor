import { Icon } from '@iconify/react'
import { Button } from './ui/button'
import { cn } from '../lib/utils'

export function MotherAccountBadge({ label }: { label: string }) {
  return (
    <span className="inline-flex items-center gap-1 rounded-full border border-amber-400/35 bg-amber-300/12 px-2 py-0.5 text-[11px] font-semibold text-amber-200">
      <Icon icon="mdi:crown" className="h-3.5 w-3.5" aria-hidden />
      {label}
    </span>
  )
}

export function MotherAccountToggle({
  checked,
  disabled,
  iconOnly = false,
  label,
  description,
  onToggle,
  ariaLabel,
}: {
  checked: boolean
  disabled?: boolean
  iconOnly?: boolean
  label: string
  description?: string
  onToggle: () => void
  ariaLabel?: string
}) {
  return (
    <Button
      type="button"
      variant="ghost"
      size={iconOnly ? 'icon' : 'sm'}
      aria-pressed={checked}
      aria-label={ariaLabel ?? label}
      disabled={disabled}
      onClick={onToggle}
      className={cn(
        iconOnly
          ? 'h-9 w-9 shrink-0 rounded-full border'
          : 'h-auto min-h-11 justify-start gap-3 rounded-2xl border px-3 py-2 text-left',
        checked
          ? 'border-amber-400/45 bg-amber-300/14 text-amber-100 hover:bg-amber-300/20'
          : 'border-base-300/80 bg-base-100/72 text-base-content/68 hover:border-base-300 hover:bg-base-100',
      )}
    >
      <Icon icon={checked ? 'mdi:crown' : 'mdi:crown-outline'} className={cn(iconOnly ? 'h-4 w-4' : 'h-5 w-5 shrink-0')} aria-hidden />
      {iconOnly ? null : (
        <span className="min-w-0 space-y-0.5">
          <span className="block text-sm font-semibold text-current">{label}</span>
          {description ? <span className="block text-xs leading-5 text-current/70">{description}</span> : null}
        </span>
      )}
    </Button>
  )
}
