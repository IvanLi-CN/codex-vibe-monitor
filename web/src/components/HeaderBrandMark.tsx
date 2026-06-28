import { cn } from '../lib/utils'

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
      className={cn('relative inline-flex h-20 w-20 shrink-0 items-center justify-center', className)}
      data-logo-state={state}
      data-testid={dataTestId}
    >
      <span
        className={cn(
          'pointer-events-none absolute inline-flex h-20 w-20 rounded-full bg-gradient-to-r from-secondary/26 via-primary/6 to-primary/34 opacity-0 transition-opacity duration-500 motion-reduce:transition-none',
          isActive && 'opacity-100 motion-safe:animate-signal-glow',
        )}
        aria-hidden
      />
      <span
        className={cn(
          'pointer-events-none absolute inline-flex h-16 w-16 rounded-full bg-primary/25 opacity-0 blur-xl transition-opacity duration-500 motion-reduce:transition-none',
          isActive && 'opacity-80 motion-safe:animate-signal-halo',
        )}
        aria-hidden
      />
      <span
        className={cn(
          'pointer-events-none absolute inline-flex h-14 w-14 rounded-full border border-primary/60 opacity-0 transition-opacity duration-500 motion-reduce:transition-none',
          isActive && 'opacity-100 motion-safe:animate-signal-ring',
        )}
        aria-hidden
      />
      <span
        className={cn(
          'pointer-events-none absolute inline-flex h-14 w-14 rounded-full border border-secondary/40 opacity-0 transition-opacity duration-500 motion-reduce:transition-none',
          isActive && 'opacity-75 motion-safe:animate-signal-ring',
        )}
        style={{ animationDelay: '-1.15s' }}
        aria-hidden
      />
      <span
        className={cn(
          'pointer-events-none absolute inline-flex h-16 w-16 rounded-full border-2 border-dashed opacity-0 transition-opacity duration-300 motion-reduce:transition-none',
          isReconnecting && 'border-primary/72 opacity-95 motion-safe:animate-orbit-spin',
          isDisabled && 'border-warning/75 opacity-45',
        )}
        aria-hidden
      />
      <img
        src="/brand-mark.svg"
        alt={alt}
        className={cn(
          'relative z-20 h-10 w-10 transform-gpu transition-[transform,opacity,filter] duration-500 ease-[cubic-bezier(0.25,1,0.5,1)] motion-reduce:transition-none',
          isActive
            ? 'drop-shadow-[0_0_18px_rgba(59,130,246,0.52)] motion-safe:animate-signal-core'
            : 'drop-shadow-[0_0_6px_rgba(59,130,246,0.35)]',
          isReconnecting && 'opacity-90 saturate-[0.92]',
          isDisabled && 'grayscale opacity-60',
          markClassName,
        )}
      />
    </span>
  )
}
