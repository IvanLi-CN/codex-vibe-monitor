import {
  CONCURRENCY_LIMIT_MAX,
  CONCURRENCY_LIMIT_MIN,
  CONCURRENCY_LIMIT_UNLIMITED_SLIDER_VALUE,
  formatConcurrencyLimitValue,
  sliderConcurrencyLimitToApiValue,
} from '../lib/concurrencyLimit'

interface ConcurrencyLimitSliderProps {
  value: number
  disabled?: boolean
  title: string
  description: string
  currentLabel: string
  unlimitedLabel: string
  onChange: (value: number) => void
}

export function ConcurrencyLimitSlider({
  value,
  disabled = false,
  title,
  description,
  currentLabel,
  unlimitedLabel,
  onChange,
}: ConcurrencyLimitSliderProps) {
  const storedValue = sliderConcurrencyLimitToApiValue(value)
  const displayValue = formatConcurrencyLimitValue(storedValue, unlimitedLabel)

  return (
    <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-1">
          <p className="font-medium text-base-content">{title}</p>
          <p className="text-sm leading-6 text-base-content/68">{description}</p>
        </div>
        <div className="rounded-full border border-base-300/80 bg-base-200/80 px-3 py-1 text-sm font-semibold text-base-content">
          <span className="mr-2 text-xs uppercase tracking-[0.12em] text-base-content/55">{currentLabel}</span>
          <span>{displayValue}</span>
        </div>
      </div>

      <div className="mt-4 space-y-2">
        <input
          type="range"
          min={CONCURRENCY_LIMIT_MIN}
          max={CONCURRENCY_LIMIT_UNLIMITED_SLIDER_VALUE}
          step={1}
          value={value}
          disabled={disabled}
          aria-label={title}
          aria-valuetext={displayValue}
          onChange={(event) => onChange(Number(event.target.value))}
          className="h-2 w-full cursor-pointer appearance-none rounded-full bg-base-300 accent-primary disabled:cursor-not-allowed disabled:opacity-60"
        />
        <div
          className="grid items-center text-[11px] font-semibold uppercase tracking-[0.12em] text-base-content/45"
          style={{
            gridTemplateColumns: `repeat(${CONCURRENCY_LIMIT_MAX}, minmax(0, 1fr)) max-content`,
          }}
        >
          <span className="justify-self-start">{CONCURRENCY_LIMIT_MIN}</span>
          <span
            className="justify-self-center"
            style={{ gridColumn: `${CONCURRENCY_LIMIT_MAX} / span 1` }}
          >
            {CONCURRENCY_LIMIT_MAX}
          </span>
          <span
            className="justify-self-end pl-2"
            style={{ gridColumn: `${CONCURRENCY_LIMIT_MAX + 1} / span 1` }}
            aria-label={unlimitedLabel}
            title={unlimitedLabel}
          >
            ∞
          </span>
        </div>
      </div>
    </div>
  )
}
