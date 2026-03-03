import { describe, expect, it } from 'vitest'
import {
  PROMPT_CACHE_OPEN_RESYNC_COOLDOWN_MS,
  PROMPT_CACHE_SSE_REFRESH_THROTTLE_MS,
  getPromptCacheSseRefreshDelay,
  shouldTriggerPromptCacheOpenResync,
} from './usePromptCacheConversations'

describe('usePromptCacheConversations sync guards', () => {
  it('returns zero delay when refresh window is already open', () => {
    const delay = getPromptCacheSseRefreshDelay(10_000, 10_000 + PROMPT_CACHE_SSE_REFRESH_THROTTLE_MS)
    expect(delay).toBe(0)
  })

  it('returns remaining delay when events are too dense', () => {
    const delay = getPromptCacheSseRefreshDelay(20_000, 22_300)
    expect(delay).toBe(PROMPT_CACHE_SSE_REFRESH_THROTTLE_MS - 2_300)
  })

  it('throttles open resync inside cooldown window', () => {
    const allowed = shouldTriggerPromptCacheOpenResync(30_000, 30_000 + PROMPT_CACHE_OPEN_RESYNC_COOLDOWN_MS - 1)
    expect(allowed).toBe(false)
  })

  it('allows forced open resync regardless of cooldown', () => {
    const allowed = shouldTriggerPromptCacheOpenResync(40_000, 40_200, true)
    expect(allowed).toBe(true)
  })
})
