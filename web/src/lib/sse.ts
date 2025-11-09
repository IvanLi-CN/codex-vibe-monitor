import type { BroadcastPayload } from './api'
import { createEventSource } from './api'

export type SseListener = (payload: BroadcastPayload) => void

export type SseConnectionPhase = 'idle' | 'connecting' | 'reconnecting' | 'connected' | 'disabled'

export interface SseStatus {
  phase: SseConnectionPhase
  downtimeMs: number
  nextRetryAt: number | null
  autoReconnect: boolean
}

type StatusListener = (status: SseStatus) => void

let eventSource: EventSource | null = null
const listeners = new Set<SseListener>()
const openListeners = new Set<() => void>()
const statusListeners = new Set<StatusListener>()

let reconnectTimer: number | null = null
let connectionWatchdog: number | null = null
let reconnectAttempts = 0
let connectingSince: number | null = null
let downtimeStartedAt: number | null = null
let downtimeTicker: number | null = null
let nextRetryAt: number | null = null
let hasConnectedOnce = false

const BASE_RECONNECT_DELAY_MS = 2000
const MAX_RECONNECT_DELAY_MS = 30000
const CONNECTING_TIMEOUT_MS = 45000
const WATCHDOG_INTERVAL_MS = 5000
const MAX_DOWNTIME_BEFORE_DISABLE_MS = 10 * 60 * 1000

let sseDisabled = false
let connectionPhase: SseConnectionPhase = 'idle'
let lastStatus: SseStatus = {
  phase: 'idle',
  downtimeMs: 0,
  nextRetryAt: null,
  autoReconnect: true,
}

function hasActiveSubscribers() {
  return listeners.size + openListeners.size + statusListeners.size > 0
}

function ensureDevSimulator() {}

function stopDevSimulator() {}

function computeStatus(): SseStatus {
  const now = Date.now()
  const downtime = downtimeStartedAt != null ? now - downtimeStartedAt : 0
  return {
    phase: connectionPhase,
    downtimeMs: downtime,
    nextRetryAt,
    autoReconnect: !sseDisabled,
  }
}

function emitStatus() {
  lastStatus = computeStatus()
  statusListeners.forEach((listener) => {
    try {
      listener(lastStatus)
    } catch (err) {
      console.error('Failed to dispatch SSE status update', err)
    }
  })
}

function setConnectionPhase(next: SseConnectionPhase) {
  if (connectionPhase === next) {
    emitStatus()
    return
  }
  connectionPhase = next
  emitStatus()
}

function startDowntimeTicker() {
  if (downtimeTicker != null) return
  downtimeTicker = window.setInterval(() => {
    emitStatus()
    if (
      downtimeStartedAt != null &&
      Date.now() - downtimeStartedAt >= MAX_DOWNTIME_BEFORE_DISABLE_MS &&
      !sseDisabled
    ) {
      disableSse()
    }
  }, 1000)
}

function stopDowntimeTicker() {
  if (downtimeTicker != null) {
    clearInterval(downtimeTicker)
    downtimeTicker = null
  }
}

function beginDowntimeWindow() {
  if (downtimeStartedAt == null) {
    downtimeStartedAt = Date.now()
    startDowntimeTicker()
  }
  emitStatus()
}

function resetDowntimeWindow() {
  downtimeStartedAt = null
  stopDowntimeTicker()
  emitStatus()
}

function disableSse() {
  if (sseDisabled) return
  sseDisabled = true
  destroyEventSource()
  stopConnectionWatchdog()
  clearReconnectTimer()
  setConnectionPhase('disabled')
}

function handleMessage(event: MessageEvent<string>) {
  try {
    const payload = JSON.parse(event.data) as BroadcastPayload
    listeners.forEach((listener) => listener(payload))
    // no-op
  } catch (err) {
    console.error('Failed to parse SSE message', err)
  }
}

function handleError() {
  if (!hasActiveSubscribers()) return
  beginDowntimeWindow()
  scheduleReconnect({ immediate: true })
}

function handleOpen() {
  reconnectAttempts = 0
  connectingSince = null
  hasConnectedOnce = true
  nextRetryAt = null
  clearReconnectTimer()
  resetDowntimeWindow()
  setConnectionPhase('connected')
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
  setConnectionPhase(hasConnectedOnce ? 'reconnecting' : 'connecting')
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
  if (!hasActiveSubscribers()) {
    destroyEventSource()
    stopDevSimulator()
    stopConnectionWatchdog()
    clearReconnectTimer()
    reconnectAttempts = 0
    hasConnectedOnce = false
    sseDisabled = false
    resetDowntimeWindow()
    setConnectionPhase('idle')
  }
}

function clearReconnectTimer() {
  if (reconnectTimer != null) {
    clearTimeout(reconnectTimer)
    reconnectTimer = null
  }
  nextRetryAt = null
  emitStatus()
}

function scheduleReconnect(options: { immediate?: boolean } = {}) {
  if (!hasActiveSubscribers()) return
  if (sseDisabled) return
  const { immediate = false } = options
  if (!immediate && reconnectTimer != null) return

  clearReconnectTimer()
  destroyEventSource()

  const delay = immediate
    ? 0
    : Math.min(BASE_RECONNECT_DELAY_MS * 2 ** reconnectAttempts, MAX_RECONNECT_DELAY_MS)
  const nextAttempts = Math.min(reconnectAttempts + 1, 50)

  nextRetryAt = Date.now() + delay
  emitStatus()

  reconnectTimer = window.setTimeout(() => {
    reconnectTimer = null
    reconnectAttempts = nextAttempts
    nextRetryAt = null
    ensureEventSource()
    emitStatus()
  }, delay)

  setConnectionPhase(hasConnectedOnce ? 'reconnecting' : 'connecting')
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
      beginDowntimeWindow()
      scheduleReconnect({ immediate: true })
      return
    }
    if (eventSource.readyState === EventSource.CONNECTING) {
      if (connectingSince == null) {
        connectingSince = Date.now()
      }
      if (connectingSince != null && Date.now() - connectingSince > CONNECTING_TIMEOUT_MS) {
        beginDowntimeWindow()
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
    cleanupEventSource()
  }
}

export function subscribeToSseStatus(listener: StatusListener) {
  statusListeners.add(listener)
  listener(lastStatus)
  ensureEventSource()
  return () => {
    statusListeners.delete(listener)
    cleanupEventSource()
  }
}

export function getCurrentSseStatus() {
  return lastStatus
}

export function requestImmediateReconnect() {
  if (!hasActiveSubscribers()) return
  if (sseDisabled) {
    sseDisabled = false
  }
  beginDowntimeWindow()
  reconnectAttempts = 0
  scheduleReconnect({ immediate: true })
}

if (typeof document !== 'undefined') {
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState !== 'visible') return
    const status = getCurrentSseStatus()
    if (status.phase === 'connected' || status.phase === 'idle') return
    requestImmediateReconnect()
  })
}

