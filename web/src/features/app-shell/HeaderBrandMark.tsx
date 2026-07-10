import { cn } from '../../lib/utils'

export type HeaderBrandMarkState = 'idle' | 'active' | 'reconnecting' | 'disabled'

interface HeaderBrandMarkProps {
  alt: string
  state?: HeaderBrandMarkState
  className?: string
  markClassName?: string
  'data-testid'?: string
}

export function HeaderBrandMark({
  alt,
  state = 'idle',
  className,
  markClassName,
  'data-testid': dataTestId,
}: HeaderBrandMarkProps) {
  const isActive = state === 'active'
  const isReconnecting = state === 'reconnecting'
  const isDisabled = state === 'disabled'

  return (
    <span
      className={cn('header-brand-mark relative inline-flex h-10 w-10 shrink-0 items-center justify-center overflow-visible', className)}
      data-logo-state={state}
      data-testid={dataTestId}
    >
      <span
        className={cn(
          'header-brand-mark__glow pointer-events-none absolute inline-flex h-[4.6rem] w-[4.6rem] rounded-full bg-gradient-to-r from-secondary/20 via-primary/4 to-primary/26 opacity-0 transition-opacity duration-500 motion-reduce:transition-none',
          isActive && 'opacity-95 motion-safe:animate-signal-glow',
        )}
        aria-hidden
      />
      <span
        className={cn(
          'header-brand-mark__halo pointer-events-none absolute inline-flex h-[3.55rem] w-[3.55rem] rounded-full bg-primary/18 opacity-0 blur-lg transition-opacity duration-500 motion-reduce:transition-none',
          isActive && 'opacity-70 motion-safe:animate-signal-halo',
        )}
        aria-hidden
      />
      <span
        className={cn(
          'header-brand-mark__status-disc pointer-events-none absolute inline-flex h-[3.4rem] w-[3.4rem] rounded-full opacity-0 transition-opacity duration-500 motion-reduce:transition-none',
          (isReconnecting || isDisabled) && 'opacity-100',
        )}
        aria-hidden
      />
      <span
        className={cn(
          'header-brand-mark__ring pointer-events-none absolute inline-flex h-[3.2rem] w-[3.2rem] rounded-full border border-primary/44 opacity-0 transition-opacity duration-500 motion-reduce:transition-none',
          isActive && 'opacity-90 motion-safe:animate-signal-ring',
        )}
        aria-hidden
      />
      <span
        className={cn(
          'header-brand-mark__ring header-brand-mark__ring--secondary pointer-events-none absolute inline-flex h-[3.2rem] w-[3.2rem] rounded-full border border-secondary/28 opacity-0 transition-opacity duration-500 motion-reduce:transition-none',
          isActive && 'opacity-55 motion-safe:animate-signal-ring',
        )}
        style={{ animationDelay: '-1.15s' }}
        aria-hidden
      />
      <span
        className={cn(
          'header-brand-mark__reconnect pointer-events-none absolute inline-flex h-16 w-16 rounded-full border-2 border-dashed opacity-0 transition-opacity duration-300 motion-reduce:transition-none',
          isReconnecting && 'border-primary/72 opacity-95 motion-safe:animate-orbit-spin',
          isDisabled && 'border-warning/75 opacity-45',
        )}
        aria-hidden
      />
      <img
        src={`${import.meta.env.BASE_URL}brand-mark.svg`}
        alt={alt}
        className={cn(
          'header-brand-mark__image relative z-20 h-10 w-10 transform-gpu transition-[transform,opacity,filter] duration-500 ease-[cubic-bezier(0.25,1,0.5,1)] motion-reduce:transition-none',
          isActive && 'motion-safe:animate-signal-core',
          isReconnecting && 'opacity-90',
          isDisabled && 'opacity-60',
          markClassName,
        )}
      />
    </span>
  )
}
