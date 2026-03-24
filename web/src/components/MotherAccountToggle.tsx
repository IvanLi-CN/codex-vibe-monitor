import { AppIcon } from './AppIcon'
import { Button } from './ui/button'
import { Badge } from './ui/badge'
import { cn } from '../lib/utils'

const motherBadgeClassName = cn(
  'shrink-0 whitespace-nowrap px-2 py-px text-[11px] font-semibold leading-4 text-base-content shadow-none',
  'border-[color:color-mix(in_oklab,oklch(var(--color-warning))_58%,oklch(var(--color-base-300))_42%)]',
  'bg-[color:color-mix(in_oklab,oklch(var(--color-warning))_26%,oklch(var(--color-base-100))_74%)]',
  'shadow-[inset_0_0_0_1px_color-mix(in_oklab,oklch(var(--color-warning))_18%,transparent)]',
)

const motherAccentIconClassName =
  'text-[color:color-mix(in_oklab,oklch(var(--color-warning))_58%,currentColor_42%)]'

const motherToggleCheckedClassName = cn(
  'border-[color:color-mix(in_oklab,oklch(var(--color-warning))_60%,oklch(var(--color-base-300))_40%)]',
  'bg-[color:color-mix(in_oklab,oklch(var(--color-warning))_24%,oklch(var(--color-base-100))_76%)]',
  'text-base-content',
  'shadow-[inset_0_0_0_1px_color-mix(in_oklab,oklch(var(--color-warning))_18%,transparent),0_12px_28px_color-mix(in_oklab,oklch(var(--color-warning))_14%,transparent)]',
  'hover:border-[color:color-mix(in_oklab,oklch(var(--color-warning))_66%,oklch(var(--color-base-300))_34%)]',
  'hover:bg-[color:color-mix(in_oklab,oklch(var(--color-warning))_29%,oklch(var(--color-base-100))_71%)]',
  'disabled:opacity-[0.84]',
)

export function MotherAccountBadge({ label }: { label: string }) {
  return (
    <Badge variant="warning" className={motherBadgeClassName}>
      <span className="inline-flex items-center gap-1 leading-4">
        <AppIcon
          name="crown"
          className={cn('h-2.5 w-2.5 shrink-0', motherAccentIconClassName)}
          aria-hidden
        />
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
      data-state={checked ? 'checked' : 'unchecked'}
      data-icon-only={iconOnly ? 'true' : 'false'}
      disabled={disabled}
      onClick={onToggle}
      className={cn(
        'disabled:opacity-50',
        iconOnly
          ? 'h-9 w-9 shrink-0 rounded-full border p-0'
          : 'h-auto min-h-11 justify-start gap-3 rounded-2xl border px-3 py-2 text-left',
        checked
          ? motherToggleCheckedClassName
          : 'border-base-300/80 bg-base-100/72 text-base-content/68 hover:border-base-300 hover:bg-base-100',
      )}
    >
      <AppIcon
        name={checked ? 'crown' : 'crown-outline'}
        className={cn(
          iconOnly ? 'h-4 w-4' : 'h-5 w-5 shrink-0',
          checked ? motherAccentIconClassName : 'text-current/72',
        )}
        aria-hidden
      />
      {iconOnly ? null : (
        <span className="min-w-0 space-y-0.5">
          <span className="block text-sm font-semibold text-current">{label}</span>
          {description ? <span className="block text-xs leading-5 text-current/78">{description}</span> : null}
        </span>
      )}
    </Button>
  )
}
