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
    model: string
    cacheWrite: string
    cacheRead: string
    output: string
    input: string
    reasoning: string
    unavailable: string
    tokenUnavailable: string
    unknownModel: string
  }
}

interface BreakdownTableRow {
  label: string
  values?: string[]
  unavailable?: string
}

interface BreakdownTableColumn {
  label: string
}

function modelLabel(model: string, unknownModel: string) {
  return model === 'unknown' ? unknownModel : model
}

function BreakdownTable({
  title,
  columns,
  rows,
  modelLabel: modelColumnLabel,
  modelWidth,
}: {
  title: string
  columns: readonly BreakdownTableColumn[]
  rows: readonly BreakdownTableRow[]
  modelLabel: string
  modelWidth: string
}) {
  return (
    <table className="w-full table-fixed border-collapse text-[10px] leading-4 sm:text-[11px]">
      <caption className="sr-only">{title}</caption>
      <thead className="border-y border-base-300/50 bg-base-200/45 text-[9px] font-semibold text-base-content/58 sm:text-[10px]">
        <tr>
          <th scope="col" className="px-1.5 py-1.5 text-left font-semibold" style={{ width: modelWidth }}>
            {modelColumnLabel}
          </th>
          {columns.map((column) => (
            <th key={column.label} scope="col" className="px-1 py-1.5 text-right font-semibold break-words">
              {column.label}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row, rowIndex) => (
          <tr key={row.label} className={rowIndex === 0 ? 'border-b border-base-300/50 bg-base-100/45' : 'border-b border-base-300/30 last:border-b-0'}>
            <th scope="row" className="px-1.5 py-1.5 text-left font-medium text-base-content/76 break-all">
              {row.label}
            </th>
            {row.unavailable ? (
              <td colSpan={columns.length} className="px-1.5 py-1.5 text-left text-base-content/62">
                {row.unavailable}
              </td>
            ) : (
              row.values?.map((value, columnIndex) => (
                <td key={`${row.label}:${columnIndex}`} className="px-1 py-1.5 text-right font-mono font-semibold text-base-content tabular-nums whitespace-nowrap">
                  {value}
                </td>
              ))
            )}
          </tr>
        ))}
      </tbody>
    </table>
  )
}

function CostBreakdownTable({
  title,
  breakdown,
  models,
  formatCurrency,
  labels,
}: Pick<UsageBreakdownTooltipProps, 'formatCurrency' | 'labels'> & {
  title: string
  breakdown?: UsageBreakdown | null
  models: UsageBreakdown['models']
}) {
  const columns = [
    { label: labels.input, key: 'input' },
    { label: labels.cacheWrite, key: 'cacheWrite' },
    { label: labels.cacheRead, key: 'cacheRead' },
    { label: labels.output, key: 'output' },
    { label: labels.reasoning, key: 'reasoning' },
  ] as const
  const rowFor = (label: string, costs: UsageBreakdown['costs']): BreakdownTableRow => {
    if (!costs) return { label, unavailable: labels.unavailable }
    return {
      label,
      values: columns.map(({ key }) => costs[key] === 0 ? '-' : formatCurrency(costs[key])),
    }
  }

  return (
    <BreakdownTable
      title={title}
      modelLabel={labels.model}
      modelWidth="22%"
      columns={columns}
      rows={[
        rowFor(labels.total, breakdown?.costs),
        ...models.map((model) => rowFor(modelLabel(model.model, labels.unknownModel), model.costs)),
      ]}
    />
  )
}

function TokenBreakdownTable({
  title,
  breakdown,
  models,
  formatNumber,
  labels,
}: Pick<UsageBreakdownTooltipProps, 'formatNumber' | 'labels'> & {
  title: string
  breakdown?: UsageBreakdown | null
  models: UsageBreakdown['models']
}) {
  const columns = [
    { label: labels.cacheWrite },
    { label: labels.cacheRead },
    { label: labels.output },
  ]
  const rowFor = (
    label: string,
    item: Pick<UsageBreakdown, 'cacheWriteTokens' | 'cacheReadTokens' | 'outputTokens'>,
  ): BreakdownTableRow => ({
    label,
    values: [
      formatNumber(item.cacheWriteTokens),
      formatNumber(item.cacheReadTokens),
      formatNumber(item.outputTokens),
    ],
  })

  return (
    <BreakdownTable
      title={title}
      modelLabel={labels.model}
      modelWidth="32%"
      columns={columns}
      rows={breakdown
        ? [
            rowFor(labels.total, breakdown),
            ...models.map((model) => rowFor(modelLabel(model.model, labels.unknownModel), model)),
          ]
        : [{ label: labels.total, unavailable: labels.tokenUnavailable }]}
    />
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

  return (
    <div data-testid={`usage-breakdown-tooltip-${kind}`} className="space-y-1.5">
      <div className="px-0.5 text-[11px] font-semibold leading-4 text-base-content/72">{title}</div>
      {kind === 'tokens' ? (
        <TokenBreakdownTable title={title} breakdown={breakdown} models={models} formatNumber={formatNumber} labels={labels} />
      ) : (
        <CostBreakdownTable title={title} breakdown={breakdown} models={models} formatCurrency={formatCurrency} labels={labels} />
      )}
    </div>
  )
}
