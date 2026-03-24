import { AppIcon } from './AppIcon'
import { Button } from './ui/button'
import { Badge } from './ui/badge'
import { cn } from '../lib/utils'

export function MotherAccountBadge({ label }: { label: string }) {
  return (
    <Badge
      variant="warning"
      className="shrink-0 whitespace-nowrap border-warning/70 bg-warning/25 px-2 py-px text-[11px] font-medium leading-4 text-warning-content shadow-none"
    >
      <span className="inline-flex items-center gap-1 leading-4">
        <AppIcon name="crown" className="h-2.5 w-2.5 shrink-0" aria-hidden />
        <span className="leading-4">{label}</span>
      </span>
    </Badge>
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
          ? 'border-warning/70 bg-warning/25 text-warning-content shadow-sm shadow-warning/20 hover:bg-warning/40'
          : 'border-base-300/80 bg-base-100/72 text-base-content/68 hover:border-base-300 hover:bg-base-100',
      )}
    >
      <AppIcon name={checked ? 'crown' : 'crown-outline'} className={cn(iconOnly ? 'h-4 w-4' : 'h-5 w-5 shrink-0')} aria-hidden />
      {iconOnly ? null : (
        <span className="min-w-0 space-y-0.5">
          <span className="block text-sm font-semibold text-current">{label}</span>
          {description ? <span className="block text-xs leading-5 text-current/70">{description}</span> : null}
        </span>
      )}
    </Button>
  )
}
