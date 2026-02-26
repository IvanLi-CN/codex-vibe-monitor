import { Button } from './ui/button'

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
  return (
    <div
      className="sticky top-[70px] z-40 mx-auto mt-2 flex w-[calc(100%-2rem)] max-w-[1200px] items-start gap-2 rounded-xl border border-info/45 bg-info/15 px-4 py-3 text-base-content shadow"
      role="status"
      aria-live="polite"
    >
      <div className="flex flex-1 flex-wrap items-center gap-3">
        <span>
          {labels.available}{' '}
          <span className="font-mono font-semibold text-info">{currentVersion}</span>
          {' → '}
          <span className="font-mono font-semibold text-info">{availableVersion}</span>
        </span>
        <div className="ml-auto flex gap-2">
          <Button size="sm" onClick={onReload}>{labels.refresh}</Button>
          <Button size="sm" variant="secondary" onClick={onDismiss}>{labels.later}</Button>
        </div>
      </div>
    </div>
  )
}

export default UpdateAvailableBanner
