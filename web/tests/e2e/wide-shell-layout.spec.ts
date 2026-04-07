import { expect, test, type Page } from '@playwright/test'

type RouteCase = {
  path: string
  label: string
  waitFor: (page: Page) => Promise<void>
}

type BoundaryMetrics = {
  left: number
  right: number
  width: number
  centerX: number
}

type ShellMetrics = {
  viewportWidth: number
  rootOverflow: number
  header: BoundaryMetrics
  main: BoundaryMetrics
  footer: BoundaryMetrics
  banner: BoundaryMetrics
  mainContentWidth: number
  pageRootWidth: number
}

const VIEWPORTS = [
  { width: 1660, height: 960 },
  { width: 1873, height: 960 },
]

const ROUTES: RouteCase[] = [
  {
    path: '/#/dashboard',
    label: 'dashboard',
    waitFor: async (page) => {
      await expect(page.getByTestId('today-stats-overview-card')).toBeVisible()
    },
  },
  {
    path: '/#/stats',
    label: 'stats',
    waitFor: async (page) => {
      await expect(page.getByTestId('stats-range-select-trigger')).toBeVisible()
    },
  },
  {
    path: '/#/live',
    label: 'live',
    waitFor: async (page) => {
      await expect(page.getByTestId('live-prompt-cache-selection')).toBeVisible()
    },
  },
  {
    path: '/#/records',
    label: 'records',
    waitFor: async (page) => {
      await expect(page.getByTestId('records-filters-panel')).toBeVisible()
    },
  },
  {
    path: '/#/account-pool/upstream-accounts',
    label: 'account-pool',
    waitFor: async (page) => {
      await expect(page.getByRole('heading', { name: /号池|Account Pool/ })).toBeVisible()
      await expect(page.getByTestId('upstream-accounts-roster-region')).toBeVisible()
    },
  },
  {
    path: '/#/settings',
    label: 'settings',
    waitFor: async (page) => {
      await expect(page.getByRole('heading', { name: /设置|Settings/ }).first()).toBeVisible()
    },
  },
]

async function forceUpdateBanner(page: Page) {
  await page.waitForFunction(() => typeof (window as Window & { __DEV_FORCE_UPDATE_BANNER__?: () => void }).__DEV_FORCE_UPDATE_BANNER__ === 'function')
  await page.evaluate(() => {
    ;(window as Window & { __DEV_FORCE_UPDATE_BANNER__?: () => void }).__DEV_FORCE_UPDATE_BANNER__?.()
  })
  await expect(page.getByTestId('update-available-banner')).toBeVisible()
}

async function readShellMetrics(page: Page): Promise<ShellMetrics> {
  return page.evaluate(() => {
    const doc = document.documentElement
    const readBoundary = (element: HTMLElement): BoundaryMetrics => {
      const rect = element.getBoundingClientRect()
      return {
        left: rect.left,
        right: rect.right,
        width: rect.width,
        centerX: rect.left + rect.width / 2,
      }
    }

    const header = document.querySelector('[data-testid="app-header-inner"]')
    const main = document.querySelector('[data-testid="app-main"]')
    const footer = document.querySelector('[data-testid="app-footer-inner"]')
    const banner = document.querySelector('[data-testid="update-available-banner"]')
    const pageRoot = document.querySelector('[data-testid="app-main"] > *')

    if (!(header instanceof HTMLElement) || !(main instanceof HTMLElement) || !(footer instanceof HTMLElement) || !(banner instanceof HTMLElement) || !(pageRoot instanceof HTMLElement)) {
      throw new Error('missing shell boundary elements')
    }

    const mainRect = main.getBoundingClientRect()
    const mainStyle = getComputedStyle(main)
    const mainPaddingLeft = Number.parseFloat(mainStyle.paddingLeft) || 0
    const mainPaddingRight = Number.parseFloat(mainStyle.paddingRight) || 0
    const pageRootRect = pageRoot.getBoundingClientRect()

    return {
      viewportWidth: window.innerWidth,
      rootOverflow: doc.scrollWidth - doc.clientWidth,
      header: readBoundary(header),
      main: readBoundary(main),
      footer: readBoundary(footer),
      banner: readBoundary(banner),
      mainContentWidth: mainRect.width - mainPaddingLeft - mainPaddingRight,
      pageRootWidth: pageRootRect.width,
    }
  })
}

test.describe('Wide shell layout contract', () => {
  for (const viewport of VIEWPORTS) {
    for (const route of ROUTES) {
      test(`keeps shell aligned at ${route.label} ${viewport.width}px`, async ({ page }) => {
        await page.setViewportSize(viewport)
        await page.goto(route.path)
        await route.waitFor(page)
        await forceUpdateBanner(page)
        await page.waitForTimeout(200)

        const metrics = await readShellMetrics(page)
        const expectedShellWidth = Math.min(1660, viewport.width)
        const expectedBannerWidth = Math.min(1660, viewport.width - 32)
        const expectedCenterX = viewport.width / 2

        test.info().annotations.push({
          type: 'wide-shell-metrics',
          description: JSON.stringify({ route: route.label, viewport, metrics }),
        })

        expect(metrics.rootOverflow).toBeLessThanOrEqual(1)

        expect(Math.abs(metrics.header.width - expectedShellWidth)).toBeLessThanOrEqual(2)
        expect(Math.abs(metrics.main.width - expectedShellWidth)).toBeLessThanOrEqual(2)
        expect(Math.abs(metrics.footer.width - expectedShellWidth)).toBeLessThanOrEqual(2)
        expect(Math.abs(metrics.banner.width - expectedBannerWidth)).toBeLessThanOrEqual(2)

        expect(Math.abs(metrics.header.centerX - expectedCenterX)).toBeLessThanOrEqual(1)
        expect(Math.abs(metrics.main.centerX - expectedCenterX)).toBeLessThanOrEqual(1)
        expect(Math.abs(metrics.footer.centerX - expectedCenterX)).toBeLessThanOrEqual(1)
        expect(Math.abs(metrics.banner.centerX - expectedCenterX)).toBeLessThanOrEqual(1)

        expect(Math.abs(metrics.pageRootWidth - metrics.mainContentWidth)).toBeLessThanOrEqual(2)
      })
    }
  }
})
