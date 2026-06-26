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
let originalLocalStorageDescriptor: PropertyDescriptor | undefined

function createMemoryStorage(): Storage {
  const store = new Map<string, string>()

  return {
    get length() {
      return store.size
    },
    clear() {
      store.clear()
    },
    getItem(key: string) {
      return store.get(key) ?? null
    },
    key(index: number) {
      return Array.from(store.keys())[index] ?? null
    },
    removeItem(key: string) {
      store.delete(key)
    },
    setItem(key: string, value: string) {
      store.set(key, value)
    },
  }
}

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
    originalLocalStorageDescriptor ??= Object.getOwnPropertyDescriptor(window, 'localStorage')
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      value: createMemoryStorage(),
    })
    window.localStorage.setItem('codex-vibe-monitor.locale', 'zh')
    apiMocks.fetchSystemStatus.mockResolvedValue({
      liveInvocationsCount: 6,
      successCount: 3,
      nonSuccessCount: 3,
      completedArchiveBatchesCount: 2,
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
    if (originalLocalStorageDescriptor) {
      Object.defineProperty(window, 'localStorage', originalLocalStorageDescriptor)
    }
    vi.useRealTimers()
  })

  it('loads system status immediately and refreshes every minute', async () => {
    renderPage()

    await act(async () => {
      await Promise.resolve()
    })

    expect(apiMocks.fetchSystemStatus).toHaveBeenCalledTimes(1)
    expect(host?.querySelector('[data-testid="system-status-layout"]')).not.toBeNull()
    expect(host?.querySelector('[data-testid="system-status-overview"]')).not.toBeNull()
    expect(host?.querySelector('[data-testid="system-status-records-section"]')).not.toBeNull()
    expect(host?.querySelector('[data-testid="system-status-archive-section"]')).not.toBeNull()
    expect(host?.textContent ?? '').toContain('实际磁盘占用总览')
    expect(host?.textContent ?? '').toContain('数据库记录概况')
    expect(host?.textContent ?? '').toContain('归档与逻辑体量')
    expect(host?.textContent ?? '').toContain('当前项目磁盘占用')
    expect(host?.textContent ?? '').toContain('当前项目磁盘占用 = raw payload 并集总量 + archive + 数据库 + 其他运行文件。')
    expect(host?.textContent ?? '').toContain('并集总量')
    expect(host?.textContent ?? '').toContain('侧向拆分')
    expect(host?.textContent ?? '').toContain('live invocations')
    expect(host?.textContent ?? '').toContain('已完成归档批次数')
    expect(host?.querySelector('[data-testid="system-status-request-raw-breakdown"]')?.textContent ?? '').toContain('request 侧 raw payload')
    expect(host?.querySelector('[data-testid="system-status-request-raw-breakdown"]')?.textContent ?? '').toContain('体积')
    expect(host?.querySelector('[data-testid="system-status-request-raw-breakdown"]')?.textContent ?? '').toContain('数量')
    expect(host?.querySelector('[data-testid="system-status-request-raw-breakdown"]')?.textContent ?? '').toContain('侧向拆分')
    expect(host?.querySelector('[data-testid="system-status-request-raw-breakdown"]')?.textContent ?? '').toContain('4.0 KB')
    expect(host?.querySelector('[data-testid="system-status-request-raw-breakdown"]')?.textContent ?? '').toContain('2')
    expect(host?.querySelector('[data-testid="system-status-response-raw-breakdown"]')?.textContent ?? '').toContain('response 侧 raw payload')
    expect(host?.querySelector('[data-testid="system-status-response-raw-breakdown"]')?.textContent ?? '').toContain('2.0 KB')
    expect(host?.querySelector('[data-testid="system-status-response-raw-breakdown"]')?.textContent ?? '').toContain('3')
    expect(host?.textContent ?? '').toContain('raw payload 总量按 request + response 去重文件并集统计；request / response 体积只用于解释侧向分布，不能直接相加回总量。')

    await act(async () => {
      vi.advanceTimersByTime(60_000)
      await Promise.resolve()
    })

    expect(apiMocks.fetchSystemStatus).toHaveBeenCalledTimes(2)
  })
})
