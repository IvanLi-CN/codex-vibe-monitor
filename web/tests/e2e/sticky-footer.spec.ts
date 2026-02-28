import { expect, test } from '@playwright/test'

test.describe('Sticky footer layout', () => {
  test('keeps footer pinned to viewport bottom when main content is short', async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.goto('/#/stats')

    const footer = page.locator('footer')
    await expect(footer).toBeVisible()

    // Force the "short content" scenario without depending on backend data volume.
    await page.addStyleTag({
      content: `
        main > * { display: none !important; }
        main { padding: 0 !important; }
      `,
    })

    await page.waitForTimeout(50)

    const metrics = await page.evaluate(() => {
      const node = document.querySelector('footer') as HTMLElement | null
      if (!node) return null
      const rect = node.getBoundingClientRect()
      return {
        footerBottom: rect.bottom,
        viewportHeight: window.innerHeight,
      }
    })

    expect(metrics).not.toBeNull()
    expect(Math.abs((metrics?.viewportHeight ?? 0) - (metrics?.footerBottom ?? 0))).toBeLessThanOrEqual(2)
  })
})

