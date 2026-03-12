import { useEffect, useRef, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { AppLayout } from './AppLayout'
import { I18nProvider } from '../i18n'
import AccountPoolLayout from '../pages/account-pool/AccountPoolLayout'

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

function MockPage({ title, description }: { title: string; description: string }) {
  return (
    <section className="surface-panel overflow-hidden">
      <div className="surface-panel-body gap-4">
        <div className="section-heading">
          <span className="text-xs font-semibold uppercase tracking-[0.24em] text-primary/80">
            Site shell preview
          </span>
          <h2 className="section-title text-2xl">{title}</h2>
          <p className="section-description max-w-2xl">{description}</p>
        </div>
        <div className="grid gap-3 md:grid-cols-3">
          <div className="rounded-2xl border border-base-300 bg-base-100/90 p-4">
            <p className="text-sm font-medium text-base-content">Live SSE</p>
            <p className="mt-2 text-3xl font-semibold text-primary">Connected</p>
            <p className="mt-1 text-sm text-base-content/70">Header pulse, footer version, and nav all render together.</p>
          </div>
          <div className="rounded-2xl border border-base-300 bg-base-100/90 p-4">
            <p className="text-sm font-medium text-base-content">Backend version</p>
            <p className="mt-2 text-3xl font-semibold text-primary">v0.2.0</p>
            <p className="mt-1 text-sm text-base-content/70">Version endpoint is mocked for Storybook isolation.</p>
          </div>
          <div className="rounded-2xl border border-base-300 bg-base-100/90 p-4">
            <p className="text-sm font-medium text-base-content">Navigation</p>
            <p className="mt-2 text-3xl font-semibold text-primary">5 tabs</p>
            <p className="mt-1 text-sm text-base-content/70">Dashboard, stats, live, account pool, settings.</p>
          </div>
        </div>
      </div>
    </section>
  )
}

function StorybookAppShellMock({ children }: { children: ReactNode }) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null)

  useEffect(() => {
    originalFetchRef.current = window.fetch.bind(window)
    originalEventSourceRef.current = window.EventSource

    window.fetch = async (input, init) => {
      const inputUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      const parsedUrl = new URL(inputUrl, window.location.origin)
      if (parsedUrl.pathname === '/api/version') {
        return new Response(JSON.stringify({ backend: 'v0.2.0' }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
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
    }
  }, [])

  return <>{children}</>
}

const meta = {
  title: 'Shell/Layout/App Layout',
  component: AppLayout,
  tags: ['autodocs'],
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorybookAppShellMock>
          <MemoryRouter initialEntries={['/account-pool/upstream-accounts']}>
            <Routes>
              <Route path="/" element={<Story />}>
                <Route
                  path="dashboard"
                  element={
                    <MockPage
                      title="Dashboard overview"
                      description="Global site layout preview with dashboard content mounted in the outlet."
                    />
                  }
                />
                <Route
                  path="stats"
                  element={
                    <MockPage
                      title="Stats workspace"
                      description="The same app shell can host time-series analytics and quota summaries."
                    />
                  }
                />
                <Route
                  path="live"
                  element={
                    <MockPage
                      title="Live monitor"
                      description="Realtime stream tables render inside the same site-wide shell."
                    />
                  }
                />
                <Route path="account-pool" element={<AccountPoolLayout />}>
                  <Route
                    path="upstream-accounts"
                    element={
                      <MockPage
                        title="Account Pool module active"
                        description="This story shows the whole site shell while the account-pool module is the active top-level tab."
                      />
                    }
                  />
                </Route>
                <Route
                  path="settings"
                  element={
                    <MockPage
                      title="Settings panel"
                      description="Global controls and feature toggles remain framed by the same app shell."
                    />
                  }
                />
              </Route>
            </Routes>
          </MemoryRouter>
        </StorybookAppShellMock>
      </I18nProvider>
    ),
  ],
  parameters: {
    layout: 'fullscreen',
  },
} satisfies Meta<typeof AppLayout>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {}
