import { test, expect } from '@playwright/test'

const VIEWPORTS = [
  { width: 375, height: 900 },
  { width: 768, height: 900 },
  { width: 1024, height: 900 },
  { width: 1440, height: 900 },
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
})
