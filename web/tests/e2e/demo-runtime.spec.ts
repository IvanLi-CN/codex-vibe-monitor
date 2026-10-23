import { expect, type Locator, type Page, test } from "@playwright/test";

type RouteCase = {
  path: string;
  expectedPath: string;
};

const allRoutes: RouteCase[] = [
  { path: "/#/", expectedPath: "/dashboard" },
  { path: "/#/dashboard", expectedPath: "/dashboard" },
  {
    path: "/#/dashboard/invocations/demo-invocation-9002",
    expectedPath: "/dashboard/invocations/demo-invocation-9002",
  },
  { path: "/#/stats", expectedPath: "/stats" },
  { path: "/#/live", expectedPath: "/live" },
  { path: "/#/records", expectedPath: "/records" },
  { path: "/#/account-pool", expectedPath: "/account-pool/transits" },
  { path: "/#/account-pool/upstream-accounts", expectedPath: "/account-pool/pool" },
  { path: "/#/account-pool/transits", expectedPath: "/account-pool/transits" },
  { path: "/#/account-pool/pool", expectedPath: "/account-pool/pool" },
  {
    path: "/#/account-pool/upstream-accounts/new?mode=apiKey",
    expectedPath: "/account-pool/transits/new",
  },
  {
    path: "/#/account-pool/upstream-accounts/new?mode=oauth",
    expectedPath: "/account-pool/pool/new",
  },
  {
    path: "/#/account-pool/maintenance-records",
    expectedPath: "/account-pool/maintenance-records",
  },
  { path: "/#/account-pool/groups", expectedPath: "/account-pool/groups" },
  { path: "/#/system", expectedPath: "/system/status" },
  { path: "/#/system/status", expectedPath: "/system/status" },
  { path: "/#/system/performance", expectedPath: "/system/performance" },
  { path: "/#/system/tasks", expectedPath: "/system/tasks" },
  {
    path: "/#/system/tasks/retention_archive",
    expectedPath: "/system/tasks/retention_archive",
  },
  { path: "/#/system/prompt-cache", expectedPath: "/system/prompt-cache" },
  { path: "/#/system/settings", expectedPath: "/system/settings" },
  { path: "/#/system/models", expectedPath: "/system/models" },
  { path: "/#/system/proxy", expectedPath: "/system/proxy" },
  { path: "/#/settings", expectedPath: "/system/settings" },
  { path: "/#/settings/legacy", expectedPath: "/system/settings" },
  { path: "/#/not-a-route", expectedPath: "/dashboard" },
];

const scenes = ["operational", "attention", "empty", "network-failure"] as const;
type DemoScene = (typeof scenes)[number];
type DemoFetchResult = {
  outcome: "failed" | "resolved";
  method: string;
  path: string;
};
const DEMO_ROUTE_TIMEOUT_MS = 45_000;
const nonOperationalScenes = scenes.filter(
  (value) => value !== "operational" && value !== "network-failure",
);

function routeLandmark(page: Page, path: string, allowExpectedFailure = false): Locator {
  switch (path) {
    case "/#/":
    case "/#/dashboard":
    case "/#/not-a-route":
      return page.getByTestId("dashboard-main-content");
    case "/#/dashboard/invocations/demo-invocation-9002":
      return page.getByTestId("dashboard-invocation-detail-drawer");
    case "/#/stats":
      return page.getByRole("heading", { name: "统计", exact: true });
    case "/#/live":
      return page.getByTestId("live-view-tabs");
    case "/#/records":
      return page.getByTestId("records-filters-panel");
    case "/#/account-pool":
    case "/#/account-pool/transits":
      return page.getByRole("heading", { name: "中转", exact: true });
    case "/#/account-pool/upstream-accounts":
    case "/#/account-pool/pool":
      return page.getByRole("heading", { name: "号池", exact: true });
    case "/#/account-pool/upstream-accounts/new?mode=apiKey":
    case "/#/account-pool/upstream-accounts/new?mode=oauth":
      return page.getByRole("heading", { name: "新增账号", exact: true });
    case "/#/account-pool/maintenance-records":
      return page.getByRole("heading", { name: "非模型调用执行记录", exact: true });
    case "/#/account-pool/groups": {
      const success = page
        .getByTestId("account-pool-groups-list")
        .or(page.getByTestId("account-pool-groups-empty"))
        .first();
      return allowExpectedFailure
        ? success.or(page.getByTestId("account-pool-groups-error")).first()
        : success;
    }
    case "/#/system":
    case "/#/system/status": {
      const success = page.getByTestId("system-status-overview");
      return allowExpectedFailure
        ? success.or(page.getByRole("heading", { name: "系统状态", exact: true })).first()
        : success;
    }
    case "/#/system/performance":
      return page.getByTestId("system-observability-page");
    case "/#/system/tasks":
      return page.getByTestId("system-tasks-list");
    case "/#/system/tasks/retention_archive": {
      const success = page.getByRole("heading", { name: "数据保留与归档", exact: true });
      return allowExpectedFailure
        ? success.or(page.getByText(/^任务观测不可用：/)).first()
        : success;
    }
    case "/#/system/prompt-cache":
      return page.getByTestId("system-prompt-cache");
    case "/#/system/settings":
    case "/#/settings":
    case "/#/settings/legacy":
      return page.getByRole("heading", { name: "系统设置", exact: true });
    case "/#/system/models":
      return page.getByRole("heading", { name: "模型", exact: true });
    case "/#/system/proxy":
      return page.getByRole("heading", { name: "代理", exact: true });
    default:
      throw new Error(`Missing route landmark for ${path}`);
  }
}

async function expectSceneTerminalState(page: Page, path: string, scene: DemoScene) {
  switch (path) {
    case "/#/stats": {
      const summaryPanel = page
        .getByRole("heading", { name: "统计", exact: true })
        .locator("xpath=../../../..");
      await expect(summaryPanel.locator(".metric-value").first()).toBeVisible();
      await expect(summaryPanel.getByText("…", { exact: true })).toHaveCount(0);
      if (scene === "empty") {
        await expect(summaryPanel.locator(".metric-value").first()).toHaveText(/^0+$/);
      } else {
        await expect(summaryPanel.locator(".metric-value").first()).not.toHaveText(/^0+$/);
      }
      return;
    }
    case "/#/live": {
      const routingPanel = page.getByTestId("model-routing-live-panel");
      if (scene === "empty") {
        await expect(
          routingPanel.getByText("没有符合筛选条件的 API Key 模型路由状态。", { exact: true }),
        ).toBeVisible();
        await expect(routingPanel.getByTestId("model-routing-gantt")).toHaveCount(0);
      } else {
        await expect(routingPanel.getByTestId("model-routing-gantt")).toBeVisible();
      }
      return;
    }
    case "/#/records": {
      await expect(page.getByTestId("invocation-records-table-loading")).toHaveCount(0);
      await expect(page.getByTestId("invocation-records-table-error")).toHaveCount(0);
      if (scene === "empty") {
        await expect(page.getByTestId("invocation-records-table-empty")).toBeVisible();
      } else {
        await expect(
          page.locator('[data-testid="invocation-records-model"]:visible').first(),
        ).toBeVisible();
      }
      return;
    }
    case "/#/account-pool/groups": {
      if (scene === "empty") {
        await expect(page.getByTestId("account-pool-groups-empty")).toBeVisible();
      } else {
        await expect(page.getByTestId("account-pool-groups-list")).toBeVisible();
      }
      return;
    }
    default:
      return;
  }
}

function networkFailureState(page: Page, path: string): Locator {
  switch (path) {
    case "/#/":
    case "/#/dashboard":
    case "/#/not-a-route":
      return page.getByText("实时更新不可用", { exact: true });
    case "/#/account-pool/upstream-accounts/new?mode=apiKey":
    case "/#/account-pool/upstream-accounts/new?mode=oauth":
      return page
        .getByRole("heading", { name: "新增账号", exact: true })
        .locator("xpath=../../..")
        .getByRole("status")
        .filter({ hasText: "Request failed: 0" });
    case "/#/dashboard/invocations/demo-invocation-9002":
      return page.getByTestId("dashboard-invocation-detail-error");
    case "/#/stats":
      return page.getByTestId("stats-error-distribution").getByText("Request failed: 0", {
        exact: true,
      });
    case "/#/live":
      return page
        .getByTestId("model-routing-live-panel")
        .getByRole("status")
        .filter({ hasText: "Request failed: 0" });
    case "/#/records":
      return page.getByTestId("invocation-records-table-error");
    case "/#/account-pool":
    case "/#/account-pool/upstream-accounts":
    case "/#/account-pool/transits":
    case "/#/account-pool/pool":
      return page
        .getByTestId("upstream-accounts-table-error")
        .or(page.getByTestId("upstream-accounts-grouped-error"))
        .first();
    case "/#/account-pool/groups":
      return page.getByTestId("account-pool-groups-error");
    case "/#/account-pool/maintenance-records":
      return page.getByTestId("maintenance-records-error");
    case "/#/system/tasks":
      return page.getByText(/^任务目录不可用：/);
    case "/#/system/tasks/retention_archive":
      return page.getByText(/^任务观测不可用：/);
    case "/#/system":
    case "/#/system/status":
      return page.getByText(/^加载系统状态失败：/);
    case "/#/system/performance":
      return page.getByRole("alert").filter({ hasText: "暂时无法加载观测状态。" });
    case "/#/system/prompt-cache":
      return page.getByTestId("system-prompt-cache-error");
    case "/#/system/models":
      return page
        .getByRole("heading", { name: "模型", exact: true })
        .locator("..")
        .getByRole("alert")
        .filter({ hasText: "Request failed: 0" });
    case "/#/system/proxy":
    case "/#/system/settings":
    case "/#/settings":
    case "/#/settings/legacy":
      return page.getByText(/^设置请求失败：/);
    default:
      throw new Error(`Missing network failure assertion for ${path}`);
  }
}

function networkFailureApiPath(path: string): string {
  switch (path) {
    case "/#/":
    case "/#/dashboard":
    case "/#/not-a-route":
      return "/events";
    case "/#/stats":
      return "/api/stats/errors";
    case "/#/live":
      return "/api/pool/model-routing-live";
    case "/#/dashboard/invocations/demo-invocation-9002":
      return "/api/invocations";
    case "/#/records":
      return "/api/invocations";
    case "/#/account-pool":
    case "/#/account-pool/upstream-accounts":
    case "/#/account-pool/transits":
    case "/#/account-pool/pool":
    case "/#/account-pool/upstream-accounts/new?mode=apiKey":
    case "/#/account-pool/upstream-accounts/new?mode=oauth":
    case "/#/account-pool/groups":
      return "/api/pool/upstream-accounts";
    case "/#/account-pool/maintenance-records":
      return "/api/pool/upstream-account-events";
    case "/#/system":
    case "/#/system/status":
      return "/api/system/status";
    case "/#/system/performance":
      return "/api/system/observability";
    case "/#/system/tasks":
      return "/api/system/managed-tasks";
    case "/#/system/tasks/retention_archive":
      return "/api/system/managed-tasks/retention_archive";
    case "/#/system/prompt-cache":
      return "/api/system/prompt-cache/materialization";
    case "/#/system/models":
    case "/#/system/proxy":
    case "/#/system/settings":
    case "/#/settings":
    case "/#/settings/legacy":
      return "/api/settings";
    default:
      throw new Error(`Missing network failure API path for ${path}`);
  }
}

function isDashboardRoute(path: string) {
  return path === "/#/" || path === "/#/dashboard" || path === "/#/not-a-route";
}

async function expectDashboardApiFailure(page: Page) {
  const result = await page.evaluate(async () => {
    try {
      const response = await fetch("/api/stats/summary");
      return { failed: !response.ok, message: `HTTP ${response.status}` };
    } catch (error) {
      return {
        failed: true,
        message: error instanceof Error ? error.message : String(error),
      };
    }
  });
  expect(result.failed, result.message || "Dashboard API request unexpectedly resolved").toBe(true);
}

function routeWithScene(path: string, scene: string) {
  const separator = path.includes("?") ? "&" : "?";
  return `${path}${separator}demoScene=${scene}&demoTheme=light`;
}

async function gotoDemoRoute(page: Page, route: RouteCase, scene: string) {
  await page.goto(routeWithScene(route.path, scene), {
    waitUntil: "domcontentloaded",
  });
  await expect
    .poll(() => new URL(page.url()).hash.split("?")[0], { timeout: DEMO_ROUTE_TIMEOUT_MS })
    .toBe(`#${route.expectedPath}`);
  const currentUrl = new URL(page.url());
  const hashQueryIndex = currentUrl.hash.indexOf("?");
  const hashPath =
    hashQueryIndex === -1 ? currentUrl.hash : currentUrl.hash.slice(0, hashQueryIndex);
  const hashSearch = hashQueryIndex === -1 ? "" : currentUrl.hash.slice(hashQueryIndex + 1);
  const search = new URLSearchParams(hashSearch);
  if (search.get("demoScene") === scene && search.get("demoTheme") === "light") return;
  search.set("demoScene", scene);
  search.set("demoTheme", "light");
  currentUrl.hash = `${hashPath}?${search.toString()}`;
  await page.goto(currentUrl.toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.reload({ waitUntil: "domcontentloaded" });
}

async function expectDemoShell(
  page: Page,
  route: RouteCase,
  options: {
    allowExpectedFailure?: boolean;
    expectedDetailState?: "success" | "empty";
    scene?: DemoScene;
  } = {},
) {
  const allowExpectedFailure = options.allowExpectedFailure === true;
  await expect(page.locator("#root")).toBeVisible();
  await expect(page.getByTestId("demo-inspector-summary")).toHaveCount(0);
  await expect(page.getByText("Demo Inspector", { exact: true })).toHaveCount(0);
  await expect(routeLandmark(page, route.path, allowExpectedFailure)).toBeVisible({
    timeout: DEMO_ROUTE_TIMEOUT_MS,
  });
  if (!allowExpectedFailure && options.scene) {
    await expectSceneTerminalState(page, route.path, options.scene);
  }
  if (route.path === "/#/system/performance" && !allowExpectedFailure) {
    await expect(page.getByRole("navigation", { name: "Grafana", exact: true })).toBeVisible({
      timeout: DEMO_ROUTE_TIMEOUT_MS,
    });
  }
  if (route.path === "/#/system/prompt-cache") {
    const promptCachePage = page.getByTestId("system-prompt-cache");
    const promptCacheState = allowExpectedFailure
      ? promptCachePage
          .getByRole("switch")
          .or(promptCachePage.getByTestId("system-prompt-cache-error"))
          .first()
      : promptCachePage.getByRole("switch");
    await expect(promptCacheState).toBeVisible({ timeout: DEMO_ROUTE_TIMEOUT_MS });
  }
  if (route.path === "/#/system/tasks" && !allowExpectedFailure) {
    const tasksPage = page.getByTestId("system-tasks-list");
    await expect(
      tasksPage.getByRole("status", { name: "正在读取任务目录", exact: true }),
    ).toHaveCount(0);
    await expect(tasksPage.locator("[data-task-catalog-row]:visible").first()).toBeVisible({
      timeout: DEMO_ROUTE_TIMEOUT_MS,
    });
  }
  if (route.path === "/#/dashboard/invocations/demo-invocation-9002") {
    const detailDrawer = page.getByTestId("dashboard-invocation-detail-drawer");
    await expect(detailDrawer.getByText("demo-invocation-9002", { exact: true })).toBeVisible();
    if (options.expectedDetailState === "success") {
      await expect(page.locator(".invocation-detail-hero-surface")).toBeVisible();
    } else if (options.expectedDetailState === "empty") {
      await expect(page.getByTestId("dashboard-invocation-detail-empty")).toBeVisible();
    }
  }
  await expect(page.getByLabel("Loading page", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "页面加载失败", exact: true })).toHaveCount(0);
  await expect.poll(() => new URL(page.url()).hash).toContain(route.expectedPath);
}

test.describe("Web Demo runtime", () => {
  test.describe.configure({ timeout: 120_000 });

  test("resolves every production route in operational scene", async ({ page }) => {
    for (const route of allRoutes) {
      const pageErrors: string[] = [];
      const onPageError = (error: Error) => pageErrors.push(error.message);
      page.on("pageerror", onPageError);
      try {
        await gotoDemoRoute(page, route, "operational");
        await expectDemoShell(page, route, {
          expectedDetailState: "success",
          scene: "operational",
        });
        expect(pageErrors).toEqual([]);
      } finally {
        page.off("pageerror", onPageError);
      }
    }
  });

  for (const scene of nonOperationalScenes) {
    test(`resolves every production route in ${scene} scene`, async ({ page }) => {
      for (const route of allRoutes) {
        const pageErrors: string[] = [];
        const onPageError = (error: Error) => pageErrors.push(error.message);
        page.on("pageerror", onPageError);
        try {
          await gotoDemoRoute(page, route, scene);
          await expectDemoShell(page, route, {
            expectedDetailState: scene === "empty" ? "empty" : "success",
            scene,
          });
          expect(pageErrors).toEqual([]);
        } finally {
          page.off("pageerror", onPageError);
        }
      }
    });
  }

  test("keeps every production route error-safe in network-failure scene", async ({ page }) => {
    await page.addInitScript(() => {
      const initialFetch = window.fetch.bind(window);
      const fetchResults: DemoFetchResult[] = [];
      let currentFetch: typeof window.fetch = initialFetch;
      const recordFailedRequest = (requestUrl: URL, method = "GET") => {
        if (requestUrl.pathname.startsWith("/api/") || requestUrl.pathname === "/events") {
          fetchResults.push({ outcome: "failed", method, path: requestUrl.pathname });
        }
      };
      let createEventSource: ((path: string) => EventSource) | undefined;
      Object.defineProperty(window, "__CVM_DEMO_FETCH_RESULTS__", {
        configurable: true,
        value: fetchResults,
      });
      Object.defineProperty(window, "__CVM_DEMO_CREATE_EVENT_SOURCE__", {
        configurable: true,
        get: () => createEventSource,
        set: (next: (path: string) => EventSource) => {
          createEventSource = (path) => {
            recordFailedRequest(new URL(path, window.location.href));
            return next(path);
          };
        },
      });
      Object.defineProperty(window, "fetch", {
        configurable: true,
        get: () => currentFetch,
        set: (nextFetch: typeof window.fetch) => {
          currentFetch = ((...args: Parameters<typeof window.fetch>) => {
            const requestInput = args[0];
            const requestUrl = new URL(
              requestInput instanceof Request ? requestInput.url : String(requestInput),
              window.location.href,
            );
            const method =
              requestInput instanceof Request ? requestInput.method : (args[1]?.method ?? "GET");
            if (!requestUrl.pathname.startsWith("/api/")) return nextFetch(...args);
            try {
              const result = nextFetch(...args);
              void result.then(
                (response) =>
                  fetchResults.push({
                    outcome: response.ok ? "resolved" : "failed",
                    method,
                    path: requestUrl.pathname,
                  }),
                () => fetchResults.push({ outcome: "failed", method, path: requestUrl.pathname }),
              );
              return result;
            } catch (error) {
              fetchResults.push({ outcome: "failed", method, path: requestUrl.pathname });
              throw error;
            }
          }) as typeof window.fetch;
        },
      });
    });

    for (const route of allRoutes) {
      const pageErrors: string[] = [];
      const onPageError = (error: Error) => pageErrors.push(error.message);
      page.on("pageerror", onPageError);
      try {
        await page.evaluate(() => {
          const results = (window as Window & { __CVM_DEMO_FETCH_RESULTS__?: DemoFetchResult[] })
            .__CVM_DEMO_FETCH_RESULTS__;
          if (results) results.length = 0;
        });
        await gotoDemoRoute(page, route, "network-failure");
        await page.reload({ waitUntil: "domcontentloaded" });
        await expectDemoShell(page, route, { allowExpectedFailure: true });
        await expect(networkFailureState(page, route.path)).toBeVisible({
          timeout: DEMO_ROUTE_TIMEOUT_MS,
        });
        const expectedApiPath = networkFailureApiPath(route.path);
        await expect
          .poll(
            () =>
              page.evaluate((expectedPath) => {
                const results =
                  (
                    window as Window & {
                      __CVM_DEMO_FETCH_RESULTS__?: DemoFetchResult[];
                    }
                  ).__CVM_DEMO_FETCH_RESULTS__ ?? [];
                return {
                  matched: results.some(
                    (result) => result.outcome === "failed" && result.path === expectedPath,
                  ),
                  results,
                };
              }, expectedApiPath),
            {
              message: `Missing ${expectedApiPath} fetch for ${route.path}`,
              timeout: DEMO_ROUTE_TIMEOUT_MS,
            },
          )
          .toMatchObject({ matched: true });
        if (isDashboardRoute(route.path)) await expectDashboardApiFailure(page);
        expect(pageErrors).toEqual([]);
      } finally {
        page.off("pageerror", onPageError);
      }
    }
  });

  test("round-trips query-driven scene and theme state in the shareable hash", async ({ page }) => {
    await page.goto("/#/dashboard?demoScene=attention&demoTheme=dark");
    await expect(page.locator("html")).toHaveAttribute("data-color-mode", "dark");
    await expect(page).toHaveURL(/demoScene=attention/);
    const attentionSummary = await page.evaluate(async () => {
      const response = await fetch("/api/stats/summary");
      return (await response.json()) as { totalCount: number };
    });
    expect(attentionSummary.totalCount).toBeGreaterThan(0);

    const lightPage = await page.context().newPage();
    try {
      await lightPage.goto("/#/dashboard?demoScene=empty&demoTheme=light");
      await expect(lightPage).toHaveURL(/demoScene=empty/);
      await expect(lightPage.locator("html")).toHaveAttribute("data-color-mode", "light");
      await expect(lightPage).toHaveURL(/demoTheme=light/);
      const emptySummary = await lightPage.evaluate(async () => {
        const response = await fetch("/api/stats/summary");
        return (await response.json()) as { totalCount: number };
      });
      expect(emptySummary.totalCount).toBe(0);
    } finally {
      await lightPage.close();
    }
  });

  test("keeps the live route surface free of debug controls", async ({ page }) => {
    await page.goto("/#/live?demoScene=operational&demoTheme=light");
    await expect(page.getByRole("heading", { name: "模型路由" })).toBeVisible();
    await expect(page.getByTestId("demo-inspector-summary")).toHaveCount(0);
    await expect(page.getByText("Demo Inspector", { exact: true })).toHaveCount(0);
  });

  test("shows bounded retention outcomes and pending prompt-cache statistics", async ({ page }) => {
    await page.goto("/#/system/tasks/retention_archive?demoScene=operational&demoTheme=dark");

    await expect(page.getByRole("heading", { name: "数据保留与归档" })).toBeVisible();
    await expect(page.getByText("默认计划 · 3600s")).toBeVisible();
    const workload = page.getByRole("region", { name: "运行趋势", exact: true });
    await expect(workload.getByRole("tab", { name: "次数", exact: true })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(workload.getByRole("figure", { name: "工作量：invocation rows" })).toBeVisible();
    await expect(workload.getByRole("button", { name: "隐藏待处理量", exact: true })).toBeVisible();
    await expect(workload.getByRole("button", { name: "隐藏本次发现", exact: true })).toBeVisible();
    await expect(workload.getByRole("button", { name: "隐藏本次处理", exact: true })).toBeVisible();
    await expect(page.getByText(/完成度：部分完成/)).toBeVisible();
    await expect(page.getByText(/Prompt 缓存统计：暂不可用（积压 3）/)).toBeVisible();
    await workload.getByRole("tab", { name: "时间", exact: true }).click();
    const backlog = workload.getByRole("tabpanel", { name: "时间", exact: true });
    await expect(backlog.getByText("待归档数量", { exact: true })).toBeVisible();
    await expect(backlog.getByText("最长逾期", { exact: true })).toBeVisible();
    await expect(backlog.locator(".recharts-surface")).toHaveCount(2);
    await expect(page.getByText("缺测留空，不补零", { exact: true })).toHaveCount(0);
    await expect(page.getByText("空积压显示 0 条、逾期未知", { exact: true })).toHaveCount(0);
    await expect(
      page.getByRole("heading", { name: "性能指标（性能库）", exact: true }),
    ).toHaveCount(0);
    await expect(page.getByTestId("task-observability-link").getByRole("link")).toHaveAttribute(
      "href",
      /\/d\/cvm-runtime\?.*var-task_key=retention_archive/,
    );
  });

  test("keeps an external key creation flow inside the local memory model", async ({ page }) => {
    await page.goto("/#/system/settings?demoScene=operational&demoTheme=light");

    await page.getByText("创建 Key", { exact: true }).click();
    const dialog = page.getByRole("dialog");
    await dialog
      .getByPlaceholder("例如：Vendor A upstream sync", { exact: true })
      .fill("Synthetic Key");
    await dialog.getByText("创建 Key", { exact: true }).click();

    await expect(dialog).toBeHidden();
    await expect(page.getByTestId("external-api-key-secret-alert")).toBeVisible();
    const createdKeys = await page.evaluate(async () => {
      const response = await fetch("/api/settings/external-api-keys");
      return (await response.json()) as { items: Array<{ name: string }> };
    });
    expect(createdKeys.items.some((item) => item.name === "Synthetic integration 7")).toBe(true);
  });
});
