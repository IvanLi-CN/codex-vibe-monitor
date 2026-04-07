import { test, expect } from '@playwright/test'

const VIEWPORTS = [
  { width: 375, height: 900 },
  { width: 768, height: 900 },
  { width: 1024, height: 900 },
  { width: 1440, height: 900 },
  { width: 1660, height: 900 },
  // Regression for shrink-on-large screens (reported at ~1873px)
  { width: 1873, height: 900 },
]

test.describe('UsageCalendar responsive layout', () => {
  for (const viewport of VIEWPORTS) {
    test(`maintains card fit at ${viewport.width}px`, async ({ page }) => {
      await page.setViewportSize(viewport)
      await page.goto('/dashboard')

      const card = page.getByTestId('usage-calendar-card')
      await card.waitFor({ state: 'visible' })
      const wrapper = page.getByTestId('usage-calendar-wrapper')
      await wrapper.waitFor({ state: 'visible' })
      await page.waitForTimeout(250)

      const layout = await card.evaluate((node) => {
        const rect = node.getBoundingClientRect()
        const inner = node.querySelector('[data-testid="usage-calendar-wrapper"]') as HTMLElement | null
        if (!inner) {
          return {
            cardWidth: rect.width,
            wrapperWidth: rect.width,
            hasWrapper: false,
            scrollWidth: rect.width,
            clientWidth: rect.width,
          }
        }
        const innerRect = inner.getBoundingClientRect()
        const { scrollWidth, clientWidth } = inner
        return {
          cardWidth: rect.width,
          wrapperWidth: innerRect.width,
          hasWrapper: true,
          scrollWidth,
          clientWidth,
        }
      })

      test.info().annotations.push({
        type: 'layout-metrics',
        description: JSON.stringify({ viewport, layout }),
      })

      expect(layout.hasWrapper).toBeTruthy()
      expect(layout.scrollWidth - layout.clientWidth).toBeLessThanOrEqual(80)
      if (viewport.width >= 1280) {
        expect(layout.cardWidth - layout.wrapperWidth).toBeLessThanOrEqual(96)
      }
    })
  }
  
  test('does not jitter width at 1873px', async ({ page }) => {
    await page.setViewportSize({ width: 1873, height: 900 })
    await page.goto('/dashboard')
    const wrapper = page.getByTestId('usage-calendar-wrapper')
    await wrapper.waitFor({ state: 'visible' })
    // wait for calendar body to render
    await page.locator('article').first().waitFor({ state: 'visible' })
    // sample width several times; ensure it stays stable (<= 2px span)
    const samples: number[] = []
    for (let i = 0; i < 6; i++) {
      await page.waitForTimeout(250)
      const w = await wrapper.evaluate((el) => (el as HTMLElement).getBoundingClientRect().width)
      samples.push(Math.round(w))
    }
    const max = Math.max(...samples)
    const min = Math.min(...samples)
    test.info().annotations.push({ type: 'width-samples', description: JSON.stringify(samples) })
    expect(max - min).toBeLessThanOrEqual(2)
  })

  test('centers calendar when stacked at 768px', async ({ page }) => {
    await page.setViewportSize({ width: 768, height: 900 })
    await page.goto('/dashboard')
    const wrapper = page.getByTestId('usage-calendar-wrapper')
    await wrapper.waitFor({ state: 'visible' })
    await page.waitForTimeout(300)
    const gaps = await wrapper.evaluate((el) => {
      const rect = (el as HTMLElement).getBoundingClientRect()
      const article = (el as HTMLElement).querySelector('article') as HTMLElement | null
      if (!article) {
        return { leftGap: 0, rightGap: 0, width: rect.width, articleWidth: rect.width }
      }
      const a = article.getBoundingClientRect()
      const leftGap = Math.round(a.left - rect.left)
      const rightGap = Math.round(rect.right - a.right)
      return { leftGap, rightGap, width: Math.round(rect.width), articleWidth: Math.round(a.width) }
    })
    test.info().annotations.push({ type: 'center-gaps', description: JSON.stringify(gaps) })
    expect(Math.abs(gaps.leftGap - gaps.rightGap)).toBeLessThanOrEqual(16)
  })

  test('does not shift dashboard top row while 90d timeseries is loading', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })

    let releaseTimeseries: (() => void) | null = null
    const gate = new Promise<void>((resolve) => {
      releaseTimeseries = resolve
    })

    await page.route('**/api/stats/timeseries**', async (route) => {
      const requestUrl = new URL(route.request().url())
      const range = requestUrl.searchParams.get('range')
      const bucket = requestUrl.searchParams.get('bucket')
      if (range === '90d' && bucket === '1d') {
        await gate
      }
      await route.continue()
    })

    await page.goto('/dashboard')

    const todayCard = page.getByTestId('today-stats-overview-card')
    const usageCard = page.getByTestId('usage-calendar-card')
    await todayCard.waitFor({ state: 'visible' })
    await usageCard.waitFor({ state: 'visible' })

    // Ensure we are measuring the loading skeleton state, not the hydrated state.
    const pulseBefore = await usageCard.locator('rect.animate-pulse').count()
    expect(pulseBefore).toBeGreaterThan(0)

    const todayBefore = await todayCard.boundingBox()
    const usageBefore = await usageCard.boundingBox()
    expect(todayBefore).not.toBeNull()
    expect(usageBefore).not.toBeNull()

    const todayBox = todayBefore!
    const usageBox = usageBefore!
    test.info().annotations.push({
      type: 'dashboard-top-row-before',
      description: JSON.stringify({
        today: { x: Math.round(todayBox.x), y: Math.round(todayBox.y), w: Math.round(todayBox.width), h: Math.round(todayBox.height) },
        usage: { x: Math.round(usageBox.x), y: Math.round(usageBox.y), w: Math.round(usageBox.width), h: Math.round(usageBox.height) },
      }),
    })

    // Two cards should stay on the same row (desktop layout) even during loading.
    expect(Math.abs(todayBox.y - usageBox.y)).toBeLessThanOrEqual(8)
    expect(todayBox.x + todayBox.width).toBeLessThan(usageBox.x)

    const waitTimeseries = page.waitForResponse((resp) => {
      if (!resp.url().includes('/api/stats/timeseries')) return false
      try {
        const url = new URL(resp.url())
        return url.searchParams.get('range') === '90d' && url.searchParams.get('bucket') === '1d'
      } catch {
        return false
      }
    })

    releaseTimeseries?.()
    await waitTimeseries

    // Wait until the UI flips from skeleton to hydrated render (pulse class removed).
    await expect(usageCard.locator('rect.animate-pulse')).toHaveCount(0)

    const usageAfter = await usageCard.boundingBox()
    expect(usageAfter).not.toBeNull()
    const usageAfterBox = usageAfter!

    test.info().annotations.push({
      type: 'dashboard-top-row-after',
      description: JSON.stringify({
        usage: { x: Math.round(usageAfterBox.x), y: Math.round(usageAfterBox.y), w: Math.round(usageAfterBox.width), h: Math.round(usageAfterBox.height) },
      }),
    })

    expect(Math.abs(usageAfterBox.x - usageBox.x)).toBeLessThanOrEqual(2)
  })

  test('renders an empty 90d calendar when timeseries returns no points', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })

    await page.route('**/api/stats/timeseries**', async (route) => {
      const requestUrl = new URL(route.request().url())
      const range = requestUrl.searchParams.get('range')
      const bucket = requestUrl.searchParams.get('bucket')
      if (range === '90d' && bucket === '1d') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            rangeStart: '2026-01-01T00:00:00Z',
            rangeEnd: '2026-04-01T00:00:00Z',
            bucketSeconds: 86400,
            points: [],
          }),
        })
        return
      }
      await route.continue()
    })

    await page.goto('/dashboard')

    const usageCard = page.getByTestId('usage-calendar-card')
    await usageCard.waitFor({ state: 'visible' })
    await expect(usageCard.locator('[data-testid="usage-calendar-wrapper"]')).toBeVisible()
    await expect(usageCard.locator('rect.animate-pulse')).toHaveCount(0)

    // Empty calendar should still render a full grid of blocks (no fallback to library loading skeleton).
    const blockCount = await usageCard.locator('svg rect').count()
    expect(blockCount).toBeGreaterThanOrEqual(60)
  })
})
