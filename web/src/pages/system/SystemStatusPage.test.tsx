/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../../i18n'
import SystemStatusPage from './SystemStatusPage'

const apiMocks = vi.hoisted(() => ({
  fetchSystemStatus: vi.fn(),
}))

vi.mock('../../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../../lib/api')>('../../lib/api')
  return {
    ...actual,
    fetchSystemStatus: apiMocks.fetchSystemStatus,
  }
})

let host: HTMLDivElement | null = null
let root: Root | null = null

function renderPage() {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)

  act(() => {
    root?.render(
      <I18nProvider>
        <SystemStatusPage />
      </I18nProvider>,
    )
  })
}

describe('SystemStatusPage', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    window.localStorage.setItem('codex-vibe-monitor.locale', 'zh')
    apiMocks.fetchSystemStatus.mockResolvedValue({
      successCount: 3,
      nonSuccessCount: 3,
      archivedBodies: { count: 9, bytes: 4_096 },
      rawBodies: { count: 5, bytes: 6_144 },
      requestRawBodies: { count: 2, bytes: 4_096 },
      responseRawBodies: { count: 3, bytes: 2_048 },
      databaseBytes: 2_048,
      otherFilesBytes: 8_192,
      refreshedAt: '2026-06-22T08:00:00Z',
    })
  })

  afterEach(() => {
    act(() => {
      root?.unmount()
    })
    host?.remove()
    host = null
    root = null
    apiMocks.fetchSystemStatus.mockReset()
    window.localStorage.removeItem('codex-vibe-monitor.locale')
    vi.useRealTimers()
  })

  it('loads system status immediately and refreshes every minute', async () => {
    renderPage()

    await act(async () => {
      await Promise.resolve()
    })

    expect(apiMocks.fetchSystemStatus).toHaveBeenCalledTimes(1)
    expect(host?.querySelector('[data-testid="system-status-grid"]')).not.toBeNull()
    expect(host?.textContent ?? '').toContain('调用成功数')
    expect(host?.textContent ?? '').toContain('raw payload 体积')
    expect(host?.textContent ?? '').toContain('raw payload 数量')
    expect(host?.textContent ?? '').toContain('request raw payload 数量')
    expect(host?.textContent ?? '').toContain('response raw payload 数量')

    await act(async () => {
      vi.advanceTimersByTime(60_000)
      await Promise.resolve()
    })

    expect(apiMocks.fetchSystemStatus).toHaveBeenCalledTimes(2)
  })
})
