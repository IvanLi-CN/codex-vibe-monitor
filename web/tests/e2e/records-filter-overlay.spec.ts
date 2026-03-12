import { expect, test, type Page } from '@playwright/test'

const RECORDS_FIXTURE = {
  snapshotId: 901,
  total: 2,
  page: 1,
  pageSize: 20,
  records: [
    {
      id: 9001,
      invokeId: 'inv_records_overlay_9001',
      occurredAt: '2026-03-12T03:35:52Z',
      createdAt: '2026-03-12T03:35:52Z',
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
      tUpstreamTtfbMs: 105.5,
      tTotalMs: 7969.3,
    },
    {
      id: 9002,
      invokeId: 'inv_records_overlay_9002',
      occurredAt: '2026-03-12T03:34:52Z',
      createdAt: '2026-03-12T03:34:52Z',
      source: 'proxy',
      proxyDisplayName: 'iad-relay-edge-02',
      endpoint: '/v1/responses/compact',
      model: 'gpt-5.3-codex',
      status: 'failed',
      requestedServiceTier: 'priority',
      serviceTier: 'auto',
      inputTokens: 95250,
      outputTokens: 69,
      cacheInputTokens: 99072,
      totalTokens: 99319,
      cost: 0.0186,
      tUpstreamTtfbMs: 102.2,
      tTotalMs: 7348.7,
      errorMessage: 'request aborted by downstream client',
    },
  ],
}

const SUMMARY_FIXTURE = {
  snapshotId: RECORDS_FIXTURE.snapshotId,
  newRecordsCount: 0,
  totalCount: RECORDS_FIXTURE.records.length,
  successCount: 1,
  failureCount: 1,
  totalCost: 0.0467,
  totalTokens: 212768,
  token: {
    requestCount: RECORDS_FIXTURE.records.length,
    totalTokens: 212768,
    avgTokensPerRequest: 106384,
    cacheInputTokens: 208640,
    totalCost: 0.0467,
  },
  network: {
    avgTtfbMs: 103.85,
    p95TtfbMs: 105.5,
    avgTotalMs: 7658.8,
    p95TotalMs: 7969.3,
  },
  exception: {
    failureCount: 1,
    serviceFailureCount: 1,
    clientFailureCount: 0,
    clientAbortCount: 0,
    actionableFailureCount: 1,
  },
}

const SUGGESTIONS_FIXTURE = {
  model: { items: [], hasMore: false },
  proxy: { items: [], hasMore: false },
  endpoint: { items: [], hasMore: false },
  failureKind: { items: [], hasMore: false },
  promptCacheKey: {
    items: Array.from({ length: 8 }, (_, idx) => ({
      value: `019cdbc${idx}-0fd5-7cd0-a74d-79f0a4d${idx}`,
      count: 8 - idx,
    })),
    hasMore: false,
  },
  requesterIp: { items: [], hasMore: false },
}

async function mockRecordsPageApis(page: Page) {
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
    const { pathname } = requestUrl

    if (pathname === '/api/invocations') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(RECORDS_FIXTURE),
      })
      return
    }

    if (pathname === '/api/invocations/summary') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(SUMMARY_FIXTURE),
      })
      return
    }

    if (pathname === '/api/invocations/new-count') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          snapshotId: RECORDS_FIXTURE.snapshotId,
          newRecordsCount: 0,
        }),
      })
      return
    }

    if (pathname === '/api/invocations/suggestions') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(SUGGESTIONS_FIXTURE),
      })
      return
    }

    if (pathname === '/api/version') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          backend: '1.2.0',
          frontend: '0.2.0',
        }),
      })
      return
    }

    throw new Error(`unmocked API route in records overlay test: ${pathname}`)
  })
}

test.describe('Records filter overlay', () => {
  test('keeps the prompt cache key dropdown above the summary panel', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 960 })
    await mockRecordsPageApis(page)
    await page.goto('/#/records')

    const filtersPanel = page.getByTestId('records-filters-panel')
    const summaryPanel = page.getByTestId('records-summary-panel')
    const promptCacheKeyInput = page.locator('#records-filter-prompt-cache-key')

    await expect(filtersPanel).toBeVisible()
    await expect(summaryPanel).toBeVisible()
    await expect(promptCacheKeyInput).toBeVisible()

    // Force an overlap scenario so the regression stays stable across copy/layout tweaks.
    await page.addStyleTag({
      content: `
        [data-testid="records-summary-panel"] {
          margin-top: -7rem !important;
        }
      `,
    })

    await promptCacheKeyInput.click()

    const listbox = filtersPanel.locator('[role="listbox"]')
    await expect(listbox).toBeVisible()
    await expect(filtersPanel).toHaveAttribute('data-suggestions-open', 'true')

    const overlayState = await page.evaluate(() => {
      const panel = document.querySelector('[data-testid="records-filters-panel"]') as HTMLElement | null
      const summary = document.querySelector('[data-testid="records-summary-panel"]') as HTMLElement | null
      const listboxNode = panel?.querySelector('[role="listbox"]') as HTMLElement | null
      if (!panel || !summary || !listboxNode) {
        return null
      }

      const panelStyles = window.getComputedStyle(panel)
      const listRect = listboxNode.getBoundingClientRect()
      const summaryRect = summary.getBoundingClientRect()
      const overlapY = Math.min(listRect.bottom, summaryRect.bottom) - Math.max(listRect.top, summaryRect.top)
      const probeX = Math.min(listRect.right - 12, Math.max(listRect.left + 12, listRect.left + listRect.width * 0.25))
      const probeY = summaryRect.top + Math.max(12, Math.min(overlapY / 2, 40))
      const topNode = document.elementFromPoint(probeX, probeY) as HTMLElement | null

      return {
        panelZIndex: panelStyles.zIndex,
        listBottom: listRect.bottom,
        summaryTop: summaryRect.top,
        overlapY,
        topNodeInsideListbox: !!topNode && listboxNode.contains(topNode),
      }
    })

    expect(overlayState).not.toBeNull()
    expect(overlayState?.panelZIndex).toBe('10')
    expect(overlayState?.listBottom).toBeGreaterThan(overlayState?.summaryTop ?? 0)
    expect(overlayState?.overlapY).toBeGreaterThan(12)
    expect(overlayState?.topNodeInsideListbox).toBe(true)
  })
})
