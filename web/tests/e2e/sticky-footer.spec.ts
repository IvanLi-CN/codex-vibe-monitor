import { expect, test } from '@playwright/test'

test.describe('Sticky footer layout', () => {
  test('keeps footer pinned to viewport bottom when main content is short', async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.goto('/#/stats')

    const footer = page.getByTestId('app-footer')
    await expect(footer).toBeAttached()

    // Force the "short content" scenario without depending on backend data volume.
    await page.addStyleTag({
      content: `
        .app-shell > main > * { display: none !important; }
        .app-shell > main { padding: 0 !important; }
      `,
    })

    await expect(footer).toBeVisible()

    await expect
      .poll(async () => {
        return footer.evaluate((node) => {
          const rect = (node as HTMLElement).getBoundingClientRect()
          return Math.abs(window.innerHeight - rect.bottom)
        })
      })
      .toBeLessThanOrEqual(2)
  })

  test('keeps footer off-viewport on long pages until scrolled to bottom', async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.goto('/#/stats')

    const footer = page.getByTestId('app-footer')
    await expect(footer).toBeAttached()

    // Make the page tall using CSS (avoids mutating the React-managed DOM).
    await page.addStyleTag({
      content: `
        .app-shell > main::after { content: ''; display: block; height: 200vh; }
      `,
    })

    await expect(footer).not.toBeInViewport()

    await page.evaluate(() => {
      const scrollingElement = document.scrollingElement ?? document.documentElement
      window.scrollTo(0, scrollingElement.scrollHeight)
    })

    await expect(footer).toBeInViewport()
    await expect
      .poll(async () => {
        return footer.evaluate((node) => {
          const rect = (node as HTMLElement).getBoundingClientRect()
          return Math.abs(window.innerHeight - rect.bottom)
        })
      })
      .toBeLessThanOrEqual(2)
  })
})
