import { expect, test, type Page } from '@playwright/test'

type RouteCase = {
  path: string
  waitFor: (page: Page) => Promise<void>
}

const routes: RouteCase[] = [
  {
    path: '/#/account-pool/upstream-accounts',
    waitFor: async (page) => {
      await expect(page.getByTestId('upstream-accounts-roster-region')).toBeVisible()
    },
  },
  {
    path: '/#/account-pool/upstream-accounts/new?mode=oauth',
    waitFor: async (page) => {
      await expect(page.locator('input[name="oauthMailboxInput"]')).toBeVisible()
    },
  },
  {
    path: '/#/account-pool/upstream-accounts/new?mode=batchOauth',
    waitFor: async (page) => {
      await expect(page.locator('[data-testid^="batch-oauth-row-"]').first()).toBeVisible()
    },
  },
  {
    path: '/#/account-pool/upstream-accounts/new?mode=import',
    waitFor: async (page) => {
      await expect(page.locator('input[name="importOauthFiles"]')).toBeVisible()
    },
  },
  {
    path: '/#/account-pool/upstream-accounts/new?mode=apiKey',
    waitFor: async (page) => {
      await expect(page.locator('input[name="apiKeyValue"]')).toBeVisible()
    },
  },
]

test.describe('Account pool create flow smoke', () => {
  for (const route of routes) {
    test(`loads ${route.path}`, async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 960 })
      await page.goto(route.path)
      await route.waitFor(page)
      await expect(page.locator('h2.section-title').first()).toBeVisible()
    })
  }
})
