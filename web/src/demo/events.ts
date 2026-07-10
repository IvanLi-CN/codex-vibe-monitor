export type DemoRealtimePayload = {
  type: 'records'
  records: Array<Record<string, unknown>>
}

const listeners = new Set<(payload: DemoRealtimePayload) => void>()

export function subscribeToDemoRealtime(listener: (payload: DemoRealtimePayload) => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function publishDemoRealtime(payload: DemoRealtimePayload) {
  listeners.forEach((listener) => listener(payload))
}
