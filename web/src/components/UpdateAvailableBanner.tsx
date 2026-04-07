import { useState } from 'react'
import { Button } from './ui/button'
import { floatingSurfaceStyle } from './ui/floating-surface'
import { usePortaledTheme } from './ui/use-portaled-theme'

export interface UpdateAvailableBannerProps {
  currentVersion: string
  availableVersion: string
  onReload: () => void
  onDismiss: () => void
  labels: {
    available: string
    refresh: string
    later: string
  }
}

export function UpdateAvailableBanner({
  currentVersion,
  availableVersion,
  onReload,
  onDismiss,
  labels,
}: UpdateAvailableBannerProps) {
  const [rootElement, setRootElement] = useState<HTMLDivElement | null>(null)
  const surfaceTheme = usePortaledTheme(rootElement)

  return (
    <div
      ref={setRootElement}
      style={floatingSurfaceStyle('primary', surfaceTheme)}
      className="app-shell-banner-boundary sticky top-[70px] z-40 mt-2 flex items-start gap-2 rounded-[1.15rem] border px-4 py-3 text-base-content"
      data-testid="update-available-banner"
      role="status"
      aria-live="polite"
    >
      <div className="flex flex-1 flex-wrap items-center gap-3">
        <span className="text-sm font-medium leading-6 sm:text-[0.95rem]">
          {labels.available}{' '}
          <span className="font-mono font-semibold text-primary drop-shadow-[0_1px_0_rgba(255,255,255,0.08)]">{currentVersion}</span>
          {' → '}
          <span className="font-mono font-semibold text-primary drop-shadow-[0_1px_0_rgba(255,255,255,0.08)]">{availableVersion}</span>
        </span>
        <div className="ml-auto flex gap-2 self-start sm:self-auto">
          <Button size="sm" onClick={onReload}>{labels.refresh}</Button>
          <Button size="sm" variant="secondary" onClick={onDismiss}>{labels.later}</Button>
        </div>
      </div>
    </div>
  )
}

export default UpdateAvailableBanner
