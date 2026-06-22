/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { MemoryRouter, Outlet, useLocation } from 'react-router-dom'
import { afterEach, describe, expect, it, vi } from 'vitest'
import App from './App'

vi.mock('./components/AppLayout', () => ({
  AppLayout: () => <Outlet />,
}))

vi.mock('./pages/Dashboard', () => ({
  default: () => <div>dashboard page</div>,
}))

vi.mock('./pages/Live', () => ({
  default: () => <div>live page</div>,
}))

vi.mock('./pages/Records', () => ({
  default: () => <div>records page</div>,
}))

vi.mock('./pages/Settings', () => ({
  default: ({ mode }: { mode?: string }) => <div>{`legacy settings page ${mode ?? 'all'}`}</div>,
}))

vi.mock('./pages/Stats', () => ({
  default: () => <div>stats page</div>,
}))

vi.mock('./pages/account-pool/AccountPoolLayout', () => ({
  default: () => <Outlet />,
}))

vi.mock('./pages/account-pool/Groups', () => ({
  default: () => <div>groups page</div>,
}))

vi.mock('./pages/account-pool/MaintenanceRecords', () => ({
  default: () => <div>maintenance records page</div>,
}))

vi.mock('./pages/account-pool/UpstreamAccounts', () => ({
  default: () => <div>upstream accounts page</div>,
}))

vi.mock('./pages/account-pool/UpstreamAccountCreate', () => ({
  default: () => <div>upstream account create page</div>,
}))

vi.mock('./pages/account-pool/Tags', () => ({
  default: () => <div>tags page</div>,
}))

vi.mock('./pages/system/SystemLayout', () => ({
  default: () => <Outlet />,
}))

vi.mock('./pages/system/SystemStatusPage', () => ({
  default: () => <div>system status page</div>,
}))

vi.mock('./pages/system/SystemTasksPage', () => ({
  default: () => <div>system tasks page</div>,
}))

vi.mock('./pages/system/SystemSettingsPage', () => ({
  default: () => <div>system settings page</div>,
}))

vi.mock('./pages/system/SystemProxyPage', () => ({
  default: () => <div>system proxy page</div>,
}))

let host: HTMLDivElement | null = null
let root: Root | null = null

function LocationProbe() {
  const location = useLocation()
  return <div data-testid="location">{location.pathname}</div>
}

function renderApp(entry: string) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)

  act(() => {
    root?.render(
      <MemoryRouter initialEntries={[entry]}>
        <App />
        <LocationProbe />
      </MemoryRouter>,
    )
  })
}

describe('App routes', () => {
  afterEach(() => {
    act(() => {
      root?.unmount()
    })
    host?.remove()
    host = null
    root = null
  })

  it('redirects the legacy settings route to /system/settings', () => {
    renderApp('/settings')

    expect(host?.textContent ?? '').toContain('system settings page')
    expect(host?.querySelector('[data-testid="location"]')?.textContent).toBe('/system/settings')
  })
})
