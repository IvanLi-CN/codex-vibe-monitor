import type { ApiInvocation } from '../lib/api'

interface InvocationTableProps {
  records: ApiInvocation[]
  isLoading: boolean
  error?: string | null
}

const dateFormatter = new Intl.DateTimeFormat(undefined, {
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
  hour12: false,
})

const numberFormatter = new Intl.NumberFormat('en-US')

export function InvocationTable({ records, isLoading, error }: InvocationTableProps) {
  if (error) {
    return (
      <div className="alert alert-error">
        <span>Failed to load records: {error}</span>
      </div>
    )
  }

  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <span className="loading loading-bars loading-lg" aria-label="Loading records" />
      </div>
    )
  }

  if (records.length === 0) {
    return <div className="alert">No records yet.</div>
  }

  return (
    <div className="overflow-x-auto">
      <table className="table table-zebra">
        <thead>
          <tr>
            <th>Time</th>
            <th>Invoke ID</th>
            <th>Model</th>
            <th>Status</th>
            <th>Input</th>
            <th>Output</th>
            <th>Total Tokens</th>
            <th>Cost (USD)</th>
            <th>Error</th>
          </tr>
        </thead>
        <tbody>
          {records.map((record) => {
            const occurred = new Date(record.occurredAt)
            return (
              <tr key={`${record.invokeId}-${record.occurredAt}`}>
                <td>{isNaN(occurred.getTime()) ? record.occurredAt : dateFormatter.format(occurred)}</td>
                <td className="font-mono text-xs">{record.invokeId}</td>
                <td>{record.model ?? '—'}</td>
                <td>
                  <span
                    className={`badge ${
                      record.status === 'success'
                        ? 'badge-success'
                        : record.status === 'failed'
                          ? 'badge-error'
                          : 'badge-neutral'
                    }`}
                  >
                    {record.status ?? 'unknown'}
                  </span>
                </td>
                <td>{numberFormatter.format(record.inputTokens ?? 0)}</td>
                <td>{numberFormatter.format(record.outputTokens ?? 0)}</td>
                <td>{numberFormatter.format(record.totalTokens ?? 0)}</td>
                <td>${record.cost?.toFixed(4) ?? '0.0000'}</td>
                <td className="max-w-xs truncate" title={record.errorMessage ?? ''}>
                  {record.errorMessage || '—'}
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}
