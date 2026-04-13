import { useLayoutEffect, useRef, type ReactNode } from 'react'
import type { StorybookPageSseController } from './storybookPageSse'

export interface StorybookRequestContext {
  url: URL
  init?: RequestInit
}

export type StorybookRequestHandler = (
  context: StorybookRequestContext,
) => Response | Promise<Response | undefined> | undefined

const storybookPageEventSources = new Set<MockEventSource>()

function ensureStorybookPageSseController() {
  if (typeof window === 'undefined') return null
  const existing = window.__storybookPageSseController__
  if (existing) return existing

  const controller: StorybookPageSseController = {
    emit: (payload) => {
      const event = new MessageEvent<string>('message', {
        data: JSON.stringify(payload),
      })
      storybookPageEventSources.forEach((eventSource) => {
        eventSource.dispatchEvent(event)
      })
    },
    emitOpen: () => {
      const event = new Event('open')
      storybookPageEventSources.forEach((eventSource) => {
        eventSource.dispatchEvent(event)
      })
    },
    reset: () => {
      Array.from(storybookPageEventSources).forEach((eventSource) => {
        eventSource.close()
      })
      storybookPageEventSources.clear()
    },
  }

  window.__storybookPageSseController__ = controller
  return controller
}

class MockEventSource implements EventTarget {
  static CONNECTING = 0
  static OPEN = 1
  static CLOSED = 2

  readonly url: string
  readonly withCredentials = false
  readyState = MockEventSource.CONNECTING
  onerror: ((this: EventSource, ev: Event) => unknown) | null = null
  onmessage: ((this: EventSource, ev: MessageEvent<string>) => unknown) | null = null
  onopen: ((this: EventSource, ev: Event) => unknown) | null = null

  #listeners = new Map<string, Set<EventListenerOrEventListenerObject>>()

  constructor(url: string | URL) {
    this.url = typeof url === 'string' ? url : url.toString()
    ensureStorybookPageSseController()
    storybookPageEventSources.add(this)
    window.setTimeout(() => {
      if (this.readyState === MockEventSource.CLOSED) return
      this.readyState = MockEventSource.OPEN
      this.#emit('open', new Event('open'))
    }, 40)
  }

  addEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return
    const bucket = this.#listeners.get(type) ?? new Set<EventListenerOrEventListenerObject>()
    bucket.add(listener)
    this.#listeners.set(type, bucket)
  }

  removeEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return
    this.#listeners.get(type)?.delete(listener)
  }

  dispatchEvent(event: Event) {
    this.#emit(event.type, event)
    return true
  }

  close() {
    this.readyState = MockEventSource.CLOSED
    storybookPageEventSources.delete(this)
  }

  #emit(type: string, event: Event) {
    if (type === 'open') this.onopen?.call(this as unknown as EventSource, event)
    if (type === 'error') this.onerror?.call(this as unknown as EventSource, event)
    if (type === 'message') this.onmessage?.call(this as unknown as EventSource, event as MessageEvent<string>)

    for (const listener of this.#listeners.get(type) ?? []) {
      if (typeof listener === 'function') {
        listener(event)
      } else {
        listener.handleEvent(event)
      }
    }
  }
}

export function StorybookPageEnvironment({
  children,
  onRequest,
}: {
  children: ReactNode
  onRequest?: StorybookRequestHandler
}) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null)

  useLayoutEffect(() => {
    originalFetchRef.current = window.fetch.bind(window)
    originalEventSourceRef.current = window.EventSource
    ensureStorybookPageSseController()

    window.fetch = async (input, init) => {
      const inputUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      const url = new URL(inputUrl, window.location.origin)
      const mocked = onRequest ? await onRequest({ url, init }) : undefined
      if (mocked) {
        return mocked
      }
      return (originalFetchRef.current ?? fetch)(input as RequestInfo | URL, init)
    }

    window.EventSource = MockEventSource as unknown as typeof EventSource

    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current
      }
      if (originalEventSourceRef.current) {
        window.EventSource = originalEventSourceRef.current
      }
      storybookPageEventSources.clear()
      delete window.__storybookPageSseController__
    }
  }, [onRequest])

  return <>{children}</>
}

export function FullPageStorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 text-base-content">
      <div className="app-shell-boundary px-4 py-6">{children}</div>
    </div>
  )
}
