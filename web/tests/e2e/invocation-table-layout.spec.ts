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
      requestedServiceTier: 'priority',
      serviceTier: 'priority',
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
      requestedServiceTier: 'priority',
      serviceTier: 'auto',
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
    {
      id: 9003,
      invokeId: 'inv_layout_9003',
      occurredAt: '2026-02-26T02:33:52Z',
      createdAt: '2026-02-26T02:33:52Z',
      source: 'proxy',
      proxyDisplayName: 'seoul-edge-02',
      endpoint: '/v1/responses',
      model: 'gpt-5.3-codex',
      status: 'success',
      requestedServiceTier: 'priority',
      inputTokens: 80312,
      outputTokens: 140,
      cacheInputTokens: 80000,
      totalTokens: 80452,
      cost: 0.0144,
      proxyWeightDelta: 0.12,
      tUpstreamTtfbMs: 118.4,
      tTotalMs: 6123.2,
    },
    {
      id: 9004,
      invokeId: 'inv_layout_9004',
      occurredAt: '2026-02-26T02:32:52Z',
      createdAt: '2026-02-26T02:32:52Z',
      source: 'proxy',
      proxyDisplayName: 'iad-relay-edge-04',
      endpoint: '/v1/responses',
      model: 'gpt-5.3-codex',
      status: 'success',
      requestedServiceTier: 'auto',
      serviceTier: 'priority',
      inputTokens: 72112,
      outputTokens: 154,
      cacheInputTokens: 71000,
      totalTokens: 72266,
      cost: 0.0121,
      proxyWeightDelta: 0.07,
      tUpstreamTtfbMs: 96.7,
      tTotalMs: 5402.4,
    },
    {
      id: 9005,
      invokeId: 'inv_layout_9005',
      occurredAt: '2026-02-26T02:31:52Z',
      createdAt: '2026-02-26T02:31:52Z',
      source: 'proxy',
      proxyDisplayName: 'la-relay-edge-05',
      endpoint: '/v1/responses',
      model: 'gpt-5.3-codex',
      status: 'success',
      requestedServiceTier: 'flex',
      serviceTier: 'flex',
      inputTokens: 62144,
      outputTokens: 132,
      cacheInputTokens: 60000,
      totalTokens: 62276,
      cost: 0.0106,
      proxyWeightDelta: 0,
      tUpstreamTtfbMs: 128.1,
      tTotalMs: 5108.9,
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
          successCount: 4,
          failureCount: 1,
          totalCost: 0.0838,
          totalTokens: 428762,
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

    if (pathname === '/api/stats/prompt-cache-conversations') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          rangeStart: '2026-03-02T00:00:00Z',
          rangeEnd: '2026-03-02T12:00:00Z',
          conversations: [],
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

          const items = page.getByTestId('invocation-list-item')
          await expect(mobileList.locator('[data-testid="invocation-fast-icon"][data-fast-state="effective"]')).toHaveCount(2)
          await expect(mobileList.locator('[data-testid="invocation-fast-icon"][data-fast-state="requested_only"]')).toHaveCount(2)
          await expect(items.nth(0).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'effective')
          await expect(items.nth(1).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'requested_only')
          await expect(items.nth(2).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'requested_only')
          await expect(items.nth(3).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'effective')
          await expect(items.nth(4).getByTestId('invocation-fast-icon')).toHaveCount(0)

          const listToggle = mobileList.locator('button[aria-expanded]').first()
          await expect(listToggle).toBeVisible()
          await listToggle.click()
          await expect(listToggle).toHaveAttribute('aria-expanded', 'true')
          const listDetailId = await listToggle.getAttribute('aria-controls')
          if (!listDetailId) throw new Error('Missing mobile invocation detail panel id')
          const listDetailPanel = page.locator(`#${listDetailId}`)
          await expect(listDetailPanel.getByText(/代理权重变化（本次）|Proxy weight delta \(this call\)/)).toBeVisible()
          await expect(listDetailPanel.getByText(/Requested service tier/i)).toBeVisible()
          await expect(listDetailPanel.getByText(/^Service tier$/i)).toBeVisible()
          await expect(listDetailPanel.getByText('priority')).toHaveCount(2)
          await expect(listDetailPanel.getByText('0.55')).toBeVisible()

          const viewportOverflow = await readViewportOverflow(page)
          test.info().annotations.push({
            type: 'invocation-mobile-layout',
            description: JSON.stringify({ target: target.label, viewport, viewportOverflow }),
          })
          expect(viewportOverflow).toBeLessThanOrEqual(1)
        } else {
          await expect(page.getByTestId('invocation-list')).toBeHidden()
          const tableScroll = page.getByTestId('invocation-table-scroll')
          const tableRows = tableScroll.locator('tbody tr')
          await expect(tableScroll.locator('[data-testid="invocation-fast-icon"][data-fast-state="effective"]')).toHaveCount(2)
          await expect(tableScroll.locator('[data-testid="invocation-fast-icon"][data-fast-state="requested_only"]')).toHaveCount(2)
          await expect(tableRows.nth(0).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'effective')
          await expect(tableRows.nth(1).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'requested_only')
          await expect(tableRows.nth(2).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'requested_only')
          await expect(tableRows.nth(3).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'effective')
          await expect(tableRows.nth(4).getByTestId('invocation-fast-icon')).toHaveCount(0)

          const metricsBeforeExpand = await readTableMetrics(page)
          const firstToggle = page.getByTestId('invocation-table-scroll').locator('tbody tr button').first()
          await expect(firstToggle).toBeVisible()
          await firstToggle.click()
          await expect(firstToggle).toHaveAttribute('aria-expanded', 'true')
          const tableDetailId = await firstToggle.getAttribute('aria-controls')
          if (!tableDetailId) throw new Error('Missing desktop invocation detail panel id')
          const tableDetailPanel = page.locator(`#${tableDetailId}`)
          await expect(tableDetailPanel.getByText(/代理权重变化（本次）|Proxy weight delta \(this call\)/)).toBeVisible()
          await expect(tableDetailPanel.getByText(/Requested service tier/i)).toBeVisible()
          await expect(tableDetailPanel.getByText(/^Service tier$/i)).toBeVisible()
          await expect(tableDetailPanel.getByText('priority')).toHaveCount(2)
          await expect(tableDetailPanel.getByText('0.55')).toBeVisible()
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
