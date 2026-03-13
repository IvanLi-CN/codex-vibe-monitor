import { describe, expect, it } from 'vitest'
import {
  UPSTREAM_STICKY_OPEN_RESYNC_COOLDOWN_MS,
  UPSTREAM_STICKY_SSE_REFRESH_THROTTLE_MS,
  getUpstreamStickySseRefreshDelay,
  shouldTriggerUpstreamStickyOpenResync,
} from './useUpstreamStickyConversations'

describe('useUpstreamStickyConversations sync guards', () => {
  it('returns zero delay when the SSE refresh window is already open', () => {
    const delay = getUpstreamStickySseRefreshDelay(10_000, 10_000 + UPSTREAM_STICKY_SSE_REFRESH_THROTTLE_MS)
    expect(delay).toBe(0)
  })

  it('returns the remaining delay when refreshes are too dense', () => {
    const delay = getUpstreamStickySseRefreshDelay(20_000, 22_000)
    expect(delay).toBe(UPSTREAM_STICKY_SSE_REFRESH_THROTTLE_MS - 2_000)
  })

  it('throttles open resync inside the cooldown window', () => {
    const allowed = shouldTriggerUpstreamStickyOpenResync(
      30_000,
      30_000 + UPSTREAM_STICKY_OPEN_RESYNC_COOLDOWN_MS - 1,
    )
    expect(allowed).toBe(false)
  })

  it('allows forced open resync regardless of cooldown', () => {
    const allowed = shouldTriggerUpstreamStickyOpenResync(40_000, 40_100, true)
    expect(allowed).toBe(true)
  })
})
