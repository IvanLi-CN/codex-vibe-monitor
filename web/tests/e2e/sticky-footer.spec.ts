import { expect, test } from '@playwright/test'

test.describe('Sticky footer layout', () => {
  test('keeps footer pinned to viewport bottom when main content is short', async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.goto('/#/stats')

    const footer = page.locator('.app-shell > footer')
    await expect(footer).toBeAttached()

    // Force the "short content" scenario without depending on backend data volume.
    await page.addStyleTag({
      content: `
        main > * { display: none !important; }
        main { padding: 0 !important; }
      `,
    })

    await expect(footer).toBeVisible()

    await expect
      .poll(async () => {
        return page.evaluate(() => {
          const node = document.querySelector('.app-shell > footer') as HTMLElement | null
          if (!node) return Number.POSITIVE_INFINITY
          const rect = node.getBoundingClientRect()
          return Math.abs(window.innerHeight - rect.bottom)
        })
      })
      .toBeLessThanOrEqual(2)
  })

  test('keeps footer off-viewport on long pages until scrolled to bottom', async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.goto('/#/stats')

    const footer = page.locator('.app-shell > footer')
    await expect(footer).toBeAttached()

    // Make the page tall using CSS (avoids mutating the React-managed DOM).
    await page.addStyleTag({
      content: `
        main::after { content: ''; display: block; height: 200vh; }
      `,
    })

    await expect(footer).not.toBeInViewport()

    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight))

    await expect(footer).toBeInViewport()
    await expect
      .poll(async () => {
        return page.evaluate(() => {
          const node = document.querySelector('.app-shell > footer') as HTMLElement | null
          if (!node) return Number.POSITIVE_INFINITY
          const rect = node.getBoundingClientRect()
          return Math.abs(window.innerHeight - rect.bottom)
        })
      })
      .toBeLessThanOrEqual(2)
  })
})
