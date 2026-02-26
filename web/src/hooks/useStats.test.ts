import { describe, expect, it, vi } from 'vitest'
import {
  createUnsupportedRefreshGate,
  runUnsupportedSummaryRefresh,
  shouldHandleUnsupportedSummaryRefresh,
  UNSUPPORTED_SSE_REFRESH_INTERVAL_MS,
} from './useStats'

describe('useSummary unsupported window fallback', () => {
  it('throttles summary event storms for unsupported windows', async () => {
    const gate = createUnsupportedRefreshGate()
    const refresh = vi.fn().mockResolvedValue(undefined)
    const base = UNSUPPORTED_SSE_REFRESH_INTERVAL_MS

    const firstTrigger = await runUnsupportedSummaryRefresh(gate, base, refresh)
    expect(firstTrigger).toBe(true)

    for (let i = 1; i <= 6; i += 1) {
      const triggered = await runUnsupportedSummaryRefresh(gate, base + i * 500, refresh)
      expect(triggered).toBe(false)
    }

    expect(refresh).toHaveBeenCalledTimes(1)
  })

  it('allows refresh again once the 60s gate expires', async () => {
    const gate = createUnsupportedRefreshGate()
    const refresh = vi.fn().mockResolvedValue(undefined)
    const base = UNSUPPORTED_SSE_REFRESH_INTERVAL_MS

    await runUnsupportedSummaryRefresh(gate, base, refresh)

    const tooEarly = await runUnsupportedSummaryRefresh(gate, base + UNSUPPORTED_SSE_REFRESH_INTERVAL_MS - 1, refresh)
    const reopened = await runUnsupportedSummaryRefresh(gate, base + UNSUPPORTED_SSE_REFRESH_INTERVAL_MS, refresh)

    expect(tooEarly).toBe(false)
    expect(reopened).toBe(true)
    expect(refresh).toHaveBeenCalledTimes(2)
  })

  it('recovers after a silent refresh failure and can retry after interval', async () => {
    const gate = createUnsupportedRefreshGate()
    const refresh = vi
      .fn<() => Promise<void>>()
      .mockRejectedValueOnce(new Error('network down'))
      .mockResolvedValue(undefined)
    const base = UNSUPPORTED_SSE_REFRESH_INTERVAL_MS

    const first = await runUnsupportedSummaryRefresh(gate, base, refresh)
    const second = await runUnsupportedSummaryRefresh(gate, base + UNSUPPORTED_SSE_REFRESH_INTERVAL_MS, refresh)

    expect(first).toBe(true)
    expect(second).toBe(true)
    expect(gate.inFlight).toBe(false)
    expect(refresh).toHaveBeenCalledTimes(2)
  })

  it('keeps supported-window summary behavior unchanged', () => {
    expect(shouldHandleUnsupportedSummaryRefresh('1d', '1d', true)).toBe(false)
    expect(shouldHandleUnsupportedSummaryRefresh('30m', '1d', true)).toBe(false)
    expect(shouldHandleUnsupportedSummaryRefresh('1h', 'current', false)).toBe(false)
    expect(shouldHandleUnsupportedSummaryRefresh('1h', 'today', false)).toBe(true)
  })
})
