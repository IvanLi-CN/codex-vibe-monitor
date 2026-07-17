import { expect, test } from "@playwright/test";

test.beforeEach(async ({ context, page, request }) => {
  await context.setOffline(false);
  await request.get("/__test/reset");
  await page.goto("/#/dashboard");
  await expect(page.getByTestId("app-main")).toBeVisible();
});

test("handles Chromium install prompts through the shared app-shell control", async ({ page }) => {
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

  const installControl = page.getByTestId("pwa-install-control");
  await expect(installControl).toBeVisible();
  await expect(installControl).toHaveAttribute("data-install-mode", "prompt");

  await installControl.click();

  await expect(installControl).toHaveAttribute("data-install-mode", "installed");
  await expect
    .poll(() => page.evaluate(() => (window as Window & { __pwaPrompted?: boolean }).__pwaPrompted))
    .toBe(true);
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
