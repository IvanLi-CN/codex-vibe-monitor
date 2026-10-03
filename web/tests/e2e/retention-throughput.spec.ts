import { expect, test } from "@playwright/test";

test.describe("Retention task throughput", () => {
  for (const width of [1280, 393]) {
    test(`keeps dataset units and pending statistics readable at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto("/#/system/tasks/retention_archive?demoTheme=light");

      const throughput = page.getByRole("region", { name: "归档吞吐" }).first();
      await expect(throughput).toContainText("调用记录 · 2026-09");
      await expect(throughput).toContainText("上游尝试 · 2026-09");
      await expect(throughput).toContainText("1000 / 1000 条");
      await expect(throughput).toContainText("32.26 条/s");
      await expect(throughput).toContainText("92.9×");
      await expect(throughput).toContainText("超时次数：0");
      await expect(page.getByText(/Prompt 缓存统计：暂不可用/).first()).toBeVisible();
      await throughput.scrollIntoViewIfNeeded();
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
      expect(overflow).toBe(false);
      await expect(throughput).toBeVisible();
    });
  }
});
