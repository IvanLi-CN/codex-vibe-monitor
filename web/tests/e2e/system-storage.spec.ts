import { expect, test } from "@playwright/test";

test.describe("System project storage total", () => {
  test("stays visible while raw inventory is preparing", async ({ page }) => {
    await page.goto("/#/system/status?demoScene=system-raw-inventory-preparing&demoTheme=light");

    const summary = page.getByTestId("system-storage-summary");
    await expect(summary).toContainText("105 GiB");
    await expect(page.getByTestId("system-status-overview")).toContainText("未知");
  });

  test("stays visible when the business status endpoint returns 503", async ({ page }) => {
    await page.goto("/#/system/status?demoScene=system-storage-status-unavailable&demoTheme=light");

    await expect(page.getByTestId("system-storage-summary")).toContainText("105 GiB");
    await expect(page.getByText(/503/)).toBeVisible();
  });

  test("shows the last successful sample while a new scan has failed", async ({ page }) => {
    await page.goto("/#/system/status?demoScene=system-storage-error&demoTheme=light");

    const summary = page.getByTestId("system-storage-summary");
    await expect(summary).toContainText("105 GiB");
    await expect(summary).toContainText("最近成功读数已过期");
    await expect(summary).toContainText("权限不足");
  });

  test("does not turn a missing first sample into zero bytes", async ({ page }) => {
    await page.goto("/#/system/status?demoScene=system-storage-unknown&demoTheme=light");

    const summary = page.getByTestId("system-storage-summary");
    await expect(summary).toContainText("未知");
    await expect(summary).not.toContainText("0 B");
  });
});
