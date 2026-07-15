import { expect, type Page, test } from "@playwright/test";

type PromptCacheConversationInvocationPreview = {
  id: number;
  invokeId: string;
  occurredAt: string;
  status: string;
  failureClass?: string;
  routeMode?: string;
  model?: string;
  totalTokens?: number;
  cost?: number;
  proxyDisplayName?: string;
  upstreamAccountId?: number;
  upstreamAccountName?: string;
  endpoint?: string;
  source?: string;
  inputTokens?: number;
  outputTokens?: number;
  cacheInputTokens?: number;
  reasoningTokens?: number;
  reasoningEffort?: string;
  errorMessage?: string;
  failureKind?: string;
  isActionable?: boolean;
  responseContentEncoding?: string;
  requestedServiceTier?: string;
  serviceTier?: string;
  tReqReadMs?: number | null;
  tReqParseMs?: number | null;
  tUpstreamConnectMs?: number | null;
  tUpstreamTtfbMs?: number | null;
  tUpstreamStreamMs?: number | null;
  tRespParseMs?: number | null;
  tPersistMs?: number | null;
  tTotalMs?: number | null;
};

function createPreview(
  overrides: Partial<PromptCacheConversationInvocationPreview> & {
    id: number;
    invokeId: string;
    occurredAt: string;
    status: string;
  },
): PromptCacheConversationInvocationPreview {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    occurredAt: overrides.occurredAt,
    status: overrides.status,
    failureClass: overrides.failureClass ?? "none",
    routeMode: overrides.routeMode ?? "pool",
    model: overrides.model ?? "gpt-5.4",
    totalTokens: overrides.totalTokens ?? 280,
    cost: overrides.cost ?? 0.0178,
    proxyDisplayName: overrides.proxyDisplayName ?? "tokyo-edge-01",
    upstreamAccountId: overrides.upstreamAccountId ?? 42,
    upstreamAccountName: overrides.upstreamAccountName ?? "pool-alpha@example.com",
    endpoint: overrides.endpoint ?? "/v1/responses",
    source: overrides.source ?? "pool",
    inputTokens: overrides.inputTokens ?? 164,
    outputTokens: overrides.outputTokens ?? 116,
    cacheInputTokens: overrides.cacheInputTokens ?? 42,
    reasoningTokens: overrides.reasoningTokens ?? 18,
    reasoningEffort: overrides.reasoningEffort ?? "high",
    errorMessage: overrides.errorMessage,
    failureKind: overrides.failureKind,
    isActionable: overrides.isActionable,
    responseContentEncoding: overrides.responseContentEncoding ?? "gzip",
    requestedServiceTier: overrides.requestedServiceTier ?? "priority",
    serviceTier: overrides.serviceTier ?? "priority",
    tReqReadMs: overrides.tReqReadMs ?? 11,
    tReqParseMs: overrides.tReqParseMs ?? 7,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 84,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 91,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs ?? 220,
    tRespParseMs: overrides.tRespParseMs ?? 10,
    tPersistMs: overrides.tPersistMs ?? 8,
    tTotalMs: overrides.tTotalMs ?? 431,
  };
}

function createConversation(
  promptCacheKey: string,
  recentInvocations: PromptCacheConversationInvocationPreview[],
) {
  const lastInFlightPreview = recentInvocations.find(
    (invocation) => invocation.status === "running" || invocation.status === "pending",
  );
  const lastTerminalPreview = recentInvocations.find(
    (invocation) => invocation.status !== "running" && invocation.status !== "pending",
  );
  return {
    promptCacheKey,
    requestCount: recentInvocations.length,
    totalTokens: recentInvocations.reduce(
      (sum, invocation) => sum + (invocation.totalTokens ?? 0),
      0,
    ),
    totalCost: Number(
      recentInvocations.reduce((sum, invocation) => sum + (invocation.cost ?? 0), 0).toFixed(4),
    ),
    createdAt: recentInvocations.at(-1)?.occurredAt ?? "2026-04-06T12:00:00.000Z",
    lastActivityAt: recentInvocations[0]?.occurredAt ?? "2026-04-06T12:00:00.000Z",
    lastTerminalAt: lastTerminalPreview?.occurredAt ?? null,
    lastInFlightAt: lastInFlightPreview?.occurredAt ?? null,
    cursor: promptCacheKey,
    upstreamAccounts: [],
    recentInvocations,
    last24hRequests: recentInvocations.map((invocation, index) => ({
      occurredAt: invocation.occurredAt,
      status: invocation.status,
      isSuccess: invocation.status === "completed" || invocation.status === "success",
      requestTokens: 180 + index * 24,
      cumulativeTokens: 180 + index * 24,
    })),
  };
}

function buildWorkingConversationsResponse() {
  const conversations = [
    createConversation("wc-current-1", [
      createPreview({
        id: 1,
        invokeId: "wc-1-a",
        occurredAt: "2026-04-06T12:00:00.000Z",
        status: "running",
        upstreamAccountName: "paisleeeinar5710 Team sandbox workflow monitor",
        endpoint: "/v1/responses/compact",
        tTotalMs: null,
      }),
      createPreview({
        id: 2,
        invokeId: "wc-1-b",
        occurredAt: "2026-04-06T11:57:20.000Z",
        status: "success",
        model: "gpt-5.4-mini",
        upstreamAccountName: "paisleeeinar5710 Team sandbox workflow monitor",
        endpoint: "/v1/responses/compact",
      }),
    ]),
    createConversation("wc-current-2", [
      createPreview({
        id: 3,
        invokeId: "wc-2-a",
        occurredAt: "2026-04-06T11:59:10.000Z",
        status: "http_502",
        failureClass: "service_failure",
        failureKind: "upstream_timeout",
        errorMessage: "upstream gateway closed before first byte",
        upstreamAccountName: "pool-beta@example.com",
        requestedServiceTier: "auto",
        serviceTier: "auto",
      }),
      createPreview({
        id: 4,
        invokeId: "wc-2-b",
        occurredAt: "2026-04-06T11:56:10.000Z",
        status: "success",
        upstreamAccountName: "pool-beta@example.com",
      }),
    ]),
    createConversation("wc-current-3", [
      createPreview({
        id: 5,
        invokeId: "wc-3-a",
        occurredAt: "2026-04-06T11:58:40.000Z",
        status: "completed",
        upstreamAccountName: "pool-gamma@example.com",
        totalTokens: 364,
        cost: 0.0204,
        inputTokens: 218,
        outputTokens: 146,
        cacheInputTokens: 68,
      }),
    ]),
    createConversation("wc-current-4", [
      createPreview({
        id: 6,
        invokeId: "wc-4-a",
        occurredAt: "2026-04-06T11:58:02.000Z",
        status: "pending",
        upstreamAccountName: "pool-delta@example.com",
        tTotalMs: null,
      }),
      createPreview({
        id: 7,
        invokeId: "wc-4-b",
        occurredAt: "2026-04-06T11:54:32.000Z",
        status: "success",
        upstreamAccountName: "pool-delta@example.com",
      }),
    ]),
    createConversation("wc-current-5", [
      createPreview({
        id: 8,
        invokeId: "wc-5-a",
        occurredAt: "2026-04-06T11:57:24.000Z",
        status: "completed",
        upstreamAccountName: "pool-epsilon@example.com",
        totalTokens: 412,
        cost: 0.0236,
        inputTokens: 244,
        outputTokens: 168,
        cacheInputTokens: 72,
      }),
      createPreview({
        id: 9,
        invokeId: "wc-5-b",
        occurredAt: "2026-04-06T11:53:50.000Z",
        status: "success",
        upstreamAccountName: "pool-epsilon@example.com",
        model: "gpt-5.4-mini",
      }),
    ]),
    createConversation("wc-current-6", [
      createPreview({
        id: 10,
        invokeId: "wc-6-a",
        occurredAt: "2026-04-06T11:56:48.000Z",
        status: "running",
        upstreamAccountName: "pool-zeta@example.com",
        tTotalMs: null,
      }),
      createPreview({
        id: 11,
        invokeId: "wc-6-b",
        occurredAt: "2026-04-06T11:52:18.000Z",
        status: "success",
        upstreamAccountName: "pool-zeta@example.com",
      }),
    ]),
    createConversation("wc-current-7", [
      createPreview({
        id: 12,
        invokeId: "wc-7-a",
        occurredAt: "2026-04-06T11:56:06.000Z",
        status: "http_429",
        failureClass: "service_failure",
        failureKind: "upstream_rate_limit",
        errorMessage: "upstream rate limit reached for the current account",
        upstreamAccountName: "pool-eta@example.com",
        requestedServiceTier: "priority",
        serviceTier: "priority",
        tUpstreamTtfbMs: null,
        tUpstreamStreamMs: null,
        tTotalMs: 1820,
      }),
      createPreview({
        id: 13,
        invokeId: "wc-7-b",
        occurredAt: "2026-04-06T11:51:12.000Z",
        status: "success",
        upstreamAccountName: "pool-eta@example.com",
      }),
    ]),
    createConversation("wc-current-8", [
      createPreview({
        id: 14,
        invokeId: "wc-8-a",
        occurredAt: "2026-04-06T11:55:20.000Z",
        status: "completed",
        upstreamAccountName: "pool-theta@example.com",
        totalTokens: 396,
        cost: 0.0228,
        inputTokens: 228,
        outputTokens: 168,
        cacheInputTokens: 66,
      }),
      createPreview({
        id: 15,
        invokeId: "wc-8-b",
        occurredAt: "2026-04-06T11:50:20.000Z",
        status: "success",
        upstreamAccountName: "pool-theta@example.com",
        model: "gpt-5.4-mini",
      }),
    ]),
  ];

  return {
    rangeStart: "2026-04-06T11:55:00.000Z",
    rangeEnd: "2026-04-06T12:00:00.000Z",
    snapshotAt: "2026-04-06T12:00:00.000Z",
    selectionMode: "activityWindow",
    selectedLimit: null,
    selectedActivityHours: null,
    selectedActivityMinutes: 5,
    implicitFilter: { kind: null, filteredCount: 0 },
    totalMatched: conversations.length,
    hasMore: false,
    nextCursor: null,
    conversations,
  };
}

function buildDashboardActivityResponse() {
  const longErrorMessage =
    '[upstream_http_429]poolupstreamrespondedwith429:error:code=429reason="DAILY_LIMIT_EXCEEDED"message="dailyusagelimitexceeded"metadata=map[]';
  const createAccount = (
    upstreamAccountId: number,
    displayName: string,
    invokeId: string,
    errorMessage?: string,
  ) => ({
    upstreamAccountId,
    displayName,
    requestCount: 12,
    successCount: errorMessage ? 3 : 12,
    failureCount: errorMessage ? 9 : 0,
    nonSuccessCount: errorMessage ? 9 : 0,
    totalTokens: 265_246,
    successTokens: 192_000,
    nonSuccessTokens: errorMessage ? 73_246 : 0,
    failureTokens: errorMessage ? 73_246 : 0,
    failureCost: errorMessage ? 0.28 : 0,
    totalCost: 0.28,
    cacheHitRate: 0.94,
    tokensPerMinute: 0,
    spendRate: 0,
    firstByteAvgMs: 220,
    firstResponseByteTotalAvgMs: 6610,
    avgTotalMs: 8300,
    inProgressInvocationCount: 0,
    inProgressPhaseCounts: { queued: 0, requesting: 0, responding: 0 },
    retryInvocationCount: 0,
    effectiveRoutingRule: {},
    recentInvocations: [
      createPreview({
        id: upstreamAccountId,
        invokeId,
        occurredAt: "2026-04-06T12:00:00.000Z",
        status: errorMessage ? "http_429" : "success",
        failureClass: errorMessage ? "service_failure" : "none",
        failureKind: errorMessage ? "upstream_rate_limit" : undefined,
        errorMessage,
        upstreamAccountId,
        upstreamAccountName: displayName,
      }),
    ],
  });

  return {
    range: "today",
    rangeStart: "2026-04-06T11:55:00.000Z",
    rangeEnd: "2026-04-06T12:00:00.000Z",
    snapshotId: 1,
    rateWindow: {
      start: "2026-04-06T11:59:00.000Z",
      end: "2026-04-06T12:00:00.000Z",
      windowMinutes: 1,
      mode: "last_complete_1m_sma",
    },
    summary: {
      stats: buildSummary("today"),
      tokensPerMinute: 0,
      spendRate: 0,
    },
    accounts: [
      createAccount(42, "CIII", "account-error-429", longErrorMessage),
      createAccount(43, "dzw", "account-success-200"),
    ],
  };
}

function buildSummary(windowName: string) {
  if (windowName === "today") {
    return {
      totalCount: 12474,
      successCount: 9949,
      failureCount: 2525,
      totalCost: 539.42,
      totalTokens: 1314275579,
    };
  }
  if (windowName === "1d") {
    return {
      totalCount: 13564,
      successCount: 10948,
      failureCount: 2616,
      totalCost: 605.33,
      totalTokens: 1456067763,
    };
  }
  return {
    totalCount: 76421,
    successCount: 70115,
    failureCount: 6306,
    totalCost: 3128.74,
    totalTokens: 8764311220,
  };
}

function buildTimeseries(range: string | null) {
  if (range === "90d") {
    return {
      rangeStart: "2026-01-06T12:00:00.000Z",
      rangeEnd: "2026-04-06T12:00:00.000Z",
      bucketSeconds: 86400,
      effectiveBucket: "1d",
      availableBuckets: ["1d"],
      bucketLimitedToDaily: false,
      points: Array.from({ length: 90 }, (_, index) => ({
        bucketStart: new Date(
          Date.parse("2026-01-06T12:00:00.000Z") + index * 86400 * 1000,
        ).toISOString(),
        bucketEnd: new Date(
          Date.parse("2026-01-07T12:00:00.000Z") + index * 86400 * 1000,
        ).toISOString(),
        totalCount: index % 9 === 0 ? 0 : 18 + (index % 7),
        successCount: index % 9 === 0 ? 0 : 16 + (index % 5),
        failureCount: index % 9 === 0 ? 0 : 2,
        totalTokens: 48000 + index * 320,
        totalCost: Number((6.4 + index * 0.03).toFixed(4)),
      })),
    };
  }
  if (range === "7d") {
    return {
      rangeStart: "2026-03-30T12:00:00.000Z",
      rangeEnd: "2026-04-06T12:00:00.000Z",
      bucketSeconds: 3600,
      effectiveBucket: "1h",
      availableBuckets: ["1h"],
      bucketLimitedToDaily: false,
      points: Array.from({ length: 7 * 24 }, (_, index) => ({
        bucketStart: new Date(
          Date.parse("2026-03-30T12:00:00.000Z") + index * 3600 * 1000,
        ).toISOString(),
        bucketEnd: new Date(
          Date.parse("2026-03-30T13:00:00.000Z") + index * 3600 * 1000,
        ).toISOString(),
        totalCount: 12 + (index % 8),
        successCount: 10 + (index % 7),
        failureCount: 2,
        totalTokens: 32000 + index * 120,
        totalCost: Number((3.2 + index * 0.02).toFixed(4)),
      })),
    };
  }
  return {
    rangeStart: "2026-04-05T12:00:00.000Z",
    rangeEnd: "2026-04-06T12:00:00.000Z",
    bucketSeconds: 60,
    effectiveBucket: "1m",
    availableBuckets: ["1m"],
    bucketLimitedToDaily: false,
    points: Array.from({ length: 24 * 60 }, (_, index) => ({
      bucketStart: new Date(
        Date.parse("2026-04-05T12:00:00.000Z") + index * 60 * 1000,
      ).toISOString(),
      bucketEnd: new Date(Date.parse("2026-04-05T12:01:00.000Z") + index * 60 * 1000).toISOString(),
      totalCount: index % 19 === 0 ? 0 : 3 + (index % 5),
      successCount: index % 19 === 0 ? 0 : 2 + (index % 4),
      failureCount: index % 19 === 0 ? 0 : 1,
      totalTokens: 2000 + index * 9,
      totalCost: Number((0.18 + index * 0.001).toFixed(4)),
    })),
  };
}

type LayoutExpectation = {
  viewport: { width: number; height: number };
  expectedColumns: number;
};

const VIEWPORTS: LayoutExpectation[] = [
  { viewport: { width: 1440, height: 900 }, expectedColumns: 2 },
  { viewport: { width: 1600, height: 900 }, expectedColumns: 3 },
  { viewport: { width: 1660, height: 900 }, expectedColumns: 4 },
  { viewport: { width: 1873, height: 900 }, expectedColumns: 4 },
];

async function installDashboardRoutes(page: Page) {
  await page.route("**/api/stats/summary**", async (route) => {
    const url = new URL(route.request().url());
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(buildSummary(url.searchParams.get("window") ?? "today")),
    });
  });

  await page.route("**/api/stats/timeseries**", async (route) => {
    const url = new URL(route.request().url());
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(buildTimeseries(url.searchParams.get("range"))),
    });
  });

  await page.route("**/api/stats/prompt-cache-conversations**", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(buildWorkingConversationsResponse()),
    });
  });

  await page.route("**/api/stats/dashboard-activity**", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(buildDashboardActivityResponse()),
    });
  });
}

test.describe("Dashboard working conversations responsive layout", () => {
  for (const { viewport, expectedColumns } of VIEWPORTS) {
    test(`keeps ${expectedColumns} columns at ${viewport.width}px`, async ({ page }) => {
      await installDashboardRoutes(page);
      await page.setViewportSize(viewport);
      await page.goto("/dashboard");

      await expect(page.getByTestId("dashboard-working-conversations")).toBeVisible();
      await expect(page.getByTestId("dashboard-working-conversation-card")).toHaveCount(8);

      const layout = await page.evaluate(() => {
        const cards = Array.from(
          document.querySelectorAll<HTMLElement>(
            '[data-testid="dashboard-working-conversation-card"]',
          ),
        );
        const grid = document.querySelector<HTMLElement>(
          '[data-testid="dashboard-working-conversations-grid"]',
        );
        const root = document.documentElement;
        if (!grid || cards.length === 0) {
          throw new Error("missing working conversations grid");
        }

        const tops: number[] = [];
        const columnsPerRow = new Map<number, number>();
        for (const card of cards) {
          const top = Math.round(card.getBoundingClientRect().top);
          const matchedTop = tops.find((candidate) => Math.abs(candidate - top) <= 4) ?? top;
          if (!tops.includes(matchedTop)) tops.push(matchedTop);
          columnsPerRow.set(matchedTop, (columnsPerRow.get(matchedTop) ?? 0) + 1);
        }

        const firstRowTop = tops.sort((left, right) => left - right)[0];
        const cardWidths = cards.map((card) => Math.round(card.getBoundingClientRect().width));

        return {
          rootOverflow: root.scrollWidth - root.clientWidth,
          firstRowCount: columnsPerRow.get(firstRowTop) ?? 0,
          rowCount: tops.length,
          minCardWidth: Math.min(...cardWidths),
          maxCardWidth: Math.max(...cardWidths),
        };
      });

      test.info().annotations.push({
        type: "dashboard-working-conversations-layout",
        description: JSON.stringify({ viewport, expectedColumns, layout }),
      });

      expect(layout.rootOverflow).toBeLessThanOrEqual(1);
      expect(layout.firstRowCount).toBe(expectedColumns);
      expect(layout.minCardWidth).toBeGreaterThan(300);
      expect(layout.maxCardWidth - layout.minCardWidth).toBeLessThanOrEqual(4);

      if (expectedColumns === 4) {
        expect(layout.rowCount).toBe(2);
      }
    });
  }

  test("keeps account metadata inline on wide desktop cards and shows compact endpoint badges", async ({
    page,
  }) => {
    await installDashboardRoutes(page);
    await page.setViewportSize({ width: 1660, height: 1180 });
    await page.goto("/dashboard");

    const compactBadge = page.locator(
      '[data-testid="invocation-endpoint-badge"][data-endpoint-kind="compact"]',
    );
    await expect(compactBadge.first()).toContainText(/远程压缩|Compact/);

    const compactCard = page
      .getByTestId("dashboard-working-conversation-card")
      .filter({
        has: page.locator(
          '[data-testid="invocation-endpoint-badge"][data-endpoint-kind="compact"]',
        ),
      })
      .first();
    await expect(compactCard).toBeVisible();

    const layout = await compactCard.evaluate((card) => {
      const slot = card.querySelector<HTMLElement>(
        '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
      );
      const accountLine = slot?.querySelector<HTMLElement>(
        '[data-testid="dashboard-working-conversation-account-line"]',
      );
      const accountChip = slot?.querySelector<HTMLElement>(
        '[data-testid="dashboard-working-conversation-account-chip"]',
      );
      const accountMeta = slot?.querySelector<HTMLElement>(
        '[data-testid="dashboard-working-conversation-account-meta"]',
      );
      if (!slot || !accountLine || !accountChip || !accountMeta) {
        throw new Error("missing account line geometry anchors");
      }

      const slotRect = slot.getBoundingClientRect();
      const lineRect = accountLine.getBoundingClientRect();
      const chipRect = accountChip.getBoundingClientRect();
      const metaRect = accountMeta.getBoundingClientRect();

      return {
        chipMetaTopDelta: Math.abs(chipRect.top - metaRect.top),
        lineOverflowRight: lineRect.right - slotRect.right,
        lineHeight: lineRect.height,
        chipBottomDelta: Math.abs(chipRect.bottom - metaRect.bottom),
      };
    });

    expect(layout.chipMetaTopDelta).toBeLessThanOrEqual(4);
    expect(layout.chipBottomDelta).toBeLessThanOrEqual(8);
    expect(layout.lineOverflowRight).toBeLessThanOrEqual(1);
    expect(layout.lineHeight).toBeLessThan(32);
  });

  test("keeps long recent error summaries inside their upstream account cards", async ({
    page,
  }) => {
    await installDashboardRoutes(page);
    await page.setViewportSize({ width: 1660, height: 1180 });
    await page.goto("/dashboard");

    await page.getByRole("tab", { name: "上游账号" }).click();
    await expect(page.getByTestId("dashboard-upstream-account-card")).toHaveCount(2);

    const layout = await page.evaluate(() => {
      const root = document.documentElement;
      const grid = document.querySelector<HTMLElement>(
        '[data-testid="dashboard-upstream-account-grid"]',
      );
      const cards = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid="dashboard-upstream-account-card"]'),
      );
      const recentRows = Array.from(
        document.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-upstream-account-recent-row"]',
        ),
      );
      if (!grid || cards.length !== 2 || recentRows.length === 0) {
        throw new Error("missing upstream account overflow geometry anchors");
      }

      const gridRect = grid.getBoundingClientRect();
      const cardOverflowRight = Math.max(
        ...cards.map((card) => card.getBoundingClientRect().right - gridRect.right),
      );
      const rowOverflowRight = Math.max(
        ...recentRows.map((row) => {
          const card = row.closest<HTMLElement>('[data-testid="dashboard-upstream-account-card"]');
          if (!card) throw new Error("missing recent row card parent");
          return row.getBoundingClientRect().right - card.getBoundingClientRect().right;
        }),
      );

      return {
        rootOverflow: root.scrollWidth - root.clientWidth,
        gridOverflowRight: gridRect.right - root.clientWidth,
        cardOverflowRight,
        rowOverflowRight,
      };
    });

    expect(layout.rootOverflow).toBeLessThanOrEqual(1);
    expect(layout.gridOverflowRight).toBeLessThanOrEqual(1);
    expect(layout.cardOverflowRight).toBeLessThanOrEqual(1);
    expect(layout.rowOverflowRight).toBeLessThanOrEqual(1);
  });
});
