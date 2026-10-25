import { defineConfig } from "@playwright/test";

const fixtureBaseURL = process.env.E2E_BASE_URL ?? "http://127.0.0.1:60080";
const demoBaseURL = process.env.E2E_DEMO_BASE_URL ?? "http://127.0.0.1:60083";
const workers = process.env.CI_PROFILE === "main" ? 2 : 1;

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 60_000,
  expect: { timeout: 5_000 },
  retries: process.env.CI ? 2 : 0,
  reporter: [
    ["list"],
    process.env.CI
      ? ["html", { outputFolder: process.env.PLAYWRIGHT_HTML_OUTPUT_DIR ?? "playwright-report" }]
      : ["html", { open: "never" }],
  ],
  workers,
  use: {
    trace: "on-first-retry",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
    viewport: { width: 1280, height: 900 },
  },
  webServer: undefined,
  projects: [
    {
      name: "fixtures-functional",
      workers: 1,
      testMatch: /(?:records-filter-overlay|dashboard-working-conversations-layout)\.spec\.ts/,
      use: { browserName: "chromium", baseURL: fixtureBaseURL },
    },
    {
      name: "demo-functional",
      workers: 1,
      testMatch: /(?:demo-runtime|retention-maintenance-ownership)\.spec\.ts/,
      use: { browserName: "chromium", baseURL: demoBaseURL },
    },
    {
      name: "performance",
      workers: 1,
      testMatch: /dashboard-render-performance\.spec\.ts/,
      use: { browserName: "chromium", baseURL: demoBaseURL },
    },
    {
      name: "timeline-capacity",
      workers: 1,
      testMatch: /dashboard-timeline-capacity\.spec\.ts/,
      use: { browserName: "chromium", baseURL: demoBaseURL },
    },
  ],
});
