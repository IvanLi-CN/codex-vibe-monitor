import { defineConfig } from "@playwright/test";

const baseURL = process.env.PWA_E2E_BASE_URL ?? "http://127.0.0.1:61084";

export default defineConfig({
  testDir: "./tests/pwa",
  timeout: 60_000,
  expect: {
    timeout: 8_000,
  },
  retries: process.env.CI ? 2 : 0,
  reporter: [
    ["list"],
    process.env.CI
      ? ["html", { outputFolder: "playwright-report-pwa" }]
      : ["html", { open: "never" }],
  ],
  use: {
    baseURL,
    trace: "on-first-retry",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
    viewport: { width: 1280, height: 900 },
  },
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
  webServer: {
    command: "bun run pwa:test-server",
    url: baseURL,
    reuseExistingServer: !process.env.CI,
    stdout: "pipe",
    stderr: "pipe",
    timeout: 180_000,
  },
});
