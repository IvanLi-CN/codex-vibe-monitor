import { describe, expect, it } from 'vitest'
import { resolveInvocationDisplayStatus } from './invocationStatus'

describe('resolveInvocationDisplayStatus', () => {
  it('marks legacy success or running states as failed when failureClass is resolved', () => {
    expect(resolveInvocationDisplayStatus({ status: 'success', failureClass: 'service_failure' })).toBe('failed')
    expect(resolveInvocationDisplayStatus({ status: 'running', failureClass: 'client_failure' })).toBe('failed')
    expect(resolveInvocationDisplayStatus({ status: 'pending', failureClass: 'client_abort' })).toBe('failed')
    expect(resolveInvocationDisplayStatus({ status: '', failureClass: 'service_failure' })).toBe('failed')
  })

  it('preserves explicit upstream http statuses for failed rows', () => {
    expect(resolveInvocationDisplayStatus({ status: 'http_502', failureClass: 'service_failure' })).toBe('http_502')
  })

  it('keeps successful rows untouched when no failure is resolved', () => {
    expect(resolveInvocationDisplayStatus({ status: ' SUCCESS ', failureClass: 'none' })).toBe('SUCCESS')
  })
})
