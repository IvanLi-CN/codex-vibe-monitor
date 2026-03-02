import { expect, test, type Page } from '@playwright/test'

const VIEWPORTS = [
  { width: 375, height: 900 },
  { width: 768, height: 900 },
  { width: 1024, height: 900 },
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
      proxyDisplayName: 'sg-relay-edge-01',
      endpoint: '/v1/responses',
      model: 'gpt-5.3-codex',
      status: 'success',
      inputTokens: 113273,
      outputTokens: 176,
      cacheInputTokens: 109568,
      totalTokens: 113449,
      cost: 0.0281,
      proxyWeightDelta: 0.55,
      tUpstreamTtfbMs: 105.5,
      tTotalMs: 7969.3,
    },
    {
      id: 9002,
      invokeId: 'inv_layout_9002',
      occurredAt: '2026-02-26T02:34:52Z',
      createdAt: '2026-02-26T02:34:52Z',
      source: 'proxy',
      proxyDisplayName: 'tokyo-super-long-relay-name-for-overflow-regression-verify',
      endpoint: '/v1/responses/' + 'very-long-segment-'.repeat(12),
      model: 'gpt-5.3-codex',
      status: 'failed',
      inputTokens: 95250,
      outputTokens: 69,
      cacheInputTokens: 99072,
      totalTokens: 99319,
      cost: 0.0186,
      proxyWeightDelta: -0.68,
      tUpstreamTtfbMs: 102.2,
      tTotalMs: 7348.7,
      errorMessage:
        '[downstream_closed] ' +
        'x'.repeat(260),
    },
  ],
}

interface TableMetrics {
  clientWidth: number
  scrollWidth: number
  overflowDelta: number
  firstToggleHiddenRightPx: number
  firstRowTrailingGapPx: number
}

async function mockInvocations(page: Page) {
  await page.route('**/events', async (route) => {
    await route.fulfill({
      status: 204,
      headers: {
        'content-type': 'text/event-stream',
        'cache-control': 'no-cache',
      },
      body: '',
    })
  })

  await page.route('**/api/**', async (route) => {
    const requestUrl = new URL(route.request().url())
    const pathname = requestUrl.pathname

    if (pathname === '/api/invocations') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(INVOCATION_FIXTURE),
      })
      return
    }

    if (pathname === '/api/stats' || pathname === '/api/stats/summary') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          totalCount: INVOCATION_FIXTURE.records.length,
          successCount: 1,
          failureCount: 1,
          totalCost: 0.0467,
          totalTokens: 212768,
        }),
      })
      return
    }

    if (pathname === '/api/stats/timeseries') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          rangeStart: '2026-03-02T00:00:00Z',
          rangeEnd: '2026-03-02T12:00:00Z',
          bucketSeconds: 600,
          points: [],
        }),
      })
      return
    }

    if (pathname === '/api/stats/errors') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          rangeStart: '2026-03-02T00:00:00Z',
          rangeEnd: '2026-03-02T12:00:00Z',
          items: [],
        }),
      })
      return
    }

    if (pathname === '/api/stats/failures/summary') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          rangeStart: '2026-03-02T00:00:00Z',
          rangeEnd: '2026-03-02T12:00:00Z',
          totalFailures: 1,
          serviceFailureCount: 1,
          clientFailureCount: 0,
          clientAbortCount: 0,
          actionableFailureCount: 1,
          actionableFailureRate: 1,
        }),
      })
      return
    }

    if (pathname === '/api/stats/perf') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          rangeStart: '2026-03-02T00:00:00Z',
          rangeEnd: '2026-03-02T12:00:00Z',
          items: [],
        }),
      })
      return
    }

    if (pathname === '/api/stats/forward-proxy') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          rangeStart: '2026-03-02T00:00:00Z',
          rangeEnd: '2026-03-02T12:00:00Z',
          items: [],
        }),
      })
      return
    }

    if (pathname === '/api/version') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ backend: '0.2.0', frontend: '0.2.0' }),
      })
      return
    }

    throw new Error(`Unexpected API request in invocation-table-layout spec: ${pathname}`)
  })
}

async function readTableMetrics(page: Page): Promise<TableMetrics> {
  const tableScroll = page.getByTestId('invocation-table-scroll')
  await expect(tableScroll).toBeVisible()
  const firstRow = tableScroll.locator('tbody tr').first()
  await expect(firstRow).toBeVisible()

  return tableScroll.evaluate((node) => {
    const container = node as HTMLElement
    const firstDataRow = container.querySelector('tbody tr')
    const firstToggle = firstDataRow?.querySelector('button') as HTMLElement | null
    const firstDataCells = firstDataRow ? Array.from(firstDataRow.querySelectorAll('td')) : []
    const lastDataCell = firstDataCells.length > 0 ? (firstDataCells[firstDataCells.length - 1] as HTMLElement) : null
    const containerRect = container.getBoundingClientRect()
    const toggleRect = firstToggle?.getBoundingClientRect()
    const lastDataCellRect = lastDataCell?.getBoundingClientRect()

    return {
      clientWidth: container.clientWidth,
      scrollWidth: container.scrollWidth,
      overflowDelta: container.scrollWidth - container.clientWidth,
      firstToggleHiddenRightPx: toggleRect
        ? Math.max(0, toggleRect.right - containerRect.right)
        : Number.POSITIVE_INFINITY,
      firstRowTrailingGapPx: lastDataCellRect
        ? Math.max(0, containerRect.right - lastDataCellRect.right)
        : Number.POSITIVE_INFINITY,
    }
  })
}

async function readViewportOverflow(page: Page): Promise<number> {
  return page.evaluate(() => {
    const root = document.documentElement
    return root.scrollWidth - root.clientWidth
  })
}

test.describe('InvocationTable layout regression', () => {
  for (const viewport of VIEWPORTS) {
    for (const target of TARGET_PAGES) {
      test(`keeps responsive layout stable at ${target.label} ${viewport.width}px`, async ({
        page,
      }) => {
        await page.setViewportSize(viewport)
        await mockInvocations(page)
        await page.goto(target.path)
        await expect(page).toHaveURL(new RegExp(`${target.hashPath}$`))

        if (viewport.width < 768) {
          const mobileList = page.getByTestId('invocation-list')
          await expect(mobileList).toBeVisible()
          await expect(page.getByTestId('invocation-list-item')).toHaveCount(INVOCATION_FIXTURE.records.length)
          await expect(page.getByTestId('invocation-table-scroll')).toBeHidden()

          const listToggle = mobileList.locator('button[aria-expanded]').first()
          await expect(listToggle).toBeVisible()
          await listToggle.click()
          await expect(listToggle).toHaveAttribute('aria-expanded', 'true')
          await expect(page.getByText(/代理权重变化（本次）|Proxy weight delta \(this call\)/)).toBeVisible()
          await expect(page.getByText(/↑\s\+0\.55/)).toBeVisible()

          const viewportOverflow = await readViewportOverflow(page)
          test.info().annotations.push({
            type: 'invocation-mobile-layout',
            description: JSON.stringify({ target: target.label, viewport, viewportOverflow }),
          })
          expect(viewportOverflow).toBeLessThanOrEqual(1)
        } else {
          await expect(page.getByTestId('invocation-list')).toBeHidden()
          const metricsBeforeExpand = await readTableMetrics(page)
          const firstToggle = page.getByTestId('invocation-table-scroll').locator('tbody tr button').first()
          await firstToggle.click()
          await expect(firstToggle).toHaveAttribute('aria-expanded', 'true')
          await expect(page.getByText(/代理权重变化（本次）|Proxy weight delta \(this call\)/)).toBeVisible()
          await expect(page.getByText(/↑\s\+0\.55/)).toBeVisible()
          const metricsAfterExpand = await readTableMetrics(page)

          test.info().annotations.push({
            type: 'invocation-table-layout',
            description: JSON.stringify({
              target: target.label,
              viewport,
              metricsBeforeExpand,
              metricsAfterExpand,
            }),
          })
          expect(metricsBeforeExpand.overflowDelta).toBeLessThanOrEqual(1)
          expect(metricsAfterExpand.overflowDelta).toBeLessThanOrEqual(1)
          expect(metricsBeforeExpand.firstToggleHiddenRightPx).toBeLessThanOrEqual(0)
          expect(metricsAfterExpand.firstToggleHiddenRightPx).toBeLessThanOrEqual(0)
          expect(metricsBeforeExpand.firstRowTrailingGapPx).toBeLessThanOrEqual(1)
          expect(metricsAfterExpand.firstRowTrailingGapPx).toBeLessThanOrEqual(1)
        }
      })
    }
  }
})
