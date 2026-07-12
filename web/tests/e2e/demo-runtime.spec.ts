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
  await expect(page.getByTestId("demo-inspector-summary")).toBeVisible();
  expect(new URL(page.url()).hash).toContain(expectedPath);
}

async function openInspector(page: Page) {
  const inspector = page.getByTestId("demo-inspector-controls");
  if (!(await inspector.isVisible())) {
    await page.getByTestId("demo-inspector-summary").click();
  }
  await expect(inspector).toBeVisible();
  return inspector;
}

test.describe("Web Demo runtime", () => {
  for (const scene of scenes) {
    test(`resolves every production route in ${scene}`, async ({ page }) => {
      for (const route of allRoutes) {
        await page.goto(routeWithScene(route.path, scene), { waitUntil: "domcontentloaded" });
        await expectDemoShell(page, route.expectedPath);
      }
    });
  }

  test("round-trips Inspector scene and theme state in the shareable hash", async ({ page }) => {
    await page.goto("/#/dashboard?demoScene=attention&demoTheme=dark");
    const inspector = await openInspector(page);

    await expect(inspector.getByRole("button", { name: "告警" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.locator("html")).toHaveAttribute("data-color-mode", "dark");

    await inspector.getByRole("button", { name: "空态" }).click();
    await expect(page).toHaveURL(/demoScene=empty/);

    await inspector.getByRole("button", { name: "浅色" }).click();
    await expect(page).toHaveURL(/demoTheme=light/);
  });

  test("injects a mock realtime event and keeps it in the Inspector action log", async ({
    page,
  }) => {
    await page.goto("/#/records?demoScene=operational&demoTheme=light");
    const inspector = await openInspector(page);

    await inspector.getByRole("button", { name: "注入模拟实时事件" }).click();
    await expect(inspector.getByText("注入模拟实时事件")).toBeVisible();
  });

  test("keeps an external key creation flow inside the demo memory model", async ({ page }) => {
    await page.goto("/#/system/settings?demoScene=operational&demoTheme=light");
    const inspector = await openInspector(page);

    await page.getByText("创建 Key", { exact: true }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByPlaceholder("例如：Vendor A upstream sync", { exact: true }).fill("Demo Key");
    await dialog.getByText("创建 Key", { exact: true }).click();

    await expect(inspector.getByText("模拟创建外部 API Key")).toBeVisible();
    await expect(page.getByText("Demo Key", { exact: true })).toHaveCount(0);
  });
});
