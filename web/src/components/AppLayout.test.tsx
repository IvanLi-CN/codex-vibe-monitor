/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { AppLayout } from './AppLayout'

const sseMocks = vi.hoisted(() => ({
  subscribeToSse: vi.fn(() => () => undefined),
  requestImmediateReconnect: vi.fn(),
}))

const hookMocks = vi.hoisted(() => ({
  useSseStatus: vi.fn(() => ({
    phase: 'connected',
    downtimeMs: 0,
    autoReconnect: true,
    nextRetryAt: null,
  })),
  useUpdateAvailable: vi.fn(() => ({
    currentVersion: null,
    availableVersion: null,
    visible: false,
    dismiss: vi.fn(),
    reload: vi.fn(),
  })),
  fetchVersion: vi.fn(() => Promise.resolve({ backend: 'v0.2.0' })),
}))

vi.mock('../lib/sse', () => ({
  subscribeToSse: sseMocks.subscribeToSse,
  requestImmediateReconnect: sseMocks.requestImmediateReconnect,
}))

vi.mock('../hooks/useSseStatus', () => ({
  default: hookMocks.useSseStatus,
}))

vi.mock('../hooks/useUpdateAvailable', () => ({
  default: hookMocks.useUpdateAvailable,
}))

vi.mock('../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../lib/api')>('../lib/api')
  return {
    ...actual,
    fetchVersion: hookMocks.fetchVersion,
  }
})

vi.mock('../i18n', () => ({
  supportedLocales: ['zh', 'en'],
  useTranslation: () => ({
    locale: 'zh',
    setLocale: vi.fn(),
    t: (key: string, values?: Record<string, string | number>) => {
      switch (key) {
        case 'app.nav.dashboard':
          return '总览'
        case 'app.nav.stats':
          return '统计'
        case 'app.nav.live':
          return '实况'
        case 'app.nav.records':
          return '记录'
        case 'app.nav.accountPool':
          return '号池'
        case 'app.nav.settings':
          return '设置'
        case 'app.brand':
          return 'Codex Vibe Monitor'
        case 'app.logoAlt':
          return 'logo'
        case 'app.theme.currentDark':
          return '深色'
        case 'app.theme.currentLight':
          return '浅色'
        case 'app.theme.switchToLight':
          return '切换浅色'
        case 'app.theme.switchToDark':
          return '切换深色'
        case 'app.theme.switcherAria':
          return '切换主题'
        case 'app.language.option.zh':
          return '中文'
        case 'app.language.option.en':
          return 'English'
        case 'app.language.switcherAria':
          return '切换语言'
        case 'app.footer.newVersionAvailable':
          return '新版本可用'
        case 'app.footer.frontendVersion':
          return '前端版本'
        case 'app.footer.backendVersion':
          return '后端版本'
        case 'app.footer.versionUnavailable':
          return '不可用'
        case 'app.footer.sameVersion':
          return '已同步'
        case 'app.footer.updateAvailable':
          return '可更新'
        case 'app.sse.banner.durationChip':
          return `${values?.minutes ?? 0}:${values?.seconds ?? '00'}`
        case 'app.sse.banner.retryingNow':
          return '正在重连'
        case 'app.sse.banner.autoDisabled':
          return '自动重连已关闭'
        case 'app.sse.banner.title':
          return '连接异常'
        case 'app.sse.banner.description':
          return 'SSE 断开'
        case 'app.sse.banner.reconnectButton':
          return '立即重连'
        case 'app.version.loading':
          return '加载中'
        default:
          return key
      }
    },
  }),
}))

vi.mock('../theme', () => ({
  useTheme: () => ({
    themeMode: 'dark',
    toggleTheme: vi.fn(),
  }),
}))

vi.mock('./UpdateAvailableBanner', () => ({
  UpdateAvailableBanner: () => null,
}))

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  vi.clearAllMocks()
})

function render(initialEntry = '/dashboard') {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(
      <MemoryRouter initialEntries={[initialEntry]}>
        <Routes>
          <Route path="/" element={<AppLayout />}>
            <Route path="dashboard" element={<div>dashboard page</div>} />
            <Route path="stats" element={<div>stats page</div>} />
            <Route path="live" element={<div>live page</div>} />
            <Route path="records" element={<div>records page</div>} />
            <Route path="account-pool" element={<div>account pool page</div>} />
            <Route path="settings" element={<div>settings page</div>} />
          </Route>
        </Routes>
      </MemoryRouter>,
    )
  })
}

describe('AppLayout', () => {
  it('uses the shared segmented control helper for the top navigation', async () => {
    hookMocks.useUpdateAvailable.mockReturnValue({
      currentVersion: null,
      availableVersion: null,
      visible: false,
      dismiss: vi.fn(),
      reload: vi.fn(),
    })
    hookMocks.useSseStatus.mockReturnValue({
      phase: 'connected',
      downtimeMs: 0,
      autoReconnect: true,
      nextRetryAt: null,
    })
    hookMocks.fetchVersion.mockResolvedValue({ backend: 'v0.2.0' })

    render('/dashboard')

    await act(async () => {
      await Promise.resolve()
    })

    const navGroup = host?.querySelector('nav .segmented-control')
    const dashboardLink = host?.querySelector('a[href="/dashboard"]')
    const settingsLink = host?.querySelector('a[href="/settings"]')

    expect(navGroup).not.toBeNull()
    expect(dashboardLink?.className).toContain('segmented-control-item')
    expect(dashboardLink?.className).toContain('segmented-control-item--active')
    expect(settingsLink?.className).toContain('segmented-control-item')
    expect(settingsLink?.className).not.toContain('segmented-control-item--active')
  })
})
