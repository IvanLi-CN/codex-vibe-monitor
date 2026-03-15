import { AppIcon } from './AppIcon'
import { useState } from 'react'
import { useTranslation } from '../i18n'
import { cn } from '../lib/utils'
import { Button } from './ui/button'

export type RecordsNewDataButtonState = 'idle' | 'interactive' | 'loading'

export interface RecordsNewDataButtonProps {
  count: number
  isLoading?: boolean
  onRefresh: () => void
  stateOverride?: RecordsNewDataButtonState
}

export function RecordsNewDataButton({
  count,
  isLoading = false,
  onRefresh,
  stateOverride,
}: RecordsNewDataButtonProps) {
  const { t } = useTranslation()
  const [isHovered, setIsHovered] = useState(false)
  const [isFocused, setIsFocused] = useState(false)

  const visualState: RecordsNewDataButtonState = isLoading
    ? 'loading'
    : stateOverride ?? (isHovered || isFocused ? 'interactive' : 'idle')
  const isInteractive = visualState !== 'idle'
  const isDisabled = visualState === 'loading'

  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      data-testid="records-new-data-button"
      data-state={visualState}
      data-icon={visualState === 'loading' ? 'refresh' : 'help'}
      aria-label={
        visualState === 'loading'
          ? t('records.summary.notice.refreshingAria', { count })
          : isInteractive
            ? t('records.summary.notice.refreshAria', { count })
            : t('records.summary.notice.newDataAria', { count })
      }
      aria-busy={visualState === 'loading'}
      disabled={isDisabled}
      className={cn(
        'group h-auto rounded-full border px-3 py-1 text-xs font-semibold shadow-sm disabled:opacity-100',
        isInteractive
          ? 'border-primary/35 bg-primary/15 text-primary hover:bg-primary/20'
          : 'border-warning/35 bg-warning/10 text-warning hover:bg-warning/15',
      )}
      onClick={onRefresh}
      onMouseEnter={() => setIsHovered(true)}
      onMouseLeave={() => setIsHovered(false)}
      onFocus={() => setIsFocused(true)}
      onBlur={() => setIsFocused(false)}
    >
      <span className="grid whitespace-nowrap">
        <span
          data-testid="records-new-data-label-idle"
          aria-hidden={isInteractive}
          className={cn(
            'col-start-1 row-start-1 transition-opacity',
            isInteractive ? 'opacity-0' : 'opacity-100',
          )}
        >
          {t('records.summary.notice.newData', { count })}
        </span>
        <span
          data-testid="records-new-data-label-action"
          aria-hidden={!isInteractive}
          className={cn(
            'col-start-1 row-start-1 transition-opacity',
            isInteractive ? 'opacity-100' : 'opacity-0',
          )}
        >
          {t('records.summary.notice.refreshAction')}
        </span>
      </span>
      <span
        className={cn(
          'ml-1.5 inline-flex h-4 w-4 items-center justify-center rounded-full',
          isInteractive ? 'text-primary' : 'text-warning',
        )}
        aria-hidden
      >
        <AppIcon
          name={visualState === 'loading' ? 'refresh' : 'help-circle-outline'}
          className={cn('h-4 w-4', visualState === 'loading' && 'animate-spin')}
        />
      </span>
    </Button>
  )
}
