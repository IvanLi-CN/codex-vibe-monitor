import { describe, expect, it } from 'vitest'
import {
  FORWARD_PROXY_OPEN_RESYNC_COOLDOWN_MS,
  FORWARD_PROXY_SSE_REFRESH_THROTTLE_MS,
  getForwardProxySseRefreshDelay,
  shouldTriggerForwardProxyOpenResync,
} from './useForwardProxyLiveStats'

describe('useForwardProxyLiveStats sync guards', () => {
  it('returns zero delay when SSE refresh window is already open', () => {
    const delay = getForwardProxySseRefreshDelay(10_000, 10_000 + FORWARD_PROXY_SSE_REFRESH_THROTTLE_MS)
    expect(delay).toBe(0)
  })

  it('returns remaining delay when SSE events arrive too quickly', () => {
    const delay = getForwardProxySseRefreshDelay(20_000, 22_200)
    expect(delay).toBe(FORWARD_PROXY_SSE_REFRESH_THROTTLE_MS - 2_200)
  })

  it('throttles reconnect backfill in cooldown window', () => {
    const allowed = shouldTriggerForwardProxyOpenResync(30_000, 30_000 + FORWARD_PROXY_OPEN_RESYNC_COOLDOWN_MS - 1)
    expect(allowed).toBe(false)
  })

  it('allows forced reconnect backfill regardless of cooldown', () => {
    const allowed = shouldTriggerForwardProxyOpenResync(40_000, 40_500, true)
    expect(allowed).toBe(true)
  })
})
