import { expect, type Locator, type Page, test } from "@playwright/test";

type RouteCase = {
  path: string;
  expectedPath: string;
};

const allRoutes: RouteCase[] = [
  { path: "/#/", expectedPath: "/dashboard" },
  { path: "/#/dashboard", expectedPath: "/dashboard" },
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
  { path: "/#/system/settings", expectedPath: "/system/settings" },
  { path: "/#/system/proxy", expectedPath: "/system/proxy" },
  { path: "/#/settings", expectedPath: "/system/settings" },
  { path: "/#/settings/legacy", expectedPath: "/system/settings" },
  { path: "/#/not-a-route", expectedPath: "/dashboard" },
];

const scenes = ["operational", "attention", "empty", "network-failure"] as const;
const nonOperationalScenes = scenes.filter(
  (value) => value !== "operational" && value !== "network-failure",
);
const networkFailurePaths = new Set([
  "/#/account-pool/maintenance-records",
  "/#/system/tasks",
  "/#/system/settings",
]);

function routeLandmark(page: Page, path: string): Locator {
  switch (path) {
    case "/#/":
    case "/#/dashboard":
    case "/#/not-a-route":
      return page.getByTestId("dashboard-main-content");
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
    case "/#/account-pool/groups":
      return page.getByTestId("account-pool-groups-list");
    case "/#/system":
    case "/#/system/status":
      return page.getByTestId("system-status-overview");
    case "/#/system/performance":
      return page.getByTestId("system-observability-page");
    case "/#/system/tasks":
      return page.getByTestId("system-tasks-list");
    case "/#/system/settings":
    case "/#/settings":
    case "/#/settings/legacy":
      return page.getByRole("heading", { name: "系统设置", exact: true });
    case "/#/system/proxy":
      return page.getByRole("heading", { name: "代理", exact: true });
    default:
      throw new Error(`Missing route landmark for ${path}`);
  }
}

function networkFailureState(page: Page, path: string): Locator {
  switch (path) {
    case "/#/account-pool/maintenance-records":
      return page.getByTestId("maintenance-records-error");
    case "/#/system/tasks":
      return page.getByText(/^任务目录不可用：/);
    case "/#/system/settings":
      return page.getByText(/^设置请求失败：/);
    default:
      throw new Error(`Missing network failure assertion for ${path}`);
  }
}

function routeWithScene(path: string, scene: string) {
  const separator = path.includes("?") ? "&" : "?";
  return `${path}${separator}demoScene=${scene}&demoTheme=light`;
}

async function expectDemoShell(
  page: Page,
  route: RouteCase,
  options: { assertLandmark?: boolean } = {},
) {
  await expect(page.locator("#root")).toBeVisible();
  await expect(page.getByTestId("demo-inspector-summary")).toHaveCount(0);
  await expect(page.getByText("Demo Inspector", { exact: true })).toHaveCount(0);
  if (options.assertLandmark !== false) {
    await expect(routeLandmark(page, route.path)).toBeVisible({ timeout: 45_000 });
  }
  await expect(page.getByLabel("Loading page", { exact: true })).toHaveCount(0);
  await expect.poll(() => new URL(page.url()).hash).toContain(route.expectedPath);
}

test.describe("Web Demo runtime", () => {
  test("resolves every production route in operational scene", { timeout: 120_000 }, async ({
    page,
  }) => {
    for (const route of allRoutes) {
      const pageErrors: string[] = [];
      const onPageError = (error: Error) => pageErrors.push(error.message);
      page.on("pageerror", onPageError);
      try {
        await page.goto(routeWithScene(route.path, "operational"), {
          waitUntil: "domcontentloaded",
        });
        await expectDemoShell(page, route);
        expect(pageErrors).toEqual([]);
      } finally {
        page.off("pageerror", onPageError);
      }
    }
  });

  for (const scene of nonOperationalScenes) {
    test(`resolves every production route in ${scene} scene`, { timeout: 120_000 }, async ({
      page,
    }) => {
      for (const route of allRoutes) {
        const pageErrors: string[] = [];
        const onPageError = (error: Error) => pageErrors.push(error.message);
        page.on("pageerror", onPageError);
        try {
          await page.goto(routeWithScene(route.path, scene), {
            waitUntil: "domcontentloaded",
          });
          await expectDemoShell(page, route);
          expect(pageErrors).toEqual([]);
        } finally {
          page.off("pageerror", onPageError);
        }
      }
    });
  }

  test("keeps every production route error-safe in network-failure scene", {
    timeout: 120_000,
  }, async ({ page }) => {
    for (const route of allRoutes) {
      const pageErrors: string[] = [];
      const onPageError = (error: Error) => pageErrors.push(error.message);
      page.on("pageerror", onPageError);
      try {
        await page.goto(routeWithScene(route.path, "network-failure"), {
          waitUntil: "domcontentloaded",
        });
        await expectDemoShell(page, route, { assertLandmark: false });
        if (networkFailurePaths.has(route.path)) {
          await expect(networkFailureState(page, route.path)).toBeVisible();
        }
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
