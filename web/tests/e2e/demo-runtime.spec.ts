import { expect, type Page, test } from "@playwright/test";

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
  { path: "/#/account-pool", expectedPath: "/account-pool/upstream-accounts" },
  { path: "/#/account-pool/upstream-accounts", expectedPath: "/account-pool/upstream-accounts" },
  {
    path: "/#/account-pool/upstream-accounts/new?mode=apiKey",
    expectedPath: "/account-pool/upstream-accounts/new",
  },
  {
    path: "/#/account-pool/maintenance-records",
    expectedPath: "/account-pool/maintenance-records",
  },
  { path: "/#/account-pool/groups", expectedPath: "/account-pool/groups" },
  { path: "/#/system", expectedPath: "/system/status" },
  { path: "/#/system/status", expectedPath: "/system/status" },
  { path: "/#/system/tasks", expectedPath: "/system/tasks" },
  { path: "/#/system/settings", expectedPath: "/system/settings" },
  { path: "/#/system/proxy", expectedPath: "/system/proxy" },
  { path: "/#/settings", expectedPath: "/system/settings" },
  { path: "/#/settings/legacy", expectedPath: "/settings/legacy" },
  { path: "/#/not-a-route", expectedPath: "/dashboard" },
];

const scenes = ["operational", "attention", "empty", "network-failure"] as const;

function routeWithScene(path: string, scene: string) {
  const separator = path.includes("?") ? "&" : "?";
  return `${path}${separator}demoScene=${scene}&demoTheme=light`;
}

async function expectDemoShell(page: Page, expectedPath: string) {
  await expect(page.locator("#root")).toBeVisible();
  await expect(page.getByTestId("demo-inspector-summary")).toHaveCount(0);
  await expect(page.getByText("Demo Inspector", { exact: true })).toHaveCount(0);
  await expect.poll(() => new URL(page.url()).hash).toContain(expectedPath);
}

test.describe("Web Demo runtime", () => {
  for (const scene of scenes) {
    test(`resolves every production route in ${scene}`, async ({ page }) => {
      for (const route of allRoutes) {
        const routePage = route === allRoutes[0] ? page : await page.context().newPage();
        try {
          await routePage.goto(routeWithScene(route.path, scene), {
            waitUntil: "domcontentloaded",
          });
          await expectDemoShell(routePage, route.expectedPath);
        } finally {
          if (routePage !== page) {
            await routePage.close();
          }
        }
      }
    });
  }

  test("round-trips query-driven scene and theme state in the shareable hash", async ({ page }) => {
    await page.goto("/#/dashboard?demoScene=attention&demoTheme=dark");
    await expect(page.locator("html")).toHaveAttribute("data-color-mode", "dark");
    await expect(page).toHaveURL(/demoScene=attention/);

    const lightPage = await page.context().newPage();
    try {
      await lightPage.goto("/#/dashboard?demoScene=empty&demoTheme=light");
      await expect(lightPage).toHaveURL(/demoScene=empty/);
      await expect(lightPage.locator("html")).toHaveAttribute("data-color-mode", "light");
      await expect(lightPage).toHaveURL(/demoTheme=light/);
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

  test("keeps an external key creation flow inside the local memory model", async ({ page }) => {
    await page.goto("/#/system/settings?demoScene=operational&demoTheme=light");

    await page.getByText("创建 Key", { exact: true }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByPlaceholder("例如：Vendor A upstream sync", { exact: true }).fill("Demo Key");
    await dialog.getByText("创建 Key", { exact: true }).click();

    await expect(dialog).toBeHidden();
    await expect(page.getByText("Demo Key", { exact: true })).toHaveCount(0);
  });
});
