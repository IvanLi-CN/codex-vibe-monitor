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
    await page.goto("/#/system/tasks?demoTheme=light");
    for (const key of [
      "retention_archive",
      "pool_upstream_attempt_archive",
      "forward_proxy_attempt_archive",
      "invocation_detail_prune",
      "quota_snapshot_compaction",
      "model_route_expiry",
      "task_run_history_cleanup",
      "invocation_identity_cleanup",
      "raw_orphan_sweep",
      "prompt_cache_materialization",
    ]) {
      await expect(page.locator(`a[href="#/system/tasks/${key}"]`).first()).toBeVisible();
    }
    await page.goto("/#/system/tasks/retention_archive?demoTheme=light");
    await expect(page.getByText(/旧版组合范围/)).toBeVisible();
  });
}

const retentionOwnerTaskCases = [
  {
    key: "retention_archive",
    title: "数据保留与归档",
    scope: "expired_invocations:retention_policy",
    unit: "invocation rows",
    backlog: "4,920",
  },
  {
    key: "pool_upstream_attempt_archive",
    title: "连接池上游尝试归档",
    scope: "pool_upstream_request_attempts:retention_policy",
    unit: "pool attempt rows",
    backlog: "12,480",
  },
  {
    key: "forward_proxy_attempt_archive",
    title: "正向代理尝试归档",
    scope: "forward_proxy_attempts:retention_policy",
    unit: "forward proxy attempt rows",
    backlog: "1,180",
  },
  {
    key: "invocation_detail_prune",
    title: "调用详情裁剪",
    scope: "codex_invocation_details:retention_policy",
    unit: "invocation rows",
    backlog: "3,240",
  },
  {
    key: "quota_snapshot_compaction",
    title: "配额快照压缩",
    scope: "codex_quota_snapshots:retention_policy",
    unit: "quota snapshot rows",
    backlog: "860",
  },
  {
    key: "model_route_expiry",
    title: "模型路由过期",
    scope: "model_routes:expiry",
    unit: "model route rows",
    backlog: "74",
  },
  {
    key: "task_run_history_cleanup",
    title: "任务运行历史清理",
    scope: "task_runs:history_retention",
    unit: "task-run rows",
    backlog: "2,400",
  },
] as const;

for (const width of [1280, 393]) {
  test(`renders all seven retention owner detail pages at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 852 });
    for (const task of retentionOwnerTaskCases) {
      const demoVersion = task.key === "retention_archive" ? "&demoRetentionOwners=v2" : "";
      await page.goto(`/#/system/tasks/${task.key}?demoTheme=light${demoVersion}`);
      await expect(page.getByRole("heading", { name: task.title, exact: true })).toBeVisible();
      await expect(page.getByRole("heading", { name: "最近运行", exact: true })).toBeVisible();

      const facts = page.getByRole("region", {
        name: "保留 owner 运行事实",
        exact: true,
      });
      await expect(facts).toBeVisible();
      await expect(facts).toContainText("实际运行 · 结果：本轮完成");
      await expect(facts).toContainText(task.scope);
      await expect(facts).toContainText(task.unit);
      await expect(facts).toContainText(task.backlog);
      await expect(facts).toContainText("覆盖：");
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        width,
      );
    }
  });
}
