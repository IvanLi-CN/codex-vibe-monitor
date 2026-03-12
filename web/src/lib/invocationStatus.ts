import type { ApiInvocation } from './api'

function hasResolvedFailure(record: Pick<ApiInvocation, 'failureClass'>) {
  const failureClass = record.failureClass?.trim().toLowerCase()
  return Boolean(failureClass && failureClass !== 'none')
}

export function resolveInvocationDisplayStatus(record: Pick<ApiInvocation, 'status' | 'failureClass'>) {
  const raw = (record.status ?? '').trim()
  const lower = raw.toLowerCase()

  if (hasResolvedFailure(record) && (!raw || lower === 'success' || lower === 'running' || lower === 'pending')) {
    return 'failed'
  }

  return raw
}
