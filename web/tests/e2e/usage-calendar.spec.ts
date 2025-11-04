import { test, expect } from '@playwright/test'

const VIEWPORTS = [
  { width: 375, height: 900 },
  { width: 768, height: 900 },
  { width: 1024, height: 900 },
  { width: 1440, height: 900 },
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
})
