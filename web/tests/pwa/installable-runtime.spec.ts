import { expect, test } from "@playwright/test";
import {
  installDashboardOverviewRoutes,
  manifestIconUrls,
  maybeCaptureScreenshot,
  type TestManifest,
} from "./installable-runtime-routes";

test.beforeEach(async ({ context, page, request }) => {
  await context.setOffline(false);
  await request.get("/__test/reset");
  await installDashboardOverviewRoutes(page);
  await page.goto("/#/dashboard");
  await expect(page.getByTestId("app-main")).toBeVisible();
});

test("serves revalidated PWA metadata and immutable content-hashed install icons", async ({
  page,
  request,
}) => {
  const metadataCacheControl = "no-cache, max-age=0, must-revalidate";
  const iconCacheControl = "public, max-age=31536000, immutable";

  const htmlResponse = await request.get("/");
  expect(htmlResponse.headers()["cache-control"]).toBe(metadataCacheControl);
  expect(page.locator('head link[rel~="manifest"]')).toHaveCount(1);
  await expect(page.locator('head link[rel~="manifest"]')).toHaveAttribute(
    "href",
    /site\.webmanifest$/,
  );
  expect(page.locator('head link[rel~="icon"]')).toHaveCount(1);
  await expect(page.locator('head link[rel~="icon"]')).toHaveAttribute(
    "href",
    /favicon-[0-9a-f]{12}\.svg$/,
  );
  expect(page.locator('head link[rel~="apple-touch-icon"]')).toHaveCount(0);

  const manifestResponse = await request.get("/site.webmanifest");
  expect(manifestResponse.headers()["cache-control"]).toBe(metadataCacheControl);
  const manifest = await manifestResponse.json();
  expect(manifest.id).toBe("./");
  expect(manifest.scope).toBe("./");
  expect(manifest.start_url).toBe("./#/dashboard");

  for (const icon of manifest.icons) {
    expect(icon.src).not.toContain("?");
    expect(icon.src).toMatch(
      /(?:favicon|icon-192|icon-512|maskable-192|maskable-512)-[0-9a-f]{12}\.(?:png|svg)$/,
    );
    const iconResponse = await request.get(new URL(icon.src, manifestResponse.url()).pathname);
    expect(iconResponse.status()).toBe(200);
    expect(iconResponse.headers()["cache-control"]).toBe(iconCacheControl);
  }

  const serviceWorkerResponse = await request.get("/sw.js");
  expect(serviceWorkerResponse.headers()["cache-control"]).toBe(metadataCacheControl);
  const serviceWorker = await serviceWorkerResponse.text();
  expect(serviceWorker).toContain("site.webmanifest");
  expect(serviceWorker).not.toMatch(/"url":"[^"]*site\.webmanifest/);
  expect(serviceWorker).not.toMatch(/"url":"[^"]*version\.json/);
  expect(serviceWorker).not.toMatch(
    /"url":"[^"]*(?:favicon|icon-192|icon-512|maskable-192|maskable-512)-[a-f0-9]{12}\.(?:png|svg)/,
  );
  expect(serviceWorker).not.toContain("CacheFirst");

  const cachedPaths = await page.evaluate(async () => {
    const cacheNames = await caches.keys();
    const cacheEntries = await Promise.all(
      cacheNames.map(async (cacheName) => (await caches.open(cacheName)).keys()),
    );
    return cacheEntries.flat().map((request) => new URL(request.url).pathname);
  });
  expect(cachedPaths).not.toContain("/site.webmanifest");
  expect(cachedPaths).not.toContain("/version.json");
  expect(
    cachedPaths.some((path) =>
      /\/(?:favicon|icon-192|icon-512|maskable-192|maskable-512)-[0-9a-f]{12}\.(?:png|svg)$/.test(
        path,
      ),
    ),
  ).toBe(false);
});

test("updates a Chromium-installed PWA to V2 manifest and icons without reinstall", async ({
  page,
  request,
}) => {
  await page.waitForFunction(() => Boolean(navigator.serviceWorker?.controller));

  const v1Manifest = (await request
    .get("/site.webmanifest")
    .then((response) => response.json())) as TestManifest;
  const v1IconUrls = manifestIconUrls(v1Manifest);
  const registrationScope = await page.evaluate(
    async () => (await navigator.serviceWorker.ready).scope,
  );

  await request.get("/__test/switch?v=2");
  await page.evaluate(async () => {
    const registration = await navigator.serviceWorker.ready;
    await registration.update();
  });

  await expect(page.getByTestId("update-available-banner")).toBeVisible();
  const v2Manifest = await page.evaluate(async () => {
    const response = await fetch("/site.webmanifest", { cache: "no-store" });
    return (await response.json()) as TestManifest;
  });
  const v2IconUrls = manifestIconUrls(v2Manifest);

  expect(v2Manifest.id).toBe(v1Manifest.id);
  expect(v2Manifest.scope).toBe(v1Manifest.scope);
  expect(v2Manifest.start_url).toBe(v1Manifest.start_url);
  expect(v2IconUrls).not.toEqual(v1IconUrls);
  expect(await page.evaluate(async () => (await navigator.serviceWorker.ready).scope)).toBe(
    registrationScope,
  );

  for (const iconUrl of v2IconUrls) {
    expect(iconUrl).toMatch(
      /(?:favicon|icon-192|icon-512|maskable-192|maskable-512)-[0-9a-f]{12}\.(?:png|svg)$/,
    );
    const iconResponse = await request.get(new URL(iconUrl, "http://127.0.0.1/").pathname);
    expect(iconResponse.status()).toBe(200);
    expect(iconResponse.headers()["cache-control"]).toBe("public, max-age=31536000, immutable");
  }
});

test("shows an install prompt dialog with a header entry and routes confirm through native prompt", async ({
  page,
}) => {
  await page.evaluate(() => {
    const installEvent = new Event("beforeinstallprompt") as Event & {
      prompt: () => Promise<void>;
      userChoice: Promise<{ outcome: "accepted"; platform: "web" }>;
    };
    (window as Window & { __pwaPrompted?: boolean }).__pwaPrompted = false;
    installEvent.preventDefault = () => undefined;
    installEvent.prompt = async () => {
      (window as Window & { __pwaPrompted?: boolean }).__pwaPrompted = true;
      window.dispatchEvent(new Event("appinstalled"));
    };
    installEvent.userChoice = Promise.resolve({ outcome: "accepted", platform: "web" });
    window.dispatchEvent(installEvent);
  });

  await expect(page.getByTestId("pwa-install-trigger").first()).toBeVisible();

  const installDialog = page.getByTestId("pwa-install-dialog");
  await expect(installDialog).toBeVisible();
  await expect(installDialog).toHaveAttribute("data-install-mode", "prompt");

  await page.getByTestId("pwa-install-confirm").click();

  await expect(installDialog).toHaveCount(0);
  await expect
    .poll(() => page.evaluate(() => (window as Window & { __pwaPrompted?: boolean }).__pwaPrompted))
    .toBe(true);
});

test("centers the install prompt dialog on narrow screens", async ({ page }) => {
  await page.setViewportSize({ width: 393, height: 852 });
  await page.goto("/#/dashboard");
  await expect(page.getByTestId("app-main")).toBeVisible();

  await page.evaluate(() => {
    const installEvent = new Event("beforeinstallprompt") as Event & {
      prompt: () => Promise<void>;
      userChoice: Promise<{ outcome: "accepted"; platform: "web" }>;
    };
    installEvent.preventDefault = () => undefined;
    installEvent.prompt = async () => {
      window.dispatchEvent(new Event("appinstalled"));
    };
    installEvent.userChoice = Promise.resolve({ outcome: "accepted", platform: "web" });
    window.dispatchEvent(installEvent);
  });

  await expect(page.getByTestId("pwa-install-trigger").nth(1)).toBeVisible();

  const dialog = page.getByTestId("pwa-install-dialog");
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByText(/Install Codex Vibe Monitor|安装 Codex Vibe Monitor/),
  ).toBeVisible();

  const box = await dialog.boundingBox();
  expect(box).not.toBeNull();
  if (!box) {
    return;
  }

  expect(Math.abs(box.x + box.width / 2 - 393 / 2)).toBeLessThan(4);
  expect(Math.abs(box.y + box.height / 2 - 852 / 2)).toBeLessThan(10);

  await maybeCaptureScreenshot(page, "pwa-install-prompt-mobile.png");
});

test("shows a prompt-style update banner when a newer service worker is waiting", async ({
  page,
  request,
}) => {
  await page.waitForFunction(() => Boolean(navigator.serviceWorker?.controller));

  await request.get("/__test/switch?v=2");

  await page.evaluate(async () => {
    const registration = await navigator.serviceWorker.ready;
    await registration.update();
  });

  const banner = page.getByTestId("update-available-banner");
  await expect(banner).toBeVisible();
  await expect(banner).toHaveAttribute("data-current-version", "v0.2.0");
  await expect(banner).toHaveAttribute("data-available-version", "v0.2.0-pwa.1");
  await expect(page.getByTestId("update-available-apply")).toBeVisible();
});

test("reloads the cached app shell offline and surfaces the offline-state banner", async ({
  context,
  page,
}) => {
  await page.waitForFunction(() => Boolean(navigator.serviceWorker?.controller));

  await context.setOffline(true);
  await page.reload({ waitUntil: "domcontentloaded" });

  await expect(page.getByTestId("app-main")).toBeVisible();
  await expect(page.getByTestId("pwa-offline-banner")).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => Boolean(navigator.serviceWorker?.controller)))
    .toBe(true);
});

test("reads cached dashboard overview snapshots offline across all five ranges", async ({
  context,
  page,
}) => {
  const overview = page.getByTestId("dashboard-activity-overview");

  await page.waitForFunction(() => Boolean(navigator.serviceWorker?.controller));
  await expect
    .poll(async () => await overview.getAttribute("data-snapshot-ready-ranges"))
    .toBe("today,yesterday,1d,7d,usage");

  await context.setOffline(true);
  await page.reload({ waitUntil: "domcontentloaded" });

  await expect(page.getByTestId("app-main")).toBeVisible();
  await expect(page.getByTestId("pwa-offline-banner")).toBeVisible();
  await expect(overview).toHaveAttribute("data-snapshot-mode", "cached-offline");
  await expect(page.getByTestId("dashboard-overview-snapshot-banner")).toBeVisible();
  await expect(page.getByTestId("dashboard-working-conversations-offline")).toBeVisible();
  await expect(page.getByTestId("today-stats-value-tpm")).toBeVisible();
  await maybeCaptureScreenshot(page, "pwa-dashboard-offline-cached-today.png");

  await page
    .getByRole("tab", { name: /昨日|yesterday/i })
    .evaluate((element: HTMLElement) => element.click());
  await expect(page.getByTestId("dashboard-activity-range-yesterday")).toBeVisible();

  await page
    .getByRole("tab", { name: /24 小时|24 hours/i })
    .evaluate((element: HTMLElement) => element.click());
  await expect(page.getByTestId("dashboard-activity-range-1d")).toBeVisible();

  await page
    .getByRole("tab", { name: /7 日|7 days/i })
    .evaluate((element: HTMLElement) => element.click());
  await expect(page.getByTestId("dashboard-activity-range-7d")).toBeVisible();

  await page
    .getByRole("tab", { name: /历史|history/i })
    .evaluate((element: HTMLElement) => element.click());
  await expect(page.getByTestId("usage-calendar-card")).toBeVisible();
  await maybeCaptureScreenshot(page, "pwa-dashboard-offline-cached-history.png");

  await context.setOffline(false);
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("app-main")).toBeVisible();
  await expect(overview).toHaveAttribute("data-snapshot-mode", "live");
  await expect(page.getByTestId("dashboard-overview-snapshot-banner")).toHaveCount(0);
});
