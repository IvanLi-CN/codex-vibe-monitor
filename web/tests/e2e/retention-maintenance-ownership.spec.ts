import { expect, test } from "@playwright/test";

for (const width of [1280, 393]) {
  test(`keeps paused maintenance manually runnable with separate units at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 852 });
    await page.goto("/#/system/tasks/invocation_identity_cleanup?demoTheme=light");
    await expect(page.getByRole("heading", { name: "调用身份清理", exact: true })).toBeVisible();
    await page.getByRole("button", { name: "暂停自动触发", exact: true }).click();
    await expect(page.getByRole("button", { name: "恢复自动触发", exact: true })).toBeVisible();
    const run = page.getByRole("button", { name: "立即运行", exact: true });
    await expect(run).toBeEnabled();
    await expect(page.getByText(/对话身份 32 个已检查.*小时前缀 32 个已检查/)).toBeVisible();
    await run.click();
    await expect(page.getByRole("button", { name: "运行中", exact: true })).toBeDisabled();
    await expect(page.getByRole("button", { name: "恢复自动触发", exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
    await page.goto("/#/system/tasks/raw_orphan_sweep?demoTheme=light");
    await expect(
      page.getByRole("heading", { name: "Raw 孤儿文件清理", exact: true }),
    ).toBeVisible();
    await expect(page.getByText(/文件 32 个已检查.*65536 字节/)).toBeVisible();
    await expect(page.getByText(/总体剩余量未知/)).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
    await page.goto("/#/system/tasks/retention_archive?demoTheme=light");
    for (const key of [
      "invocation_identity_cleanup",
      "raw_orphan_sweep",
      "prompt_cache_materialization",
    ]) {
      await expect(page.locator(`a[href="#/system/tasks/${key}"]`)).toBeVisible();
    }
    await expect(page.getByText(/旧版组合范围/)).toBeVisible();
  });
}
