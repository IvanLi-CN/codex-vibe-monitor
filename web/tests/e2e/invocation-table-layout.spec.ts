import { expect, test, type Page } from '@playwright/test'

const LONG_PROXY_NAME = 'ivan-hkl-vless-vision-01KFXRNYWYXKN4JHCF3CCV78GD'

const VIEWPORTS = [
  { width: 375, height: 900 },
  { width: 768, height: 900 },
  { width: 1024, height: 900 },
  { width: 1280, height: 900 },
  { width: 1440, height: 900 },
  { width: 1660, height: 900 },
  { width: 1873, height: 900 },
]

const TARGET_PAGES = [
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
      routeMode: 'pool',
      upstreamAccountId: 7,
      upstreamAccountName: 'Pool Alpha',
      proxyDisplayName: 'sg-relay-edge-01',
      responseContentEncoding: 'gzip, br',
      endpoint: '/v1/responses/compact',
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
      routeMode: 'forward_proxy',
      proxyDisplayName: LONG_PROXY_NAME,
      responseContentEncoding: 'identity',
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
      routeMode: 'pool',
      upstreamAccountId: 8,
      upstreamAccountName: 'Pool Beta',
      proxyDisplayName: 'seoul-edge-02',
      responseContentEncoding: 'br',
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
      routeMode: 'forward_proxy',
      proxyDisplayName: 'iad-relay-edge-04',
      responseContentEncoding: 'gzip',
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
      routeMode: 'forward_proxy',
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

const ACCOUNT_DETAIL_FIXTURE = {
  id: 7,
  kind: 'oauth_codex',
  provider: 'openai',
  displayName: 'Pool Alpha',
  groupName: 'team-a',
  isMother: true,
  status: 'active',
  enabled: true,
  email: 'pool-alpha@example.com',
  chatgptAccountId: 'org_pool_alpha',
  chatgptUserId: 'user_pool_alpha',
  planType: 'team',
  maskedApiKey: null,
  lastSyncedAt: '2026-03-16T09:10:00Z',
  lastSuccessfulSyncAt: '2026-03-16T09:08:00Z',
  lastError: null,
  lastErrorAt: null,
  tokenExpiresAt: '2026-03-16T12:00:00Z',
  lastRefreshedAt: '2026-03-16T09:09:00Z',
  primaryWindow: {
    usedPercent: 22,
    usedText: '22 / 100',
    limitText: '100 requests',
    resetsAt: '2026-03-16T10:00:00Z',
    windowDurationMins: 300,
  },
  secondaryWindow: {
    usedPercent: 36,
    usedText: '36 / 100',
    limitText: '100 requests',
    resetsAt: '2026-03-17T00:00:00Z',
    windowDurationMins: 10080,
  },
  credits: null,
  localLimits: null,
  duplicateInfo: null,
  tags: [],
  effectiveRoutingRule: {
    guardEnabled: false,
    lookbackHours: null,
    maxConversations: null,
    allowCutOut: true,
    allowCutIn: true,
    sourceTagIds: [],
    sourceTagNames: [],
    guardRules: [],
  },
  note: null,
  upstreamBaseUrl: null,
  history: [],
}

const POOL_ATTEMPTS_FIXTURE = [
  {
    id: 1,
    invokeId: 'inv_layout_9001',
    occurredAt: '2026-02-26T02:35:52Z',
    endpoint: '/v1/responses/compact',
    upstreamAccountId: 7,
    upstreamAccountName: 'Pool Alpha',
    attemptIndex: 1,
    distinctAccountIndex: 1,
    sameAccountRetryIndex: 1,
    startedAt: '2026-02-26T02:35:52Z',
    finishedAt: '2026-02-26T02:35:53Z',
    status: 'success',
    httpStatus: 200,
    connectLatencyMs: 42.3,
    firstByteLatencyMs: 15.2,
    streamLatencyMs: 188.4,
    upstreamRequestId: 'req_layout_pool_9001',
    createdAt: '2026-02-26T02:35:53Z',
  },
]

interface TableMetrics {
  clientWidth: number
  scrollWidth: number
  overflowDelta: number
  firstToggleHiddenRightPx: number
  firstRowTrailingGapPx: number
  secondRowProxyNameOverflowPx: number
  secondRowProxyBadgeVsModelLeftPx: number
  secondRowProxyNameTitle: string | null
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

    if (pathname === '/api/invocations/inv_layout_9001/pool-attempts') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(POOL_ATTEMPTS_FIXTURE),
      })
      return
    }

    if (pathname === '/api/pool/upstream-accounts/7') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(ACCOUNT_DETAIL_FIXTURE),
      })
      return
    }

    if (pathname === '/api/pool/upstream-accounts/7/sticky-keys') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          rangeStart: '2026-02-26T00:00:00Z',
          rangeEnd: '2026-02-26T12:00:00Z',
          selectionMode: 'count',
          selectedLimit: 20,
          selectedActivityHours: null,
          implicitFilter: { kind: null, filteredCount: 0 },
          conversations: [],
        }),
      })
      return
    }

    if (pathname === '/api/pool/upstream-accounts') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          writesEnabled: true,
          items: [],
          groups: [],
          hasUngroupedAccounts: false,
          total: 0,
          page: 1,
          pageSize: 20,
          metrics: {
            total: 0,
            oauth: 0,
            apiKey: 0,
            attention: 0,
          },
          routing: null,
        }),
      })
      return
    }

    if (pathname === '/api/pool/tags') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          items: [],
          totalAccounts: 0,
          guardsEnabledCount: 0,
        }),
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
    const dataRows = Array.from(container.querySelectorAll('tbody tr')).filter((row) => row.querySelector('button[aria-expanded]'))
    const firstDataRow = dataRows[0] as HTMLElement | undefined
    const secondDataRow = dataRows[1] as HTMLElement | undefined
    const firstToggle = firstDataRow?.querySelector('button') as HTMLElement | null
    const firstDataCells = firstDataRow ? Array.from(firstDataRow.querySelectorAll('td')) : []
    const lastDataCell = firstDataCells.length > 0 ? (firstDataCells[firstDataCells.length - 1] as HTMLElement) : null
    const secondProxyBadge = secondDataRow?.querySelector('[data-testid="invocation-proxy-badge"]') as HTMLElement | null
    const secondProxyName = secondDataRow?.querySelector('[data-testid="invocation-proxy-name"]') as HTMLElement | null
    const secondDataCells = secondDataRow ? Array.from(secondDataRow.querySelectorAll('td')) : []
    const secondModelCell = secondDataCells.length > 3 ? (secondDataCells[3] as HTMLElement) : null
    const containerRect = container.getBoundingClientRect()
    const toggleRect = firstToggle?.getBoundingClientRect()
    const lastDataCellRect = lastDataCell?.getBoundingClientRect()
    const secondProxyBadgeRect = secondProxyBadge?.getBoundingClientRect()
    const secondModelCellRect = secondModelCell?.getBoundingClientRect()

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
      secondRowProxyNameOverflowPx: secondProxyName
        ? secondProxyName.scrollWidth - secondProxyName.clientWidth
        : Number.NEGATIVE_INFINITY,
      secondRowProxyBadgeVsModelLeftPx: secondProxyBadgeRect && secondModelCellRect
        ? secondProxyBadgeRect.right - secondModelCellRect.left
        : Number.POSITIVE_INFINITY,
      secondRowProxyNameTitle: secondProxyName?.getAttribute('title') ?? null,
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
          await expect(mobileList.locator('[data-testid="invocation-endpoint-badge"]')).toHaveCount(4)
          await expect(items.nth(0).getByTestId('invocation-endpoint-badge')).toHaveAttribute('data-endpoint-kind', 'compact')
          await expect(items.nth(0).getByTestId('invocation-endpoint-badge')).toContainText(/远程压缩|Compact/)
          await expect(items.nth(1).getByTestId('invocation-endpoint-path')).toHaveAttribute('data-endpoint-kind', 'raw')
          await expect(items.nth(1).getByTestId('invocation-endpoint-path')).toContainText('/v1/responses/very-long-segment-')
          await expect(items.nth(2).getByTestId('invocation-endpoint-badge')).toHaveAttribute('data-endpoint-kind', 'responses')
          await expect(items.nth(3).getByTestId('invocation-endpoint-badge')).toHaveAttribute('data-endpoint-kind', 'responses')
          await expect(items.nth(0).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'effective')
          await expect(items.nth(1).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'requested_only')
          await expect(items.nth(2).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'requested_only')
          await expect(items.nth(3).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'effective')
          await expect(items.nth(4).getByTestId('invocation-fast-icon')).toHaveCount(0)
          await expect(items.nth(0).getByTestId('invocation-account-name')).toContainText('Pool Alpha')
          await expect(items.nth(0)).toContainText('HTTP gzip, br')
          await expect(items.nth(1).getByTestId('invocation-account-name')).toContainText(/反向代理|Reverse proxy/)

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
          await expect(listDetailPanel.getByText('/v1/responses/compact')).toBeVisible()
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
          await expect(tableScroll.locator('[data-testid="invocation-endpoint-badge"]:visible')).toHaveCount(
            viewport.width >= 1280 ? 4 : 0,
          )
          await expect(tableRows.nth(0).getByTestId('invocation-account-name')).toContainText('Pool Alpha')
          await expect(tableRows.nth(1).getByTestId('invocation-account-name')).toContainText(/反向代理|Reverse proxy/)
          if (viewport.width >= 1280) {
            const compactEndpointBadge = tableRows.nth(0).locator('[data-testid="invocation-endpoint-badge"]:visible')
            await expect(compactEndpointBadge).toHaveCount(1)
            await expect(compactEndpointBadge).toHaveAttribute('data-endpoint-kind', 'compact')
            await expect(compactEndpointBadge).toContainText(/远程压缩|Compact/)
            const rawEndpointPath = tableRows.nth(1).locator('[data-testid="invocation-endpoint-path"]:visible')
            await expect(rawEndpointPath).toHaveCount(1)
            await expect(rawEndpointPath).toHaveAttribute('data-endpoint-kind', 'raw')
            await expect(rawEndpointPath).toContainText('/v1/responses/very-long-segment-')
          }
          await expect(tableRows.nth(0).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'effective')
          await expect(tableRows.nth(1).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'requested_only')
          await expect(tableRows.nth(2).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'requested_only')
          await expect(tableRows.nth(3).getByTestId('invocation-fast-icon')).toHaveAttribute('data-fast-state', 'effective')
          await expect(tableRows.nth(4).getByTestId('invocation-fast-icon')).toHaveCount(0)

          const metricsBeforeExpand = await readTableMetrics(page)
          const firstToggle = tableRows.nth(0).locator('button[aria-expanded]')
          await expect(firstToggle).toBeVisible()
          await firstToggle.click()
          await expect(firstToggle).toHaveAttribute('aria-expanded', 'true')
          const tableDetailId = await firstToggle.getAttribute('aria-controls')
          if (!tableDetailId) throw new Error('Missing desktop invocation detail panel id')
          const tableDetailPanel = page.locator(`#${tableDetailId}`)
          await expect(tableDetailPanel.getByText(/代理权重变化（本次）|Proxy weight delta \(this call\)/)).toBeVisible()
          await expect(tableDetailPanel.getByText(/Requested service tier/i)).toBeVisible()
          await expect(tableDetailPanel.getByText(/^Service tier$/i)).toBeVisible()
          await expect(tableDetailPanel.getByText('/v1/responses/compact')).toBeVisible()
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
          if (viewport.width >= 1280) {
            expect(metricsBeforeExpand.secondRowProxyNameTitle).toBe(LONG_PROXY_NAME)
            expect(metricsAfterExpand.secondRowProxyNameTitle).toBe(LONG_PROXY_NAME)
            expect(metricsBeforeExpand.secondRowProxyNameOverflowPx).toBeGreaterThan(0)
            expect(metricsAfterExpand.secondRowProxyNameOverflowPx).toBeGreaterThan(0)
            expect(metricsBeforeExpand.secondRowProxyBadgeVsModelLeftPx).toBeLessThanOrEqual(1)
            expect(metricsAfterExpand.secondRowProxyBadgeVsModelLeftPx).toBeLessThanOrEqual(1)
          }
          if (viewport.width === 1280 && target.label === 'live') {
            await tableRows.nth(0).getByRole('button', { name: 'Pool Alpha' }).click()
            const drawer = page.getByRole('dialog')
            await expect(drawer).toBeVisible()
            await expect(drawer.getByText('Pool Alpha')).toBeVisible()
            await expect(drawer.getByText('22 / 100')).toBeVisible()
            await drawer.getByRole('button', { name: /关闭|Close/ }).click()
            await expect(drawer).toBeHidden()
          }
        }
      })
    }
  }
})
