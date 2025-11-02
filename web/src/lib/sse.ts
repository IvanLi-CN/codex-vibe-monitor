import type { BroadcastPayload, StatsResponse } from './api'
import { createEventSource } from './api'

export type SseListener = (payload: BroadcastPayload) => void

let eventSource: EventSource | null = null
const listeners = new Set<SseListener>()
let lastEventAt = Date.now()

// Dev-only heartbeat simulator: when SSE is silent for a while, periodically
// synthesize a small summary update to drive UI animations.
let devSimTimer: number | null = null
let devSummary: StatsResponse | null = null
const DEV_SILENCE_MS = 5000
const DEV_TICK_MS = 3000

function devEmitSummaryTick() {
  if (!import.meta.env.DEV) return
  // Initialize or increment a fake summary
  if (!devSummary) {
    devSummary = {
      totalCount: 25,
      successCount: 24,
      failureCount: 1,
      totalCost: 0.35,
      totalTokens: 128_000,
    }
  } else {
    // Gentle increments to visualize rolling numbers clearly
    devSummary = {
      totalCount: devSummary.totalCount + 1,
      successCount: devSummary.successCount + 1,
      failureCount: devSummary.failureCount,
      totalCost: +(devSummary.totalCost + 0.01).toFixed(2),
      totalTokens: devSummary.totalTokens + 12_345,
    }
  }
  const payload: BroadcastPayload = {
    type: 'summary',
    window: '1d',
    summary: devSummary,
  }
  listeners.forEach((l) => l(payload))
}

function ensureDevSimulator() {
  if (!import.meta.env.DEV) return
  if (devSimTimer != null) return
  ;(window as unknown as { __DEV_SUMMARY_TICK__?: () => void }).__DEV_SUMMARY_TICK__ = devEmitSummaryTick
  devSimTimer = window.setInterval(() => {
    const now = Date.now()
    if (now - lastEventAt < DEV_SILENCE_MS) return
    devEmitSummaryTick()
  }, DEV_TICK_MS)
}

function stopDevSimulator() {
  if (devSimTimer != null) {
    clearInterval(devSimTimer)
    devSimTimer = null
  }
}

function handleMessage(event: MessageEvent<string>) {
  try {
    const payload = JSON.parse(event.data) as BroadcastPayload
    listeners.forEach((listener) => listener(payload))
    lastEventAt = Date.now()
    // On real traffic, pause dev simulator
    if (import.meta.env.DEV) {
      devSummary = null
    }
  } catch (err) {
    console.error('Failed to parse SSE message', err)
  }
}

function handleError(event: Event) {
  console.warn('SSE connection error', event)
}

function ensureEventSource() {
  if (eventSource) return eventSource
  eventSource = createEventSource('/events')
  eventSource.addEventListener('message', handleMessage as EventListener)
  eventSource.addEventListener('error', handleError)
  ensureDevSimulator()
  return eventSource
}

function cleanupEventSource() {
  if (eventSource && listeners.size === 0) {
    eventSource.removeEventListener('message', handleMessage as EventListener)
    eventSource.removeEventListener('error', handleError)
    eventSource.close()
    eventSource = null
  }
  if (listeners.size === 0) {
    stopDevSimulator()
  }
}

export function subscribeToSse(listener: SseListener) {
  listeners.add(listener)
  ensureEventSource()
  return () => {
    listeners.delete(listener)
    cleanupEventSource()
  }
}
