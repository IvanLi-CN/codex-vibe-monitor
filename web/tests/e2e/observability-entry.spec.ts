import { expect, test } from "@playwright/test";

test("uses provisioned Grafana dashboards and a UTC task deep link in demo", async ({ page }) => {
  await page.goto("/#/system/performance");
  const links = page.locator("main nav a[target='_blank']");
  await expect(links).toHaveCount(5);
  for (const link of await links.all()) {
    const url = new URL((await link.getAttribute("href")) ?? "");
    expect(url.protocol).toBe("https:");
    expect(url.pathname).toMatch(/^\/d\/cvm-(overview|proxy|sqlite|runtime|web)$/);
    expect(url.searchParams.get("timezone")).toBe("utc");
  }
  await page.goto("/#/system/tasks/retention_archive");
  const task = page.getByTestId("task-observability-link").getByRole("link");
  await expect(task).toBeVisible();
  const url = new URL((await task.getAttribute("href")) ?? "");
  expect(url.pathname).toBe("/d/cvm-runtime");
  expect(url.searchParams.get("var-task_key")).toBe("retention_archive");
  expect(url.searchParams.get("from")).toBe("now-30m");
});

test("shows missing configuration on a mobile viewport", async ({ page }) => {
  await page.setViewportSize({ width: 393, height: 852 });
  await page.goto("/#/system/performance?demoGrafana=missing");
  await expect(page.locator("main nav a[target='_blank']")).toHaveCount(0);
  const surface = page.getByTestId("system-observability-page");
  await expect(surface.getByText("当前实例尚未配置 Grafana。", { exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(393);
  await expect(surface.locator("svg, iframe")).toHaveCount(0);
});
