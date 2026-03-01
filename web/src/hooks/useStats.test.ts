import { describe, expect, it, vi } from 'vitest'
import {
  CURRENT_SUMMARY_OPEN_RESYNC_COOLDOWN_MS,
  CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS,
  createUnsupportedRefreshGate,
  getCurrentSummarySseRefreshDelay,
  mergePendingSummarySilentOption,
  runUnsupportedSummaryRefresh,
  shouldTriggerCurrentSummaryOpenResync,
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

  it('returns zero delay when current summary refresh is outside throttle window', () => {
    const delay = getCurrentSummarySseRefreshDelay(10_000, 10_000 + CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS)
    expect(delay).toBe(0)
  })

  it('returns remaining delay when current summary refresh is still throttled', () => {
    const delay = getCurrentSummarySseRefreshDelay(20_000, 20_250)
    expect(delay).toBe(CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS - 250)
  })

  it('merges pending silent options to preserve non-silent requests', () => {
    expect(mergePendingSummarySilentOption(null, true)).toBe(true)
    expect(mergePendingSummarySilentOption(true, false)).toBe(false)
    expect(mergePendingSummarySilentOption(false, true)).toBe(false)
  })

  it('throttles current summary reconnect resync in cooldown window', () => {
    const allowed = shouldTriggerCurrentSummaryOpenResync(
      30_000,
      30_000 + CURRENT_SUMMARY_OPEN_RESYNC_COOLDOWN_MS - 1,
    )
    expect(allowed).toBe(false)
  })

  it('allows forced reconnect resync regardless of cooldown', () => {
    const allowed = shouldTriggerCurrentSummaryOpenResync(40_000, 40_500, true)
    expect(allowed).toBe(true)
  })
})
