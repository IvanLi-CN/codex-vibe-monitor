/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { AppLayout, HEADER_BRAND_ACTIVITY_HOLD_MS } from './AppLayout'

const sseMocks = vi.hoisted(() => {
  const state = {
    lastMessageListener: null as ((payload?: unknown) => void) | null,
    subscribeToSse: vi.fn((listener: (payload?: unknown) => void) => {
      state.lastMessageListener = listener
      return () => {
        if (state.lastMessageListener === listener) {
          state.lastMessageListener = null
        }
      }
    }),
    requestImmediateReconnect: vi.fn(),
  }
  return state
})

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

vi.mock('../../lib/sse', () => ({
  subscribeToSse: sseMocks.subscribeToSse,
  requestImmediateReconnect: sseMocks.requestImmediateReconnect,
}))

vi.mock('../../hooks/useSseStatus', () => ({
  default: hookMocks.useSseStatus,
}))

vi.mock('../../hooks/useUpdateAvailable', () => ({
  default: hookMocks.useUpdateAvailable,
}))

vi.mock('../../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../../lib/api')>('../../lib/api')
  return {
    ...actual,
    fetchVersion: hookMocks.fetchVersion,
  }
})

vi.mock('../../i18n', () => ({
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
        case 'app.nav.system':
          return '系统'
        case 'app.brand':
          return 'Codex Vibe Monitor'
        case 'app.logoAlt':
          return 'product icon'
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

vi.mock('../../theme', () => ({
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
  sseMocks.lastMessageListener = null
  vi.useRealTimers()
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
            <Route path="system/*" element={<div>system page</div>} />
          </Route>
        </Routes>
      </MemoryRouter>,
    )
  })
}

describe('AppLayout', () => {
  it('keeps desktop navigation behind the compact hamburger menu contract', async () => {
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
    const desktopNavigation = navGroup?.parentElement
    const mobileMenuButton = host?.querySelector('button[aria-label="app.nav.openMenu"]') as HTMLButtonElement | null
    const dashboardLink = host?.querySelector('a[href="/dashboard"]')
    const systemLink = host?.querySelector('a[href="/system"]')
    const logoMark = host?.querySelector('[data-testid="app-header-logo-mark"]')
    const logoImage = host?.querySelector('img[src="/brand-mark.svg"][alt="product icon"]')

    expect(navGroup).not.toBeNull()
    expect(desktopNavigation?.className).toContain('hidden')
    expect(desktopNavigation?.className).toContain('min-[1024px]:block')
    expect(dashboardLink?.className).toContain('segmented-control-item')
    expect(dashboardLink?.className).toContain('segmented-control-item--active')
    expect(systemLink?.className).toContain('segmented-control-item')
    expect(systemLink?.className).not.toContain('segmented-control-item--active')
    expect(logoMark?.getAttribute('data-logo-state')).toBe('idle')
    expect(logoMark?.className).toContain('h-10')
    expect(logoMark?.className).toContain('w-10')
    expect(logoImage).not.toBeNull()

    expect(mobileMenuButton).not.toBeNull()
    act(() => {
      mobileMenuButton?.click()
    })
    expect(host?.querySelector('#app-mobile-navigation')).not.toBeNull()
    expect(host?.querySelector('a[href="/account-pool/groups"]')).not.toBeNull()
    expect(host?.querySelector('a[href="/system/tasks"]')).not.toBeNull()
  })

  it('keeps the header logo mark active across bursty updates until the recent-activity window expires', async () => {
    vi.useFakeTimers()
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

    const logoMark = host?.querySelector('[data-testid="app-header-logo-mark"]')
    expect(logoMark?.getAttribute('data-logo-state')).toBe('idle')

    act(() => {
      sseMocks.lastMessageListener?.()
    })
    expect(logoMark?.getAttribute('data-logo-state')).toBe('active')

    await act(async () => {
      vi.advanceTimersByTime(HEADER_BRAND_ACTIVITY_HOLD_MS - 500)
      await Promise.resolve()
    })
    expect(logoMark?.getAttribute('data-logo-state')).toBe('active')

    act(() => {
      sseMocks.lastMessageListener?.()
    })

    await act(async () => {
      vi.advanceTimersByTime(1000)
      await Promise.resolve()
    })
    expect(logoMark?.getAttribute('data-logo-state')).toBe('active')

    await act(async () => {
      vi.advanceTimersByTime(HEADER_BRAND_ACTIVITY_HOLD_MS + 20)
      await Promise.resolve()
    })
    expect(logoMark?.getAttribute('data-logo-state')).toBe('idle')
  })
})
