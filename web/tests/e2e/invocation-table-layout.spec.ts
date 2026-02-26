import { expect, test, type Page } from '@playwright/test'

const VIEWPORTS = [
  { width: 1280, height: 900 },
  { width: 1440, height: 900 },
  { width: 1873, height: 900 },
]

const TARGET_PAGES = [
  { path: '/#/dashboard', label: 'dashboard', hashPath: '#/dashboard' },
  { path: '/#/live', label: 'live', hashPath: '#/live' },
]

const INVOCATION_FIXTURE = {
  records: [
    {
      id: 9001,
      invokeId: 'inv_layout_9001',
      occurredAt: '2026-02-26T02:35:52Z',
      createdAt: '2026-02-26T02:35:52Z',
      source: 'proxy',
      endpoint: '/v1/responses',
      model: 'gpt-5.3-codex',
      status: 'success',
      inputTokens: 113273,
      outputTokens: 176,
      cacheInputTokens: 109568,
      totalTokens: 113449,
      cost: 0.0281,
      tUpstreamTtfbMs: 105.5,
      tTotalMs: 7969.3,
    },
    {
      id: 9002,
      invokeId: 'inv_layout_9002',
      occurredAt: '2026-02-26T02:34:52Z',
      createdAt: '2026-02-26T02:34:52Z',
      source: 'proxy',
      endpoint: '/v1/responses',
      model: 'gpt-5.3-codex',
      status: 'success',
      inputTokens: 95250,
      outputTokens: 69,
      cacheInputTokens: 99072,
      totalTokens: 99319,
      cost: 0.0186,
      tUpstreamTtfbMs: 102.2,
      tTotalMs: 7348.7,
    },
  ],
}

interface TableMetrics {
  clientWidth: number
  scrollWidth: number
  overflowDelta: number
  firstToggleHiddenRightPx: number
}

async function mockInvocations(page: Page) {
  await page.route('**/api/invocations?**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(INVOCATION_FIXTURE),
    })
  })
}

async function readTableMetrics(page: Page): Promise<TableMetrics> {
  const tableScroll = page.getByTestId('invocation-table-scroll')
  await expect(tableScroll).toBeVisible()
  const firstRow = tableScroll.locator('tbody tr').first()
  await expect(firstRow).toBeVisible()

  return tableScroll.evaluate((node) => {
    const container = node as HTMLElement
    const firstToggle = container.querySelector('tbody tr button') as HTMLElement | null
    const containerRect = container.getBoundingClientRect()
    const toggleRect = firstToggle?.getBoundingClientRect()

    return {
      clientWidth: container.clientWidth,
      scrollWidth: container.scrollWidth,
      overflowDelta: container.scrollWidth - container.clientWidth,
      firstToggleHiddenRightPx: toggleRect
        ? Math.max(0, toggleRect.right - containerRect.right)
        : Number.POSITIVE_INFINITY,
    }
  })
}

test.describe('InvocationTable layout regression', () => {
  for (const viewport of VIEWPORTS) {
    for (const target of TARGET_PAGES) {
      test(`keeps expand toggle visible without artificial horizontal overflow at ${target.label} ${viewport.width}px`, async ({
        page,
      }) => {
        await page.setViewportSize(viewport)
        await mockInvocations(page)
        await page.goto(target.path)
        await expect(page).toHaveURL(new RegExp(`${target.hashPath}$`))

        const metrics = await readTableMetrics(page)
        test.info().annotations.push({
          type: 'invocation-table-layout',
          description: JSON.stringify({ target: target.label, viewport, metrics }),
        })

        expect(metrics.overflowDelta).toBeLessThanOrEqual(2)
        expect(metrics.firstToggleHiddenRightPx).toBeLessThanOrEqual(0)
      })
    }
  }
})
