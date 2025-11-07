import type { BroadcastPayload } from './api'
import { createEventSource } from './api'

export type SseListener = (payload: BroadcastPayload) => void

let eventSource: EventSource | null = null
const listeners = new Set<SseListener>()
const openListeners = new Set<() => void>()
// let lastEventAt = Date.now()
let reconnectTimer: number | null = null
let connectionWatchdog: number | null = null
let reconnectAttempts = 0
let connectingSince: number | null = null

const BASE_RECONNECT_DELAY_MS = 2000
const MAX_RECONNECT_DELAY_MS = 30000
const CONNECTING_TIMEOUT_MS = 45000
const WATCHDOG_INTERVAL_MS = 5000
const MAX_RETRIES_BEFORE_DISABLE = 5

let sseDisabled = false

// Dev-only heartbeat simulator: when SSE is silent for a while, periodically
// synthesize a small summary update to drive UI animations.
// Dev simulator disabled to avoid data jitter in real runs
// Dev simulator disabled

// dev simulator removed

function ensureDevSimulator() {}

function stopDevSimulator() {}

function handleMessage(event: MessageEvent<string>) {
  try {
    const payload = JSON.parse(event.data) as BroadcastPayload
    listeners.forEach((listener) => listener(payload))
    // no-op
    // no-op
  } catch (err) {
    console.error('Failed to parse SSE message', err)
  }
}

function handleError() {
  // Suppress noisy console logs; rely on silent backoff and dev simulator
  if (listeners.size === 0) return
  // After too many failures, disable SSE for this session to avoid console spam
  if (reconnectAttempts >= MAX_RETRIES_BEFORE_DISABLE) {
    sseDisabled = true
    destroyEventSource()
    stopConnectionWatchdog()
    clearReconnectTimer()
    ensureDevSimulator()
    return
  }
  scheduleReconnect({ immediate: true })
}

function handleOpen() {
  reconnectAttempts = 0
  connectingSince = null
  clearReconnectTimer()
  openListeners.forEach((cb) => {
    try {
      cb()
    } catch {
      // ignore
    }
  })
}

function ensureEventSource() {
  if (sseDisabled) return eventSource
  if (eventSource) return eventSource
  connectingSince = Date.now()
  eventSource = createEventSource('/events')
  eventSource.addEventListener('message', handleMessage as EventListener)
  eventSource.addEventListener('error', handleError)
  eventSource.addEventListener('open', handleOpen)
  ensureDevSimulator()
  startConnectionWatchdog()
  return eventSource
}

function destroyEventSource() {
  if (!eventSource) return
  eventSource.removeEventListener('message', handleMessage as EventListener)
  eventSource.removeEventListener('error', handleError)
  eventSource.removeEventListener('open', handleOpen)
  eventSource.close()
  eventSource = null
}

function cleanupEventSource() {
  if (listeners.size === 0) {
    destroyEventSource()
    stopDevSimulator()
    stopConnectionWatchdog()
    clearReconnectTimer()
    reconnectAttempts = 0
  }
}

function clearReconnectTimer() {
  if (reconnectTimer != null) {
    clearTimeout(reconnectTimer)
    reconnectTimer = null
  }
}

function scheduleReconnect(options: { immediate?: boolean } = {}) {
  if (listeners.size === 0) return
  if (sseDisabled) return
  const { immediate = false } = options
  if (!immediate && reconnectTimer != null) return

  clearReconnectTimer()
  destroyEventSource()

  const delay = immediate
    ? 0
    : Math.min(BASE_RECONNECT_DELAY_MS * 2 ** reconnectAttempts, MAX_RECONNECT_DELAY_MS)
  const nextAttempts = Math.min(reconnectAttempts + 1, 10)

  reconnectTimer = window.setTimeout(() => {
    reconnectTimer = null
    reconnectAttempts = nextAttempts
    ensureEventSource()
  }, delay)
}

function startConnectionWatchdog() {
  if (connectionWatchdog != null) return
  connectionWatchdog = window.setInterval(() => {
    if (sseDisabled) return
    if (!eventSource) return
    if (eventSource.readyState === EventSource.OPEN) {
      connectingSince = null
      return
    }
    if (eventSource.readyState === EventSource.CLOSED) {
      scheduleReconnect({ immediate: true })
      return
    }
    if (eventSource.readyState === EventSource.CONNECTING) {
      if (connectingSince == null) {
        connectingSince = Date.now()
      }
      if (connectingSince != null && Date.now() - connectingSince > CONNECTING_TIMEOUT_MS) {
        // Silent reconnect to avoid console noise
        scheduleReconnect({ immediate: true })
      }
    }
  }, WATCHDOG_INTERVAL_MS)
}

function stopConnectionWatchdog() {
  if (connectionWatchdog != null) {
    clearInterval(connectionWatchdog)
    connectionWatchdog = null
  }
  connectingSince = null
}

export function subscribeToSse(listener: SseListener) {
  listeners.add(listener)
  ensureEventSource()
  return () => {
    listeners.delete(listener)
    cleanupEventSource()
  }
}

export function subscribeToSseOpen(callback: () => void) {
  openListeners.add(callback)
  ensureEventSource()
  return () => {
    openListeners.delete(callback)
    // do not cleanup event source here; rely on message listener cleanup
  }
}
