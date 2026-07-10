import type { ReactNode } from 'react'
import type { UsageBreakdown } from '../../lib/api'

export type UsageBreakdownKind = 'cost' | 'tokens'

export interface UsageBreakdownTooltipProps {
  title: string
  breakdown?: UsageBreakdown | null
  kind: UsageBreakdownKind
  formatNumber: (value: number) => string
  formatCurrency: (value: number) => string
  labels: {
    total: string
    cacheWrite: string
    cacheRead: string
    output: string
    input: string
    reasoning: string
    unavailable: string
    unknownModel: string
  }
}

function hasCostValue(value: number | undefined) {
  return value != null && Number.isFinite(value) && value !== 0
}

function modelLabel(model: string, unknownModel: string) {
  return model === 'unknown' ? unknownModel : model
}

function CostRows({
  costs,
  formatCurrency,
  labels,
}: Pick<UsageBreakdownTooltipProps, 'formatCurrency' | 'labels'> & {
  costs: UsageBreakdown['costs']
}) {
  if (!costs) {
    return <div className="text-[11px] leading-4 text-base-content/62">{labels.unavailable}</div>
  }
  const rows = [
    [labels.input, costs.input] as [string, number],
    [labels.cacheWrite, costs.cacheWrite] as [string, number],
    [labels.cacheRead, costs.cacheRead] as [string, number],
    [labels.output, costs.output] as [string, number],
    [labels.reasoning, costs.reasoning] as [string, number],
  ].filter(([, value]) => hasCostValue(value))
  if (!rows.length) {
    return <div className="text-[11px] leading-4 text-base-content/62">{formatCurrency(0)}</div>
  }
  return (
    <div className="space-y-1">
      {rows.map(([label, value]) => (
        <div key={String(label)} className="grid grid-cols-[minmax(0,1fr)_auto] gap-3 text-[11px] leading-4">
          <span className="min-w-0 truncate text-base-content/68">{label}</span>
          <span className="font-mono font-semibold text-base-content">{formatCurrency(value)}</span>
        </div>
      ))}
    </div>
  )
}

function TokenRows({
  item,
  formatNumber,
  labels,
}: Pick<UsageBreakdownTooltipProps, 'formatNumber' | 'labels'> & {
  item: Pick<UsageBreakdown, 'cacheWriteTokens' | 'cacheReadTokens' | 'outputTokens'>
}) {
  return (
    <div className="space-y-1">
      {[
        [labels.cacheWrite, item.cacheWriteTokens],
        [labels.cacheRead, item.cacheReadTokens],
        [labels.output, item.outputTokens],
      ].map(([label, value]) => (
        <div key={String(label)} className="grid grid-cols-[minmax(0,1fr)_auto] gap-3 text-[11px] leading-4">
          <span className="min-w-0 truncate text-base-content/68">{label}</span>
          <span className="font-mono font-semibold text-base-content">{formatNumber(Number(value))}</span>
        </div>
      ))}
    </div>
  )
}

export function UsageBreakdownTooltip({
  title,
  breakdown,
  kind,
  formatNumber,
  formatCurrency,
  labels,
}: UsageBreakdownTooltipProps) {
  const models = [...(breakdown?.models ?? [])]
    .filter((model) => {
      if (kind === 'tokens') {
        return model.cacheWriteTokens > 0 || model.cacheReadTokens > 0 || model.outputTokens > 0
      }
      return model.costs != null || model.cacheWriteTokens > 0 || model.cacheReadTokens > 0 || model.outputTokens > 0
    })
    .sort((left, right) => {
      const leftValue = kind === 'tokens'
        ? left.cacheWriteTokens + left.cacheReadTokens + left.outputTokens
        : (left.costs?.input ?? 0) + (left.costs?.cacheWrite ?? 0) + (left.costs?.cacheRead ?? 0) + (left.costs?.output ?? 0) + (left.costs?.reasoning ?? 0)
      const rightValue = kind === 'tokens'
        ? right.cacheWriteTokens + right.cacheReadTokens + right.outputTokens
        : (right.costs?.input ?? 0) + (right.costs?.cacheWrite ?? 0) + (right.costs?.cacheRead ?? 0) + (right.costs?.output ?? 0) + (right.costs?.reasoning ?? 0)
      return rightValue - leftValue || left.model.localeCompare(right.model)
    })

  const totalRows: ReactNode = kind === 'tokens'
    ? <TokenRows item={breakdown ?? { cacheWriteTokens: 0, cacheReadTokens: 0, outputTokens: 0 }} formatNumber={formatNumber} labels={labels} />
    : <CostRows costs={breakdown?.costs} formatCurrency={formatCurrency} labels={labels} />

  return (
    <div data-testid={`usage-breakdown-tooltip-${kind}`} className="max-h-[min(24rem,calc(100vh-4rem))] space-y-3 overflow-y-auto">
      <div className="border-b border-base-300/45 pb-2">
        <div className="text-[11px] font-semibold leading-4 text-base-content/72">{title}</div>
        <div className="mt-1 text-[10px] font-semibold leading-4 text-base-content/52">{labels.total}</div>
        <div className="mt-1.5">{totalRows}</div>
      </div>
      {models.length ? (
        <div className="space-y-3">
          {models.map((model) => (
            <div key={model.model} className="space-y-1.5">
              <div className="truncate font-mono text-[10px] font-semibold leading-4 text-base-content/62" title={modelLabel(model.model, labels.unknownModel)}>
                {modelLabel(model.model, labels.unknownModel)}
              </div>
              {kind === 'tokens' ? (
                <TokenRows item={model} formatNumber={formatNumber} labels={labels} />
              ) : (
                <CostRows costs={model.costs} formatCurrency={formatCurrency} labels={labels} />
              )}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}
