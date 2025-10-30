import type { BroadcastPayload } from './api'
import { createEventSource } from './api'

export type SseListener = (payload: BroadcastPayload) => void

let eventSource: EventSource | null = null
const listeners = new Set<SseListener>()

function handleMessage(event: MessageEvent<string>) {
  try {
    const payload = JSON.parse(event.data) as BroadcastPayload
    listeners.forEach((listener) => listener(payload))
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
  return eventSource
}

function cleanupEventSource() {
  if (eventSource && listeners.size === 0) {
    eventSource.removeEventListener('message', handleMessage as EventListener)
    eventSource.removeEventListener('error', handleError)
    eventSource.close()
    eventSource = null
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
